import gzip
import json
import os
import unittest
import urllib.request


URL = os.environ.get("OPENSEARCH_URL", "http://opensearch:9200")


class HttpCompressionTests(unittest.TestCase):
    def assert_complete_response(self, encoding):
        request = urllib.request.Request(URL + "/", headers={"Accept-Encoding": encoding})
        with urllib.request.urlopen(request, timeout=5) as response:
            self.assertEqual(response.status, 200)
            body = response.read()
            if response.headers.get("Content-Encoding") == "gzip":
                body = gzip.decompress(body)
            self.assertEqual(json.loads(body)["version"]["distribution"], "opensearch")

    def test_zstd_only_request_does_not_hang(self):
        self.assert_complete_response("zstd")

    def test_zstd_with_gzip_request_does_not_hang(self):
        self.assert_complete_response("gzip, zstd")


if __name__ == "__main__":
    unittest.main(verbosity=2)
