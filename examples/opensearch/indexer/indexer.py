import json
import hashlib
import os
import re
import signal
import sys
import threading
import time
from uuid import UUID

import yaml
from kafka import KafkaAdminClient, KafkaConsumer, TopicPartition
from kafka.consumer.subscription_state import AsyncConsumerRebalanceListener
from kafka.structs import OffsetAndMetadata
from opensearchpy import OpenSearch
from opensearchpy.exceptions import ConflictError, RequestError


STOP = False
ROLE_LOCK = threading.Lock()


def request_stop(_signum, _frame):
    global STOP
    STOP = True


signal.signal(signal.SIGINT, request_stop)
signal.signal(signal.SIGTERM, request_stop)


def log(message):
    print(message, flush=True)


def expand_env(value):
    if isinstance(value, str):
        pattern = re.compile(r"\$\{([^}:]+)(?::-([^}]*))?\}")

        def replace(match):
            return os.environ.get(match.group(1), match.group(2) or "")

        return pattern.sub(replace, value)
    if isinstance(value, list):
        return [expand_env(item) for item in value]
    if isinstance(value, dict):
        return {key: expand_env(item) for key, item in value.items()}
    return value


def read_config(path):
    with open(path, "r", encoding="utf-8") as handle:
        return expand_env(yaml.safe_load(handle))


def read_model_id():
    model_id = os.environ.get("OPENSEARCH_MODEL_ID", "")
    model_id_file = os.environ.get("OPENSEARCH_MODEL_ID_FILE", "")
    if model_id:
        return model_id
    if model_id_file and os.path.exists(model_id_file):
        with open(model_id_file, "r", encoding="utf-8") as handle:
            return handle.read().strip()
    return ""


def wait_for_opensearch(client):
    deadline = time.time() + 180
    while time.time() < deadline:
        try:
            if client.ping():
                return
        except Exception as exc:
            log(f"waiting for OpenSearch: {exc}")
        time.sleep(2)
    raise RuntimeError("OpenSearch did not become available")


def mapping_type(field):
    tags = set(field.get("tags", []))
    if "search" in tags or "vectorSearch" in tags:
        return {"type": "text"}

    declared = field.get("type", "keyword")
    if declared not in {"keyword", "integer", "long", "float", "double", "boolean", "date"}:
        raise ValueError(f"unsupported mapping type {declared!r}")
    return {"type": declared}


def build_index_body(namespace_config, stream=None):
    properties = {
        "_mongo_namespace": {"type": "keyword"},
        "_mongo_id": {"type": "keyword"},
        "_sync_deleted": {"type": "boolean"},
        "_sync_topic_id": {"type": "keyword"},
    }
    vector_fields = []
    vector_config = namespace_config.get("vector", {})
    dimension = int(vector_config.get("dimension", 384))
    model_id = vector_config.get("modelId") or read_model_id()

    for field_name, field_config in namespace_config.get("fields", {}).items():
        if field_name.split(".")[0].startswith(("_sync_", "_mongo_")) or field_name == "_id":
            raise ValueError(f"reserved output field {field_name}")
        properties[field_name] = mapping_type(field_config)
        if "vectorSearch" in set(field_config.get("tags", [])):
            vector_field_name = f"{field_name}_embedding"
            properties[vector_field_name] = {
                "type": "knn_vector",
                "dimension": dimension,
                "method": {
                    "name": "hnsw",
                    "space_type": "cosinesimil",
                    "engine": "lucene",
                },
            }
            vector_fields.append((field_name, vector_field_name))

    settings = {"index": {"knn": True}}
    if vector_fields and model_id:
        settings["index"]["default_pipeline"] = pipeline_name(namespace_config)
        settings["index"]["search"] = {"default_pipeline": search_pipeline_name(namespace_config)}

    body = {"settings": settings, "mappings": {"properties": properties}}
    signature = hashlib.sha256(json.dumps(
        {"config": namespace_config, "model": model_id, "body": body},
        sort_keys=True, separators=(",", ":")
    ).encode()).hexdigest()
    body["mappings"]["_meta"] = {"stream": stream, "config_sha256": signature}
    return body, vector_fields, model_id


