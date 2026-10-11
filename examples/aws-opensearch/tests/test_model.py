import importlib.util
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


class EmbeddingContractTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        spec = importlib.util.spec_from_file_location(
            "embedding_server", ROOT / "embeddings" / "server.py"
        )
        cls.server = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(cls.server)

    def test_accepts_only_nonempty_batch_of_nonempty_strings(self):
        self.assertEqual(self.server.validate_input(["rain", " snow "]), ["rain", " snow "])
        for value in (None, {}, "text", [], [1], [""], ["  "], ["ok", None]):
            with self.subTest(value=value), self.assertRaises(ValueError):
                self.server.validate_input(value)

    def test_bounds_batch_and_text_size(self):
        for value in (["a"] * 129, ["a" * 8193]):
            with self.assertRaises(ValueError):
                self.server.validate_input(value)

    def test_vectors_are_finite_and_match_batch_and_dimension(self):
        vectors = [[0.1] * 384, [0.2] * 384]
        self.assertEqual(self.server.validate_output(vectors, 2), vectors)
        for output in ([[0.1] * 383], [[float("nan")] * 384], [[float("inf")] * 384], []):
            with self.subTest(output=output), self.assertRaises(ValueError):
                self.server.validate_output(output, 1)


if __name__ == "__main__":
    unittest.main()
