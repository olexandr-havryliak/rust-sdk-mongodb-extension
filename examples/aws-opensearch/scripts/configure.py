import json
import os
from pathlib import Path
from urllib.parse import urlsplit

from setup_local import request


CONFIG = Path(__file__).resolve().parents[1] / "config"


def directory(topology):
    if topology not in ("replica-set", "mongos"):
        raise ValueError("Topology must be replica-set or mongos")
    return CONFIG / topology


def read(path):
    return json.loads(path.read_text())


def connector_configs(topology, endpoint, aws_region=None, role_arn=None):
    config = directory(topology)
    if topology == "replica-set":
        sink = read(config / "opensearch-sink-connector.json")
        sources = {"mongo-products-source": read(config / "mongo-source-connector.json")}
    else:
        sink = read(config / "opensearch-sink.json")
        sources = {f"mongo-{name}-source": read(config / f"mongo-{name}-source.json")
                   for name in ("products", "range", "hashed", "compound", "unsharded")}
    parsed = urlsplit(endpoint)
    if parsed.scheme not in ("http", "https") or not parsed.hostname or parsed.username or parsed.password or parsed.path not in ("", "/") or parsed.query or parsed.fragment:
        raise ValueError("Expected an OpenSearch HTTP(S) origin without credentials")
    sink["connection.url"] = endpoint.rstrip("/")
    if aws_region:
        if parsed.scheme != "https" or parsed.port not in (None, 443):
            raise ValueError("AWS Kafka sink requires HTTPS on port 443")
        # Aiven's AwsSdk2Transport requires a hostname, not an HTTP URL.
        sink["connection.url"] = parsed.hostname
        sink.update({"aws.region": aws_region, "aws.service.signing.name": "es"})
        if role_arn:
            sink.update({"aws.sts.role.arn": role_arn, "aws.sts.role.session.name": "mongodb-opensearch-demo"})
        else:
            sink.update({"aws.access_key_id": "${env:AWS_ACCESS_KEY_ID}",
                         "aws.secret_access_key": "${env:AWS_SECRET_ACCESS_KEY}"})
    return sink, sources


def pipeline_configs(topology, model_id, dimension):
    if not model_id or dimension not in (384, 512):
        raise ValueError("Expected a deployed MiniLM (384) or Titan V2 (512) model")
    config = directory(topology)
    ingest = read(config / "opensearch-ingest-pipeline.json")
    search = read(config / "opensearch-search-pipeline.json")
    template = read(config / "opensearch-index-template.json")
    ingest["processors"][1]["text_embedding"]["model_id"] = model_id
    script = ingest["processors"][2]["script"]
    script["source"] = script["source"].replace("vector.size() != 384", f"vector.size() != {dimension}")
    search["request_processors"][0]["neural_query_enricher"]["default_model_id"] = model_id
    template["template"]["mappings"]["dynamic_templates"][0]["embeddings"]["mapping"]["dimension"] = dimension
    return ingest, search, template


def main():
    state = Path("/state")
    topology = os.environ["TOPOLOGY"]
    endpoint = (state / "endpoint").read_text().strip()
    model_id = (state / "model-id").read_text().strip()
    for path, body in zip(("/_ingest/pipeline/mongodb-auto-embed", "/_search/pipeline/mongodb-default-model", "/_index_template/mongodb-vectors"), pipeline_configs(topology, model_id, 384)):
        request(endpoint, "PUT", path, body)
    write_extension_configs(state, endpoint, topology)
    print(f"Configured {topology} pipelines and extension endpoints", flush=True)


def write_extension_configs(state, endpoint, topology):
    directory(topology)
    # Each example-owned library selects a shared topology-specific stage.
    for name in ("opensearch", "opensearch_mongos"):
        (state / f"{name}.conf").write_text(
            f"sharedLibraryPath: /usr/local/lib/mongo-extensions/libaws_{name}_extension.so\n"
            f"extensionOptions:\n  endpoints: {endpoint}\n"
        )
    (state / "topology").write_text(topology)


if __name__ == "__main__":
    main()
