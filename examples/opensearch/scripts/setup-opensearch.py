import json
import os
import runpy
from pathlib import Path

request = runpy.run_path(str(Path(__file__).with_name("register-opensearch-model.py")))["request"]


def main():
    config = Path(os.environ.get("OPENSEARCH_CONFIG_DIR", "/config"))
    model_id = Path(os.environ.get("OPENSEARCH_MODEL_ID_FILE", "/model/model-id")).read_text().strip()
    if not model_id:
        raise ValueError("Model ID must not be empty")
    for path, filename in (
        ("/_ingest/pipeline/mongodb-auto-embed", "opensearch-ingest-pipeline.json"),
        ("/_search/pipeline/mongodb-default-model", "opensearch-search-pipeline.json"),
        ("/_index_template/mongodb-vectors", "opensearch-index-template.json"),
    ):
        body = json.loads((config / filename).read_text())
        if filename == "opensearch-ingest-pipeline.json":
            body["processors"][1]["text_embedding"]["model_id"] = model_id
        elif filename == "opensearch-search-pipeline.json":
            body["request_processors"][0]["neural_query_enricher"]["default_model_id"] = model_id
        request("PUT", path, body)
        print(f"configured {path}", flush=True)


if __name__ == "__main__":
    main()
