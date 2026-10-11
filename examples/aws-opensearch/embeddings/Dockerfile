FROM python:3.11-slim
ENV HF_HOME=/model-cache PYTHONUNBUFFERED=1
RUN pip install --no-cache-dir torch==2.6.0 --index-url https://download.pytorch.org/whl/cpu \
    && pip install --no-cache-dir sentence-transformers==3.4.1
WORKDIR /app
COPY examples/aws-opensearch/embeddings/server.py /app/server.py
EXPOSE 8080
CMD ["python", "/app/server.py"]
