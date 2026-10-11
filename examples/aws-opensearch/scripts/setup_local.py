import json
import os
import time
import urllib.request
from pathlib import Path


def domain_config():
    return {
        "DomainName": "mongodb-vector-demo",
        "EngineVersion": "OpenSearch_2.19",
        "ClusterConfig": {"InstanceType": "t3.small.search", "InstanceCount": 1},
        "EBSOptions": {"EBSEnabled": True, "VolumeType": "gp3", "VolumeSize": 10},
    }


def local_connector():
    return {
        "name": "local-minilm-embeddings",
        "description": "Local remote inference, not an emulation of Amazon Titan",
        "version": 1,
        "protocol": "http",
        "parameters": {},
        # ML Commons requires a nonempty map even for a public HTTP endpoint.
        # This entry is not referenced by headers or used for authentication.
        "credential": {"local_placeholder": "unused"},
        "actions": [{
            "action_type": "predict",
            "method": "POST",
            "url": "http://embeddings:8080/embed",
            "headers": {"content-type": "application/json"},
            "request_body": "${parameters.input}",
            "pre_process_function": "connector.pre_process.default.embedding",
            "post_process_function": "connector.post_process.default.embedding",
        }],
    }


def local_settings():
    return {"persistent": {
        "plugins.ml_commons.only_run_on_ml_node": False,
        "plugins.ml_commons.connector_access_control_enabled": False,
        "plugins.ml_commons.connector.private_ip_enabled": True,
        "plugins.ml_commons.trusted_connector_endpoints_regex": ["^http://embeddings:8080/embed$"],
    }}


def request(endpoint, method, path, body=None):
    data = None if body is None else json.dumps(body).encode()
    req = urllib.request.Request(
        endpoint.rstrip("/") + path, data=data, method=method,
        headers={"Content-Type": "application/json"},
    )
    with urllib.request.urlopen(req, timeout=30) as response:
        return json.load(response)


def wait_model(endpoint, model_id):
    deadline = time.monotonic() + 180
    while time.monotonic() < deadline:
        model = request(endpoint, "GET", f"/_plugins/_ml/models/{model_id}")
        if model.get("model_state") == "DEPLOYED":
            return
        if model.get("model_state") == "DEPLOY_FAILED":
            raise RuntimeError("Remote model deployment failed")
        time.sleep(2)
    raise TimeoutError("Remote model did not deploy")


def ensure_model(endpoint):
    existing = request(endpoint, "POST", "/_plugins/_ml/models/_search", {
        "query": {"term": {"name.keyword": "local-minilm"}},
    })["hits"]["hits"]
    if existing:
        model_id = existing[0]["_id"]
        if existing[0]["_source"].get("model_state") == "DEPLOYED":
            return model_id
    else:
        connector_id = request(endpoint, "POST", "/_plugins/_ml/connectors/_create", local_connector())["connector_id"]
        model_id = request(endpoint, "POST", "/_plugins/_ml/models/_register", {
            "name": "local-minilm", "function_name": "remote", "connector_id": connector_id,
        })["model_id"]
    request(endpoint, "POST", f"/_plugins/_ml/models/{model_id}/_deploy")
    wait_model(endpoint, model_id)
    return model_id


def main():
    import boto3

    client = boto3.client("opensearch", endpoint_url="http://floci:4566")
    config = domain_config()
    name = config["DomainName"]
    existing = client.list_domain_names()["DomainNames"]
    if not any(item["DomainName"] == name for item in existing):
        client.create_domain(**config)
    deadline = time.monotonic() + 300
    while time.monotonic() < deadline:
        domain = client.describe_domain(DomainName=name)["DomainStatus"]
        if domain.get("Created") and domain.get("Endpoint") and not domain.get("Processing"):
            endpoint = domain["Endpoint"]
            if not endpoint.startswith("http"):
                endpoint = "http://" + endpoint
            break
        time.sleep(3)
    else:
        raise TimeoutError("Floci domain did not become ready")
    request(endpoint, "PUT", "/_cluster/settings", local_settings())
    model_id = ensure_model(endpoint)
    prediction = request(endpoint, "POST", f"/_plugins/_ml/models/{model_id}/_predict", {
        "parameters": {"input": ["waterproof hiking jacket"]},
    })
    vector = prediction["inference_results"][0]["output"][0]["data"]
    if len(vector) != 384:
        raise RuntimeError("Remote inference did not return 384 dimensions")
    state = Path(os.environ.get("STATE_DIR", "/state"))
    state.mkdir(parents=True, exist_ok=True)
    (state / "endpoint").write_text(endpoint)
    (state / "model-id").write_text(model_id)
    print(f"Floci domain ready: {endpoint}; remote model: {model_id}", flush=True)


if __name__ == "__main__":
    main()
