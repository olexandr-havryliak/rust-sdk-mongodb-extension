import argparse
import json
import os
import re
import time
from pathlib import Path
from urllib.parse import urlsplit

from configure import pipeline_configs, write_extension_configs


def validate_endpoint(endpoint):
    parsed = urlsplit(endpoint)
    if parsed.scheme != "https" or not parsed.hostname or parsed.username or parsed.password or parsed.path not in ("", "/") or parsed.query or parsed.fragment or parsed.port not in (None, 443):
        raise ValueError("AWS endpoint must be an HTTPS origin on port 443 without credentials")
    return endpoint.rstrip("/")


def bedrock_connector(region, role_arn):
    if not re.fullmatch(r"[a-z0-9-]+", region) or not re.fullmatch(r"arn:aws:iam::\d{12}:role/.+", role_arn):
        raise ValueError("Expected a commercial AWS region and a Bedrock IAM role ARN")
    return {
        "name": "mongodb-titan-v2",
        "description": "OpenSearch-managed on-demand document and query embeddings",
        "version": 1, "protocol": "aws_sigv4",
        "credential": {"roleArn": role_arn},
        "parameters": {"region": region, "service_name": "bedrock", "model": "amazon.titan-embed-text-v2:0",
                       "dimensions": 512, "normalize": True, "embeddingTypes": ["float"]},
        "actions": [{
            "action_type": "predict", "method": "POST",
            "url": "https://bedrock-runtime.${parameters.region}.amazonaws.com/model/${parameters.model}/invoke",
            "headers": {"content-type": "application/json", "x-amz-content-sha256": "required"},
            "request_body": '{"inputText":"${parameters.inputText}","dimensions":${parameters.dimensions},"normalize":${parameters.normalize},"embeddingTypes":${parameters.embeddingTypes}}',
            "pre_process_function": "connector.pre_process.bedrock.embedding",
            "post_process_function": "connector.post_process.bedrock.embedding",
        }],
    }


def provision_request(region, account, name, principal):
    if not re.fullmatch(r"[a-z][a-z0-9-]{2,27}", name) or not re.fullmatch(r"arn:aws:iam::\d{12}:(role|user)/.+", principal):
        raise ValueError("Expected a valid domain name and an explicit IAM client principal")
    arn = f"arn:aws:es:{region}:{account}:domain/{name}"
    return {
        "DomainName": name, "EngineVersion": "OpenSearch_2.19",
        "ClusterConfig": {"InstanceType": "t3.small.search", "InstanceCount": 1},
        "EBSOptions": {"EBSEnabled": True, "VolumeType": "gp3", "VolumeSize": 10},
        "EncryptionAtRestOptions": {"Enabled": True},
        "NodeToNodeEncryptionOptions": {"Enabled": True},
        "DomainEndpointOptions": {"EnforceHTTPS": True, "TLSSecurityPolicy": "Policy-Min-TLS-1-2-2019-07"},
        "AccessPolicies": json.dumps({"Version": "2012-10-17", "Statement": [{
            "Effect": "Allow", "Principal": {"AWS": principal}, "Action": "es:ESHttp*", "Resource": arn + "/*",
        }]}),
    }


def signed_request(session, region, endpoint, method, path, body=None):
    from botocore.auth import SigV4Auth
    from botocore.awsrequest import AWSRequest
    import requests

    data = b"" if body is None else json.dumps(body).encode()
    request = AWSRequest(method=method, url=endpoint + path, data=data, headers={"Content-Type": "application/json"})
    SigV4Auth(session.get_credentials().get_frozen_credentials(), "es", region).add_auth(request)
    response = requests.request(method, request.url, data=data, headers=dict(request.headers), timeout=30, allow_redirects=False)
    if response.status_code not in (200, 201):
        raise RuntimeError(f"AWS OpenSearch {method} {path}: HTTP {response.status_code}; check IAM/PassRole/domain policy/ML roles")
    return response.json() if response.content else {}