def pipeline_name(namespace_config):
    return f"{namespace_config['index'].replace('.', '-')}-auto-embed"


def search_pipeline_name(namespace_config):
    return f"{namespace_config['index'].replace('.', '-')}-default-neural-model"


def ensure_pipeline(client, namespace_config, vector_fields, model_id):
    if not vector_fields or not model_id:
        if vector_fields:
            log(
                "OPENSEARCH_MODEL_ID is empty; created vector mappings without "
                "an ingest embedding pipeline"
            )
        return

    field_map = {source: target for source, target in vector_fields}
    body = {
        "description": "MongoDB extension PoC auto-embedding pipeline",
        "processors": [
            {
                "text_embedding": {
                    "model_id": model_id,
                    "field_map": field_map,
                }
            }
        ],
    }
    client.transport.perform_request("PUT", f"/_ingest/pipeline/{pipeline_name(namespace_config)}", body=body)


def ensure_search_pipeline(client, namespace_config, vector_fields, model_id):
    if not vector_fields or not model_id:
        return

    body = {
        "request_processors": [
            {
                "neural_query_enricher": {
                    "description": "MongoDB extension PoC default neural query model",
                    "neural_field_default_id": {
                        vector_field: model_id for _source_field, vector_field in vector_fields
                    },
                }
            }
        ]
    }
    client.transport.perform_request(
        "PUT", f"/_search/pipeline/{search_pipeline_name(namespace_config)}", body=body
    )


def ensure_index(client, namespace, namespace_config, stream=None):
    index = namespace_config["index"]
    body, vector_fields, model_id = build_index_body(namespace_config, stream)
    if not client.indices.exists(index=index):
        try:
            client.indices.create(index=index, body=body)
            log(f"created OpenSearch index {index} for {namespace}")
        except RequestError as exc:
            if exc.error != "resource_already_exists_exception":
                raise

    mapping = client.indices.get_mapping(index=index)
    existing = mapping[index]["mappings"].get("properties", {})
    if mapping[index]["mappings"].get("_meta") != body["mappings"]["_meta"]:
        raise RuntimeError(f"index {index}: stream/config changed; stop all workers and resync")
    expected = body["mappings"]["properties"]
    missing = sorted(set(expected) - set(existing))
    if missing:
        raise RuntimeError(f"index {index} is missing mapped fields: {', '.join(missing)}")
    for name, expected_field in expected.items():
        if existing[name].get("type") != expected_field["type"]:
            raise RuntimeError(f"index {index}: incompatible mapping for {name}")
        if "dimension" in expected_field and existing[name].get("dimension") != expected_field["dimension"]:
            raise RuntimeError(f"index {index}: incompatible vector dimension for {name}")
    # Only workers with the same immutable contract may update shared pipelines.
    ensure_pipeline(client, namespace_config, vector_fields, model_id)
    ensure_search_pipeline(client, namespace_config, vector_fields, model_id)
    log(f"validated OpenSearch index {index} for {namespace}")


def get_path(document, path):
    current = document
    for part in path.split("."):
        if not isinstance(current, dict) or part not in current:
            return None
        current = current[part]
    return current


def set_path(document, path, value):
    parts = path.split(".")
    current = document
    for part in parts[:-1]:
        current = current.setdefault(part, {})
    current[parts[-1]] = value


def extract_id(raw):
    if isinstance(raw, dict):
        if "$oid" in raw:
            return raw["$oid"]
        if "$uuid" in raw:
            return raw["$uuid"]
    return raw


def decode_json(raw):
    if raw is None:
        return None
    if isinstance(raw, bytes):
        raw = raw.decode("utf-8")
    if raw == "":
        return None
    return json.loads(raw)


def document_id_from_key(key):
    decoded = decode_json(key)
    if not isinstance(decoded, dict):
        return None
    if "documentKey" in decoded:
        decoded = decoded["documentKey"]
    if not isinstance(decoded, dict):
        return None
    return extract_id(decoded.get("_id"))


