import json
import os
import sys
import time
import urllib.error
import urllib.request


OPENSEARCH_URL = os.environ.get("OPENSEARCH_URL", "http://opensearch:9200").rstrip("/")
MODEL_GROUP_NAME = os.environ.get("OPENSEARCH_MODEL_GROUP", "mongodb-extension-poc")
MODEL_NAME = os.environ.get(
    "OPENSEARCH_MODEL_NAME",
    "huggingface/sentence-transformers/paraphrase-MiniLM-L3-v2",
)
MODEL_VERSION = os.environ.get("OPENSEARCH_MODEL_VERSION", "1.0.2")
MODEL_FORMAT = os.environ.get("OPENSEARCH_MODEL_FORMAT", "ONNX")
MODEL_ID_FILE = os.environ.get("OPENSEARCH_MODEL_ID_FILE", "/model/model-id")


def log(message):
    print(message, flush=True)


def request(method, path, payload=None, expected=(200, 201)):
    body = None
    headers = {}
    if payload is not None:
        body = json.dumps(payload).encode("utf-8")
        headers["Content-Type"] = "application/json"
    req = urllib.request.Request(f"{OPENSEARCH_URL}{path}", data=body, headers=headers, method=method)
    try:
        with urllib.request.urlopen(req, timeout=30) as response:
            data = response.read().decode("utf-8")
            if response.status not in expected:
                raise RuntimeError(f"{method} {path} returned {response.status}: {data}")
            return json.loads(data) if data else {}
    except urllib.error.HTTPError as exc:
        data = exc.read().decode("utf-8")
        raise RuntimeError(f"{method} {path} returned {exc.code}: {data}") from exc


def wait_for_opensearch():
    deadline = time.time() + 180
    while time.time() < deadline:
        try:
            request("GET", "/_cluster/health")
            return
        except Exception as exc:
            log(f"waiting for OpenSearch: {exc}")
            time.sleep(2)
    raise RuntimeError("OpenSearch did not become available")


def wait_task(task_id, label, timeout=900):
    deadline = time.time() + timeout
    while time.time() < deadline:
        task = request("GET", f"/_plugins/_ml/tasks/{task_id}")
        state = task.get("state") or task.get("status")
        if state == "COMPLETED":
            return task
        if state in {"FAILED", "CANCELLED", "COMPLETED_WITH_ERROR"}:
            raise RuntimeError(f"{label} failed: {json.dumps(task)}")
        log(f"{label} task {task_id} is {state}; waiting")
        time.sleep(5)
    raise RuntimeError(f"{label} task {task_id} did not complete")


def configure_ml_commons():
    request(
        "PUT",
        "/_cluster/settings",
        {
            "persistent": {
                "cluster.routing.allocation.disk.threshold_enabled": "false",
                "plugins.ml_commons.only_run_on_ml_node": "false",
                "plugins.ml_commons.model_access_control_enabled": "true",
                "plugins.ml_commons.native_memory_threshold": "100",
                "plugins.ml_commons.jvm_heap_memory_threshold": "100",
                "cluster.blocks.create_index": "false",
            }
        },
    )


def register_model_group():
    search = request(
        "POST",
        "/_plugins/_ml/model_groups/_search",
        {"query": {"term": {"name.keyword": MODEL_GROUP_NAME}}},
    )
    hits = search.get("hits", {}).get("hits", [])
    if hits:
        model_group_id = hits[0]["_id"]
        log(f"using existing OpenSearch model group {model_group_id}")
        return model_group_id

    response = request(
        "POST",
        "/_plugins/_ml/model_groups/_register",
        {
            "name": MODEL_GROUP_NAME,
            "description": "Models used by the MongoDB OpenSearch search demo",
        },
        expected=(200, 201),
    )
    if response.get("model_group_id"):
        return response["model_group_id"]
    raise RuntimeError(f"model group registration returned no id: {json.dumps(response)}")


def find_existing_model():
    search = request(
        "POST",
        "/_plugins/_ml/models/_search",
        {
            "query": {
                "bool": {
                    "must": [
                        {"term": {"name.keyword": MODEL_NAME}},
                        {"term": {"version.keyword": MODEL_VERSION}},
                    ]
                }
            }
        },
    )
    hits = search.get("hits", {}).get("hits", [])
    return hits[0]["_id"] if hits else None


def register_model(model_group_id):
    existing = find_existing_model()
    if existing:
        log(f"using existing OpenSearch model {existing}")
        return existing

    response = request(
        "POST",
        "/_plugins/_ml/models/_register",
        {
            "name": MODEL_NAME,
            "version": MODEL_VERSION,
            "model_group_id": model_group_id,
            "model_format": MODEL_FORMAT,
        },
    )
    task = wait_task(response["task_id"], "model registration")
    model_id = task.get("model_id") or response.get("model_id")
    if not model_id:
        raise RuntimeError(f"registration completed without model_id: {json.dumps(task)}")
    return model_id


def deploy_model(model_id):
    response = request("POST", f"/_plugins/_ml/models/{model_id}/_deploy")
    if response.get("task_id"):
        wait_task(response["task_id"], "model deployment")


def predict_smoke_test(model_id):
    response = request(
        "POST",
        f"/_plugins/_ml/_predict/text_embedding/{model_id}",
        {
            "text_docs": ["waterproof hiking backpack"],
            "return_number": True,
            "target_response": ["sentence_embedding"],
        },
    )
    try:
        vector = response["inference_results"][0]["output"][0]["data"]
    except (KeyError, IndexError, TypeError) as exc:
        raise RuntimeError(f"unexpected predict response: {json.dumps(response)}") from exc
    if len(vector) != 384:
        raise RuntimeError(f"expected 384 embedding dimensions, got {len(vector)}")


def main():
    wait_for_opensearch()
    configure_ml_commons()
    model_group_id = register_model_group()
    model_id = register_model(model_group_id)
    deploy_model(model_id)
    predict_smoke_test(model_id)
    os.makedirs(os.path.dirname(MODEL_ID_FILE), exist_ok=True)
    with open(MODEL_ID_FILE, "w", encoding="utf-8") as handle:
        handle.write(model_id)
    log(f"registered and deployed OpenSearch model {model_id}")


if __name__ == "__main__":
    try:
        main()
    except Exception as exc:
        log(f"OpenSearch model setup failed: {exc}")
        sys.exit(1)
