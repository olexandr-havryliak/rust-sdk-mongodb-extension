import json
import math
from http.server import BaseHTTPRequestHandler, HTTPServer


MODEL_NAME = "sentence-transformers/paraphrase-MiniLM-L3-v2"
DIMENSION = 384
MAX_BODY_BYTES = 2 * 1024 * 1024


def validate_input(value):
    if not isinstance(value, list) or not 1 <= len(value) <= 128:
        raise ValueError("Expected a batch of 1 to 128 strings")
    if any(not isinstance(text, str) or not text.strip() or len(text) > 8192 for text in value):
        raise ValueError("Expected nonempty text of at most 8192 characters")
    return value


def validate_output(vectors, count):
    if len(vectors) != count or any(
        len(vector) != DIMENSION
        or any(not isinstance(value, (int, float)) or not math.isfinite(value) for value in vector)
        for vector in vectors
    ):
        raise ValueError("Model returned invalid embeddings")
    return vectors


def serve(model):
    class Handler(BaseHTTPRequestHandler):
        def respond(self, status, value):
            body = json.dumps(value, allow_nan=False).encode()
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def do_GET(self):
            if self.path != "/health":
                return self.respond(404, {"error": "Not found"})
            self.respond(200, {"model": MODEL_NAME, "dimension": DIMENSION})

        def do_POST(self):
            if self.path != "/embed":
                return self.respond(404, {"error": "Not found"})
            try:
                length = int(self.headers.get("Content-Length", "0"))
                if not 0 < length <= MAX_BODY_BYTES:
                    return self.respond(413, {"error": "Invalid body size"})
                texts = validate_input(json.loads(self.rfile.read(length)))
            except (ValueError, UnicodeDecodeError):
                return self.respond(400, {"error": "Invalid embedding input"})
            try:
                vectors = model.encode(texts, normalize_embeddings=True).tolist()
                self.respond(200, validate_output(vectors, len(texts)))
            except Exception:
                self.respond(500, {"error": "Embedding inference failed"})

        def setup(self):
            super().setup()
            self.connection.settimeout(30)

    # A single inference worker keeps CPU/memory bounded for the local PoC.
    HTTPServer(("0.0.0.0", 8080), Handler).serve_forever()


if __name__ == "__main__":
    import torch
    from sentence_transformers import SentenceTransformer

    torch.set_num_threads(2)
    serve(SentenceTransformer(MODEL_NAME, device="cpu"))
