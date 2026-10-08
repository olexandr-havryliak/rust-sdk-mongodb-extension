# syntax=docker/dockerfile:1
ARG MONGO_IMAGE=mongodb/mongodb-community-server:9.0-ubi9
FROM rust:bookworm AS builder
WORKDIR /build
COPY Cargo.lock ./
COPY extension-sys-mongodb ./extension-sys-mongodb
COPY extension-sdk-mongodb ./extension-sdk-mongodb
COPY examples/opensearch/extension-core ./examples/opensearch/extension-core
COPY examples/opensearch-mongos/extension ./examples/opensearch-mongos/extension
RUN <<'EOF' cat > Cargo.toml
[workspace]
resolver = "2"
members = ["extension-sys-mongodb", "extension-sdk-mongodb", "examples/opensearch/extension-core", "examples/opensearch-mongos/extension"]
[workspace.package]
version = "0.1.0"
edition = "2021"
license = "MIT"
EOF
RUN cargo build -p opensearch_mongos_extension --release

FROM ${MONGO_IMAGE}
USER root
RUN mkdir -p /etc/mongo/extensions /usr/local/lib/mongo-extensions
COPY --from=builder /build/target/release/libopensearch_mongos_extension.so /usr/local/lib/mongo-extensions/
RUN printf 'sharedLibraryPath: /usr/local/lib/mongo-extensions/libopensearch_mongos_extension.so\nextensionOptions:\n  endpoints: http://opensearch:9200\n' > /etc/mongo/extensions/opensearch_mongos.conf \
    && chmod 644 /etc/mongo/extensions/opensearch_mongos.conf \
    && chmod 755 /usr/local/lib/mongo-extensions/libopensearch_mongos_extension.so
USER 999
ENTRYPOINT ["mongos"]