def main():
    import boto3

    parser = argparse.ArgumentParser(description="Configure an existing AWS domain; --provision explicitly enables billable domain creation")
    parser.add_argument("--provision", action="store_true")
    args = parser.parse_args()
    region = os.environ["AWS_DEFAULT_REGION"]
    topology = os.environ["TOPOLOGY"]
    connector = bedrock_connector(os.environ.get("AWS_BEDROCK_REGION") or region, os.environ["AWS_BEDROCK_ROLE_ARN"])
    if os.environ.get("AWS_SESSION_TOKEN") and not os.environ.get("AWS_CONNECT_ROLE_ARN"):
        raise ValueError("Aiven 4.1 needs AWS_CONNECT_ROLE_ARN to use temporary credentials through STS")
    session = boto3.Session(region_name=region)
    endpoint = os.environ.get("AWS_OPENSEARCH_ENDPOINT")
    if not endpoint:
        if not (args.provision or os.environ.get("AWS_PROVISION_DOMAIN") == "true"):
            raise ValueError("Set AWS_OPENSEARCH_ENDPOINT or explicitly enable AWS_PROVISION_DOMAIN=true")
        client = session.client("opensearch")
        name = os.environ["AWS_DOMAIN_NAME"]
        account = session.client("sts").get_caller_identity()["Account"]
        config = provision_request(region, account, name, os.environ["AWS_DOMAIN_PRINCIPAL_ARN"])
        if not any(item["DomainName"] == name for item in client.list_domain_names()["DomainNames"]):
            print("Creating a billable single-node AWS domain", flush=True)
            client.create_domain(**config)
        deadline = time.monotonic() + 1800
        while time.monotonic() < deadline:
            domain = client.describe_domain(DomainName=name)["DomainStatus"]
            if domain.get("Endpoint") and not domain.get("Processing"):
                endpoint = "https://" + domain["Endpoint"]
                break
            time.sleep(10)
        else:
            raise TimeoutError("AWS domain did not become ready; it may still be billable")
    endpoint = validate_endpoint(endpoint)
    api = lambda method, path, body=None: signed_request(session, region, endpoint, method, path, body)
    state = Path("/state")
    state.mkdir(parents=True, exist_ok=True)
    model_id = os.environ.get("OPENSEARCH_MODEL_ID")
    if not model_id:
        connector_id = api("POST", "/_plugins/_ml/connectors/_create", connector)["connector_id"]
        response = api("POST", "/_plugins/_ml/models/_register", {
            "name": "mongodb-titan-v2", "function_name": "remote", "connector_id": connector_id,
            "model_config": {"model_type": "TEXT_EMBEDDING", "embedding_dimension": 512,
                             "framework_type": "SENTENCE_TRANSFORMERS"},
        })
        model_id = response.get("model_id")
        if not model_id:
            deadline = time.monotonic() + 180
            while time.monotonic() < deadline:
                task = api("GET", "/_plugins/_ml/tasks/" + response["task_id"])
                if task.get("state") == "COMPLETED":
                    model_id = task["model_id"]
                    break
                if task.get("state") == "FAILED":
                    raise RuntimeError("AWS remote model registration failed")
                time.sleep(2)
            if not model_id:
                raise TimeoutError("AWS remote model did not register")
        api("POST", f"/_plugins/_ml/models/{model_id}/_deploy")
    deadline = time.monotonic() + 180
    while time.monotonic() < deadline:
        model = api("GET", f"/_plugins/_ml/models/{model_id}")
        if model.get("model_state") == "DEPLOYED":
            break
        if model.get("model_state") == "DEPLOY_FAILED":
            raise RuntimeError("AWS remote model deployment failed")
        time.sleep(2)
    else:
        raise TimeoutError("AWS remote model did not deploy")
    prediction = api("POST", f"/_plugins/_ml/models/{model_id}/_predict", {"parameters": {"inputText": "waterproof hiking jacket"}})
    if len(prediction["inference_results"][0]["output"][0]["data"]) != 512:
        raise RuntimeError("Expected Titan V2 float embeddings with 512 dimensions")
    for path, body in zip(("/_ingest/pipeline/mongodb-auto-embed", "/_search/pipeline/mongodb-default-model", "/_index_template/mongodb-vectors"), pipeline_configs(topology, model_id, 512)):
        api("PUT", path, body)
    (state / "endpoint").write_text(endpoint)
    (state / "model-id").write_text(model_id)
    write_extension_configs(state, endpoint, topology)
    print(f"AWS pipelines configured; model ID: {model_id}. Reuse it with OPENSEARCH_MODEL_ID on subsequent runs.", flush=True)


if __name__ == "__main__":
    main()