def preserved_mongo_id(raw):
    """JSON text of the original `_id`, stored so search can rebuild its BSON type."""
    return json.dumps(raw, separators=(",", ":"), sort_keys=True)


def project_document(namespace, namespace_config, document):
    if not isinstance(document, dict):
        raise ValueError("Kafka value must be a JSON document")
    if "_id" not in document:
        raise ValueError("Kafka document is missing _id")

    original_id = document["_id"]
    projected = {
        "_id": extract_id(original_id),
        "_mongo_namespace": namespace,
        "_mongo_id": preserved_mongo_id(original_id),
    }
    for output_name, field_config in namespace_config.get("fields", {}).items():
        value = get_path(document, field_config.get("sourcePath", output_name))
        if value is not None:
            set_path(projected, output_name, value)
    return projected


def describe_stream(admin, topic):
    metadata = admin.describe_topics([topic])
    if (len(metadata) != 1 or metadata[0].get("error_code") or
            metadata[0].get("name", metadata[0].get("topic")) != topic):
        raise RuntimeError(f"cannot describe Kafka topic {topic}")
    info = metadata[0]
    partitions = info.get("partitions", [])
    if (len(partitions) != 1 or
            partitions[0].get("partition_index", partitions[0].get("partition")) != 0 or
            partitions[0].get("error_code", 0)):
        raise RuntimeError("active-standby requires exactly one partition (partition 0)")
    topic_id = info.get("topic_id")
    if topic_id is None or UUID(str(topic_id)).int == 0:
        raise RuntimeError("Kafka broker/client must expose a nonzero topic UUID")
    return {"topic": topic, "topic_id": str(UUID(str(topic_id))), "partition": 0}


def write_record(client, namespace_config, stream, message):
    if message.topic != stream["topic"] or message.partition != 0 or message.offset < 0:
        raise RuntimeError("record belongs to an unexpected partition")
    key_id = document_id_from_key(message.key)
    deleted = message.value is None
    if deleted:
        document_id = key_id
        body = {"_mongo_namespace": message.topic}
    else:
        body = project_document(message.topic, namespace_config, decode_json(message.value))
        document_id = body.pop("_id")
        if key_id is None or str(key_id) != str(document_id):
            raise ValueError("record key must match document _id")
    if not isinstance(document_id, (str, int)) or isinstance(document_id, bool) or str(document_id) == "":
        raise ValueError("record has no supported document ID")
    body.update({"_sync_deleted": deleted, "_sync_topic_id": stream["topic_id"]})
    version = message.offset + 1
    options = {"pipeline": "_none"} if deleted else {}
    try:
        client.index(index=namespace_config["index"], id=str(document_id), body=body,
                     version=version, version_type="external", refresh=True, **options)
    except ConflictError:
        stored = client.get(index=namespace_config["index"], id=str(document_id))
        if (stored.get("_version", 0) < version or
                stored.get("_source", {}).get("_sync_topic_id") != stream["topic_id"]):
            raise RuntimeError("version conflict with an unrecognized stream")
        # A duplicate or late request cannot overwrite the newer indexed state.
    log(f"{'tombstoned' if deleted else 'indexed'} {message.topic}/{document_id} offset={message.offset}")


def process_record(consumer, client, namespace_config, stream, message):
    partition = TopicPartition(message.topic, message.partition)
    if partition not in consumer.assignment():
        raise RuntimeError("partition revoked before write")
    write_record(client, namespace_config, stream, message)
    if partition not in consumer.assignment():
        raise RuntimeError("partition revoked after write; replay required")
    consumer.commit(offsets={partition: OffsetAndMetadata(message.offset + 1, "", -1)})


def validate_checkpoint(committed, beginning, end):
    if (committed is None and beginning != 0) or (committed is not None and not beginning <= committed <= end):
        raise RuntimeError("Kafka checkpoint is outside retained history; explicit resync required")


def set_role(role):
    path = "/tmp/indexer-role"
    with ROLE_LOCK:
        with open(path + ".tmp", "w", encoding="utf-8") as handle:
            handle.write(role)
        os.replace(path + ".tmp", path)


