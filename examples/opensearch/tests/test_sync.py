import json
import sys
import time
from urllib.parse import quote

import requests
from pymongo import MongoClient


MONGO_URI = "mongodb://mongo:27017/?replicaSet=rs0"
CONNECT_URL = "http://connect:8083"
OPENSEARCH_URL = "http://opensearch:9200"
INDEX = "search_demo.products"


def wait_until(label, predicate, timeout=120, interval=2):
    deadline = time.time() + timeout
    last_error = None
    while time.time() < deadline:
        try:
            if predicate():
                return
        except Exception as exc:
            last_error = exc
        time.sleep(interval)
    raise AssertionError(f"timed out waiting for {label}; last error: {last_error}")


def os_get(path):
    response = requests.get(f"{OPENSEARCH_URL}{path}", timeout=10)
    response.raise_for_status()
    return response.json()


def os_post(path, body):
    response = requests.post(f"{OPENSEARCH_URL}{path}", json=body, timeout=10)
    response.raise_for_status()
    return response.json()


def opensearch_id(mongo_id):
    return json.dumps(mongo_id, separators=(",", ":"), sort_keys=True)


def os_doc(document_id):
    encoded = quote(document_id, safe="")
    response = requests.get(f"{OPENSEARCH_URL}/{INDEX}/_doc/{encoded}", timeout=10)
    if response.status_code == 404:
        return None
    response.raise_for_status()
    return response.json()


def assert_doc(document_id, predicate, label):
    wait_until(label, lambda: (doc := os_doc(document_id)) is not None and predicate(doc["_source"]))


def connector_running():
    status = requests.get(f"{CONNECT_URL}/connectors/mongo-products-source/status", timeout=10)
    if status.status_code != 200:
        return False
    payload = status.json()
    return (
        payload["connector"]["state"] == "RUNNING"
        and all(task["state"] == "RUNNING" for task in payload.get("tasks", []))
    )


