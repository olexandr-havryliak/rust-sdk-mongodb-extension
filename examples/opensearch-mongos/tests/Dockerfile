FROM python:3.12-slim
WORKDIR /tests
COPY examples/opensearch-mongos/tests/requirements.txt /tests/requirements.txt
RUN pip install --no-cache-dir -r /tests/requirements.txt
COPY examples/opensearch-mongos/tests /tests
CMD ["python", "-m", "unittest", "-v", "test_configuration", "test_http", "test_flow"]
