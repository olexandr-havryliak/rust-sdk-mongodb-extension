FROM python:3.11-slim

WORKDIR /app

COPY examples/opensearch/indexer/requirements.txt /app/requirements.txt
RUN pip install --no-cache-dir -r /app/requirements.txt

COPY examples/opensearch/indexer/indexer.py /app/indexer.py

CMD ["python", "/app/indexer.py"]
