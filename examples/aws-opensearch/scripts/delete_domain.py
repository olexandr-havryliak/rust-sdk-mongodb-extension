import boto3


def main():
    client = boto3.client("opensearch", endpoint_url="http://floci:4566")
    if any(item["DomainName"] == "mongodb-vector-demo" for item in client.list_domain_names()["DomainNames"]):
        client.delete_domain(DomainName="mongodb-vector-demo")
        print("Deleted local Floci domain", flush=True)


if __name__ == "__main__":
    main()
