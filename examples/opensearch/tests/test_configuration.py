import json
import unittest
from pathlib import Path


CONFIG = Path(__file__).resolve().parents[1] / "config"


class ConfigurationTests(unittest.TestCase):
    def load(self, filename):
        return json.loads((CONFIG / filename).read_text())

    def test_template_is_scoped_to_mongodb_and_maps_arbitrary_embeddings(self):
        template = self.load("opensearch-index-template.json")
        self.assertEqual(template["index_patterns"], ["mongodb.*"])
        mapping = template["template"]["mappings"]["dynamic_templates"][0]["embeddings"]
        self.assertEqual(mapping["match"], "*_embedding")
        self.assertEqual(mapping["mapping"]["type"], "knn_vector")
        self.assertEqual(mapping["mapping"]["dimension"], 384)

    def test_sink_uses_full_replacements_and_physical_deletes(self):
        sink = self.load("opensearch-sink-connector.json")
        self.assertEqual(sink["tasks.max"], "1")
        self.assertEqual(sink["max.in.flight.requests"], "1")
        self.assertEqual(sink["index.write.method"], "insert")
        self.assertEqual(sink["behavior.on.null.values"], "delete")
        self.assertEqual(sink["key.ignore"], "false")
        self.assertEqual(sink["schema.ignore"], "true")
        self.assertEqual(sink["errors.tolerance"], "none")
        self.assertEqual(sink["topics"], "mongodb.search_demo.products")

    def test_projection_is_in_kafka_and_preserves_delete_keys(self):
        sink = self.load("opensearch-sink-connector.json")
        self.assertEqual(sink["transforms.key.field"], "_id")
        self.assertEqual(sink["transforms.fields.include"], "description")
        source = self.load("mongo-source-connector.json")
        self.assertEqual(json.loads(source["topic.namespace.map"]),
                         {"search_demo.products": "mongodb.search_demo.products"})
        self.assertEqual(source["publish.full.document.only.tombstone.on.delete"], "true")

    def test_demo_documents_have_only_title_and_description_besides_id(self):
        dataset = CONFIG.parent / "datasets" / "outdoor-products.json"
        documents = json.loads(dataset.read_text())
        self.assertEqual(len(documents), 20)
        self.assertEqual(len({doc["_id"] for doc in documents}), 20)
        for doc in documents:
            self.assertEqual(set(doc), {"_id", "title", "description"})
            self.assertTrue(all(isinstance(value, str) and value.strip() for value in doc.values()))

    def test_optional_namespace_routes_to_its_own_topic_and_indexes_description_only(self):
        source = self.load("mongo-articles-source-connector.json")
        sink = self.load("opensearch-articles-sink-connector.json")
        self.assertEqual((source["database"], source["collection"]), ("catalog", "articles"))
        self.assertEqual(source["startup.mode"], "copy_existing")
        self.assertEqual(source["startup.mode.copy.existing.namespace.regex"], "^catalog\\.articles$")
        self.assertEqual(json.loads(source["topic.namespace.map"]),
                         {"catalog.articles": "mongodb.catalog.articles"})
        self.assertEqual(sink["topics"], "mongodb.catalog.articles")
        self.assertEqual(sink["transforms.fields.include"], "description")


if __name__ == "__main__":
    unittest.main()
