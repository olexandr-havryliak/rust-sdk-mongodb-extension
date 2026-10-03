# syntax=docker/dockerfile:1
# MongoDB 9.0 + OpenSearch-backed $search / $vectorSearch extension PoC.
ARG MONGO_IMAGE=mongodb/mongodb-community-server:9.0-ubi9

FROM rust:bookworm AS builder
WORKDIR /build
COPY Cargo.lock ./
COPY extension-sys-mongodb ./extension-sys-mongodb
COPY extension-sdk-mongodb ./extension-sdk-mongodb
COPY examples/opensearch/extension-core ./examples/opensearch/extension-core
COPY examples/opensearch/extension ./examples/opensearch/extension
RUN <<'EOF' cat > Cargo.toml
[workspace]
resolver = "2"
members = [
    "extension-sys-mongodb",
    "extension-sdk-mongodb",
    "examples/opensearch/extension-core",
    "examples/opensearch/extension",
]

[workspace.package]
version = "0.1.0"
edition = "2021"
license = "MIT"
EOF
RUN cargo build -p opensearch_extension --release

FROM ${MONGO_IMAGE}
USER root
RUN mkdir -p /etc/mongo/extensions /usr/local/lib/mongo-extensions
COPY --from=builder /build/target/release/libopensearch_extension.so /usr/local/lib/mongo-extensions/libopensearch_extension.so
RUN { \
      echo 'sharedLibraryPath: /usr/local/lib/mongo-extensions/libopensearch_extension.so'; \
      echo 'extensionOptions:'; \
      echo '  endpoints: http://opensearch:9200'; \
    } > /etc/mongo/extensions/opensearch.conf \
    && chmod 644 /etc/mongo/extensions/opensearch.conf \
    && chmod 755 /usr/local/lib/mongo-extensions/libopensearch_extension.so
RUN mkdir -p /data/db /data/configdb \
    && chown -R mongodb:mongodb /etc/mongo/extensions /usr/local/lib/mongo-extensions /data/db /data/configdb 2>/dev/null || \
    chown -R 999:999 /etc/mongo/extensions /usr/local/lib/mongo-extensions /data/db /data/configdb
USER 999
EXPOSE 27017
CMD ["mongod", "--bind_ip_all", "--setParameter", "featureFlagExtensionsAPI=true", "--setParameter", "featureFlagExtensionsApiSignatureValidation=false", "--extensionsConfigPath", "/etc/mongo/extensions", "--loadExtensions", "opensearch"]