def main():
    wait_until("MongoDB source connector", connector_running, timeout=180)
    wait_until("OpenSearch index", lambda: requests.head(f"{OPENSEARCH_URL}/{INDEX}", timeout=10).status_code == 200)

    mapping = os_get(f"/{INDEX}/_mapping")[INDEX]["mappings"]["properties"]
    assert mapping["name"]["type"] == "text"
    assert mapping["description"]["type"] == "text"
    assert mapping["description_embedding"]["type"] == "knn_vector"
    assert mapping["category"]["type"] == "keyword"
    assert mapping["price"]["type"] == "float"
    assert mapping["inStock"]["type"] == "boolean"
    assert mapping["updatedAt"]["type"] == "date"

    assert_doc(
        opensearch_id("p001"),
        lambda source: source["name"] == "Alpine Trail Pack 32L"
        and len(source.get("description_embedding", [])) == 384,
        "copy_existing p001 with embedding",
    )
    wait_until(
        "neural query without model_id",
        lambda: bool(
            os_post(
                f"/{INDEX}/_search",
                {
                    "size": 1,
                    "_source": False,
                    "query": {
                        "neural": {
                            "description_embedding": {
                                "query_text": "waterproof hiking backpack",
                                "k": 1,
                            }
                        }
                    },
                },
            )["hits"]["hits"]
        ),
    )
    assert_doc(opensearch_id("p007"), lambda source: source["category"] == "camp-kitchen", "copy_existing p007")

    mongo = MongoClient(MONGO_URI)
    products = mongo.search_demo.products

    def assert_search_document(document_id, query):
        expected = products.find_one({"_id": document_id})
        for stage, metadata in (("$search", "searchScore"), ("$vectorSearch", "vectorSearchScore")):
            def matches():
                hits = list(products.aggregate([{stage: {
                    "path": "description", "query": query, "limit": 1,
                    "filter": {"ids": {"values": [opensearch_id(document_id)]}},
                }}]))
                return hits == [expected]

            wait_until(f"{stage} restores the filtered MongoDB document {document_id}", matches, timeout=30)
            scores = list(products.aggregate([
                {stage: {
                    "path": "description", "query": query, "limit": 1,
                    "filter": {"ids": {"values": [opensearch_id(document_id)]}},
                }},
                {"$project": {"_id": 1, "score": {"$meta": metadata}}},
            ]))
            assert len(scores) == 1 and scores[0]["_id"] == document_id
            assert isinstance(scores[0].get("score"), (int, float)) and scores[0]["score"] > 0

    assert_search_document("p007", "camp mug coffee tea")

    def assert_ranked_documents(stage, metadata, query):
        stage_spec = {"path": "description", "query": query, "limit": 3}

        def ranked():
            rows = list(products.aggregate([
                {stage: stage_spec},
                {"$set": {"score": {"$meta": metadata}}},
            ]))
            if len(rows) < 2:
                return False
            values = [row.get("score") for row in rows]
            if not all(isinstance(value, (int, float)) and value > 0 for value in values):
                return False
            if values != sorted(values, reverse=True):
                return False
            for row in rows:
                row.pop("score")
                if row != products.find_one({"_id": row["_id"]}):
                    return False
            return True

        wait_until(f"{stage} returns ranked MongoDB documents for {query}", ranked, timeout=30)

    assert_ranked_documents("$search", "searchScore", "waterproof")
    assert_ranked_documents("$vectorSearch", "vectorSearchScore", "waterproof rain shell")

    products.insert_one(
        {
            "_id": "p999",
            "name": "Rain Jacket",
            "description": "Waterproof breathable shell for hiking",
            "category": "outerwear",
            "price": 149.0,
            "inStock": True,
            "updatedAt": "2026-10-02T12:02:00Z",
            "internalNotes": "do not index",
        }
    )
    assert_doc(
        opensearch_id("p999"),
        lambda source: source["name"] == "Rain Jacket" and "internalNotes" not in source,
        "insert propagation and projection",
    )

    products.replace_one(
        {"_id": "p999"},
        {
            "_id": "p999",
            "name": "Storm Jacket",
            "description": "Waterproof shell with taped seams",
            "category": "outerwear",
            "price": 179.0,
            "inStock": False,
            "updatedAt": "2026-10-02T12:03:00Z",
        },
    )
    assert_doc(
        opensearch_id("p999"),
        lambda source: source["name"] == "Storm Jacket" and source["inStock"] is False,
        "replace full reindex",
    )

    products.update_one(
        {"_id": "p007"},
        {
            "$set": {
                "description": "Updated waterproof day pack",
                "category": "updated-bags",
                "updatedAt": "2026-10-02T12:04:00Z",
            }
        },
    )
    assert_doc(
        opensearch_id("p007"),
        lambda source: source["description"] == "Updated waterproof day pack"
        and source["category"] == "updated-bags",
        "update full reindex",
    )
    assert_search_document("p007", "waterproof day pack")

    products.delete_one({"_id": "p001"})
    wait_until("delete propagation", lambda: os_doc(opensearch_id("p001"))["_source"].get("_sync_deleted") is True)

    for stage in ("$search", "$vectorSearch"):
        wait_until(
            f"{stage} returns no candidates for a deleted document",
            lambda: not list(products.aggregate([{stage: {
                "path": "description", "query": "hiking backpack", "limit": 1,
                "filter": {"ids": {"values": [opensearch_id("p001")]}},
            }}])),
            timeout=30,
        )

    p007 = os_doc(opensearch_id("p007"))
    assert p007["_id"] == opensearch_id("p007")
    assert p007["_source"]["_mongo_namespace"] == INDEX

    count = os_post(f"/{INDEX}/_count", {
        "query": {"bool": {"must_not": [{"term": {"_sync_deleted": True}}]}}
    })["count"]
    assert count == 20, f"expected 20 indexed documents after insert/delete, got {count}"

    print("OpenSearch demo sync tests passed")


if __name__ == "__main__":
    try:
        main()
    except Exception as exc:
        print(f"OpenSearch demo sync tests failed: {exc}", file=sys.stderr)
        sys.exit(1)