async def read_checkpoints(consumer, partitions):
    # kafka-python 3.0.8 exposes native coroutine operations through its internal
    # fetcher/coordinator, not asyncio or public KafkaConsumer async methods.
    # Keep this version-pinned adapter here; Docker failover exercises it.
    beginning = await consumer._fetcher._fetch_offsets_by_times_async(
        {partition: -2 for partition in partitions}, timeout_ms=10000)
    end = await consumer._fetcher._fetch_offsets_by_times_async(
        {partition: -1 for partition in partitions}, timeout_ms=10000)
    committed = await consumer._coordinator.fetch_committed_offsets_async(partitions, timeout_ms=10000)
    offsets = {}
    for partition in partitions:
        checkpoint = committed[partition].offset if partition in committed else None
        validate_checkpoint(checkpoint, beginning[partition].offset, end[partition].offset)
        offsets[partition] = checkpoint if checkpoint is not None else 0
    return offsets


class AssignmentListener(AsyncConsumerRebalanceListener):
    def __init__(self, consumer):
        self.consumer = consumer

    async def on_partitions_revoked(self, partitions):
        set_role("standby")
        log(f"standby: revoked {partitions}; no automatic commit")

    async def on_partitions_assigned(self, partitions):
        if partitions:
            offsets = await read_checkpoints(self.consumer, list(partitions))
            for partition, offset in offsets.items():
                self.consumer.seek(partition, offset)
        set_role("active" if partitions else "standby")
        log(f"{'active' if partitions else 'standby'}: assigned {partitions}")


def consume(consumer, admin, client, namespace_config, stream, stopped=lambda: STOP):
    try:
        set_role("standby")
        consumer.subscribe([stream["topic"]], listener=AssignmentListener(consumer))
        while not stopped():
            batch = consumer.poll(timeout_ms=1000, max_records=1)
            if describe_stream(admin, stream["topic"]) != stream:
                raise RuntimeError("Kafka topic UUID changed; explicit resync required")
            for messages in batch.values():
                for message in messages:
                    if stopped():
                        break
                    process_record(consumer, client, namespace_config, stream, message)
    finally:
        set_role("stopped")
        consumer.close(autocommit=False)


def main():
    config = read_config(os.environ.get("INDEX_CONFIG", "/config/indexing.yml"))
    namespaces = config.get("namespaces", {})
    topics = [topic.strip() for topic in (os.environ.get("KAFKA_TOPICS") or ",".join(namespaces)).split(",") if topic.strip()]
    if len(namespaces) != 1 or len(topics) != 1 or topics[0] not in namespaces:
        raise RuntimeError("active-standby supports exactly one namespace/topic")
    topic = topics[0]
    bootstrap = os.environ.get("KAFKA_BOOTSTRAP_SERVERS", "kafka:9092").split(",")
    admin = KafkaAdminClient(bootstrap_servers=bootstrap, request_timeout_ms=10000)
    stream = describe_stream(admin, topic)

    client = OpenSearch(
        hosts=[os.environ.get("OPENSEARCH_URL", "http://opensearch:9200")],
        timeout=10,
        max_retries=2,
        retry_on_timeout=True,
    )
    wait_for_opensearch(client)

    for namespace, namespace_config in namespaces.items():
        ensure_index(client, namespace, namespace_config, stream)

    consumer = KafkaConsumer(
        bootstrap_servers=bootstrap,
        group_id=os.environ.get("KAFKA_GROUP_ID", "mongodb-opensearch-indexer"),
        auto_offset_reset="none",
        enable_auto_commit=False,
        max_poll_records=1,
        max_poll_interval_ms=120000,
        session_timeout_ms=10000,
        heartbeat_interval_ms=3000,
    )
    try:
        consume(consumer, admin, client, namespaces[topic], stream)
    finally:
        admin.close()
        client.close()
    log("indexer stopped")


if __name__ == "__main__":
    try:
        main()
    except Exception as exc:
        log(f"indexer failed: {exc}")
        sys.exit(1)
