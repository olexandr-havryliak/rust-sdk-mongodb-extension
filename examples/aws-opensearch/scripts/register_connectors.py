import os
from pathlib import Path

from configure import connector_configs
from setup_local import request


def main():
    sink, sources = connector_configs(os.environ["TOPOLOGY"], Path("/state/endpoint").read_text().strip(),
                                     os.environ.get("OPENSEARCH_AWS_REGION"), os.environ.get("AWS_CONNECT_ROLE_ARN") or None)
    for name, config in {"opensearch-aws-sink": sink, **sources}.items():
        request("http://connect:8083", "PUT", f"/connectors/{name}/config", config)
        print(f"Configured {name}", flush=True)


if __name__ == "__main__":
    main()
