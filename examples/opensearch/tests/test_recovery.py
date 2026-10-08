import sys

from test_sync import MongoClient, MONGO_URI, connector_running, os_doc, os_request, wait_until


client = MongoClient(MONGO_URI, serverSelectionTimeoutMS=10000)
products = client.search_demo.products


def main():
    phase = sys.argv[1]
    if phase == "before":
        products.replace_one({"_id": "recovery-probe"}, {
            "_id": "recovery-probe", "title": "Before outage", "description": "Original hiking backpack"
        }, upsert=True)
        wait_until("recovery probe indexed", lambda: os_doc("recovery-probe") is not None)
        products.replace_one({"_id": "recovery-delete"}, {
            "_id": "recovery-delete", "title": "Deleted during outage", "description": "A hiking backpack"
        }, upsert=True)
        wait_until("delete probe indexed", lambda: os_doc("recovery-delete") is not None)
    elif phase == "offline":
        products.update_one({"_id": "recovery-probe"}, {"$set": {
            "description": "A warm sleeping bag", "title": "Updated during outage"
        }})
        products.delete_one({"_id": "recovery-delete"})
        products.replace_one({"_id": "recovery-insert"}, {
            "_id": "recovery-insert", "title": "Inserted during outage", "description": "A camping tent"
        }, upsert=True)
    elif phase == "after":
        for name in ("mongo-products-source", "opensearch-products-sink"):
            wait_until(name, lambda name=name: connector_running(name))
        expected = os_request("POST", "/_ingest/pipeline/mongodb-auto-embed/_simulate", {
            "docs": [{"_source": {"description": "A warm sleeping bag"}}]
        })["docs"][0]["doc"]["_source"]
        wait_until("outage update", lambda: os_doc("recovery-probe")["_source"] == expected)
        wait_until("outage delete", lambda: os_doc("recovery-delete") is None)
        wait_until("outage insert", lambda: os_doc("recovery-insert") is not None)
        for key in ("recovery-probe", "recovery-delete", "recovery-insert"):
            products.delete_one({"_id": key})
            wait_until("recovery probe cleanup", lambda key=key: os_doc(key) is None)
        print("Connect outage recovery passed")
    else:
        raise ValueError("expected before, offline, or after")


if __name__ == "__main__":
    try:
        main()
    finally:
        client.close()
