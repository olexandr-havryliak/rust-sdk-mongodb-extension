FROM python:3.12-slim

WORKDIR /tests

COPY examples/opensearch/tests/requirements.txt /tests/requirements.txt
RUN pip install --no-cache-dir -r /tests/requirements.txt

COPY examples/opensearch/tests/test_sync.py /tests/test_sync.py
COPY examples/opensearch/tests/test_failover.py /tests/test_failover.py

CMD ["python", "/tests/test_sync.py"]
