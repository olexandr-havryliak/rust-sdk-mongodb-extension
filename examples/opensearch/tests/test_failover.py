import sys

import requests
from pymongo import MongoClient

from test_sync import INDEX, OPENSEARCH_URL, MONGO_URI, os_doc, wait_until


products = MongoClient(MONGO_URI).search_demo.products
phase = sys.argv[1]
for revision in range(20):
    products.replace_one({"_id": "ha-probe"}, {
        "_id": "ha-probe", "name": f"HA {phase} {revision}",
        "description": f"Waterproof hiking backpack {phase} revision {revision}",
        "category": "ha-test", "price": revision, "inStock": True,
    }, upsert=True)
expected = f"HA {phase} 19"
wait_until(expected, lambda: os_doc("ha-probe")["_source"]["name"] == expected)
assert len(os_doc("ha-probe")["_source"]["description_embedding"]) == 384

if phase == "after":
    products.delete_one({"_id": "ha-probe"})
    wait_until("HA tombstone", lambda: os_doc("ha-probe")["_source"]["_sync_deleted"])
    tombstone = os_doc("ha-probe")
    stale = requests.put(f"{OPENSEARCH_URL}/{INDEX}/_doc/ha-probe", params={
        "version": tombstone["_version"] - 1, "version_type": "external", "pipeline": "_none"
    }, json={"name": "stale write", "_sync_deleted": False}, timeout=10)
    assert stale.status_code == 409, stale.text
    assert os_doc("ha-probe")["_source"]["_sync_deleted"] is True
    # Re-insertion with a newer Kafka offset must replace the persistent fence.
    products.insert_one({"_id": "ha-probe", "name": "HA reinsert",
                         "description": "Waterproof hiking backpack reinserted"})
    wait_until("HA reinsert", lambda: os_doc("ha-probe")["_source"].get("name") == "HA reinsert")
    assert os_doc("ha-probe")["_version"] > tombstone["_version"]
    products.delete_one({"_id": "ha-probe"})
    wait_until("HA final tombstone", lambda: os_doc("ha-probe")["_source"]["_sync_deleted"])

print(f"HA {phase}: ordered replacements, embeddings and final state verified")
