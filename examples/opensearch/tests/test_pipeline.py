import json
import math
import os
import unittest
import urllib.request
import uuid


URL = os.environ.get("OPENSEARCH_URL", "http://opensearch:9200")
PIPELINE = "/_ingest/pipeline/mongodb-auto-embed/_simulate"


def request(method, path, body=None):
    data = None if body is None else json.dumps(body).encode()
    req = urllib.request.Request(URL + path, data=data,
                                 headers={"Content-Type": "application/json"}, method=method)
    with urllib.request.urlopen(req, timeout=60) as response:
        return json.load(response)


class PipelineTests(unittest.TestCase):
    def simulate(self, *documents):
        return request("POST", PIPELINE, {"docs": [
            {"_index": "mongodb.pipeline.probe", "_id": str(i), "_source": doc}
            for i, doc in enumerate(documents)
        ]})["docs"]

    def assert_vectors(self, result, fields):
        self.assertNotIn("error", result, result)
        source = result["doc"]["_source"]
        self.assertEqual(set(source), {field + "_embedding" for field in fields})
        for vector in source.values():
            self.assertEqual(len(vector), 384)
            self.assertTrue(all(isinstance(x, (int, float)) and math.isfinite(x) for x in vector))
        return source

    def test_arbitrary_fields_get_separate_embeddings_and_text_is_removed(self):
        source = self.assert_vectors(self.simulate({
            "title": "waterproof hiking backpack", "summary": "warm sleeping bag"
        })[0], ["title", "summary"])
        self.assertNotEqual(source["title_embedding"], source["summary_embedding"])

    def test_field_names_and_documents_do_not_mix_embeddings(self):
        first, second = self.simulate({"name": "a red backpack"}, {"unseen": "a red backpack"})
        a = self.assert_vectors(first, ["name"])
        b = self.assert_vectors(second, ["unseen"])
        self.assertEqual(a["name_embedding"], b["unseen_embedding"])

    def test_multiple_fields_match_independent_inference(self):
        document = {"alpha": "waterproof hiking pack", "beta": "warm sleeping bag", "gamma": "camp stove"}
        together = self.assert_vectors(self.simulate(document)[0], document)
        for field, text in document.items():
            alone = self.assert_vectors(self.simulate({field: text})[0], [field])
            self.assertEqual(together[field + "_embedding"], alone[field + "_embedding"])

    def test_new_namespace_uses_template_and_default_query_model(self):
        index = "mongodb.pipeline." + uuid.uuid4().hex
        request("PUT", f"/{index}/_doc/probe?refresh=true", {"previously_unseen": "waterproof backpack"})
        try:
            mapping = request("GET", f"/{index}/_mapping")[index]["mappings"]["properties"]
            self.assertEqual(set(mapping), {"previously_unseen_embedding"})
            self.assertEqual(mapping["previously_unseen_embedding"]["type"], "knn_vector")
            self.assertEqual(mapping["previously_unseen_embedding"]["dimension"], 384)
            result = request("POST", f"/{index}/_search", {"query": {"neural": {
                "previously_unseen_embedding": {"query_text": "hiking pack", "k": 1}
            }}})
            self.assertEqual(result["hits"]["hits"][0]["_id"], "probe")
        finally:
            request("DELETE", "/" + index)

    def test_rejects_non_text_and_reserved_fields(self):
        for document in ({"price": 3}, {"title": ["a", "b"]}, {"title": None},
                         {"title": {"nested": "text"}}, {"title": "  "},
                         {"title_embedding": "collision"}, {"_vector_inputs": "collision"},
                         {"nested.title": "not a flat field"}):
            with self.subTest(document=document):
                self.assertIn("error", self.simulate(document)[0])

    def test_empty_projection_is_safe(self):
        self.assert_vectors(self.simulate({})[0], [])


if __name__ == "__main__":
    unittest.main()
