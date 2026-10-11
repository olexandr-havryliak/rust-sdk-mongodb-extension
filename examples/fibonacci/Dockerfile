# syntax=docker/dockerfile:1
# MongoDB 9.0 + Fibonacci example extension. Build from repo root context.
ARG MONGO_IMAGE=mongodb/mongodb-community-server:9.0-ubi9

FROM rust:bookworm AS builder
ENV CARGO_BUILD_JOBS=2
WORKDIR /build
COPY Cargo.lock ./
COPY extension-sys-mongodb ./extension-sys-mongodb
COPY extension-sdk-mongodb ./extension-sdk-mongodb
COPY examples/fibonacci/fibonacci-extension ./examples/fibonacci/fibonacci-extension
RUN <<'EOF' cat > Cargo.toml
[workspace]
resolver = "2"
members = ["extension-sys-mongodb", "extension-sdk-mongodb", "examples/fibonacci/fibonacci-extension"]
[workspace.package]
version = "0.1.0"
edition = "2021"
license = "MIT"
EOF
RUN cargo build -p fibonacci_extension --release

FROM ${MONGO_IMAGE}
USER root
RUN mkdir -p /etc/mongo/extensions /usr/local/lib/mongo-extensions
COPY --from=builder /build/target/release/libfibonacci_extension.so /usr/local/lib/mongo-extensions/libfibonacci_extension.so
RUN printf '%s\n' 'sharedLibraryPath: /usr/local/lib/mongo-extensions/libfibonacci_extension.so' > /etc/mongo/extensions/fibonacci.conf \
    && chmod 644 /etc/mongo/extensions/fibonacci.conf \
    && chmod 755 /usr/local/lib/mongo-extensions/libfibonacci_extension.so
RUN mkdir -p /data/db /data/configdb \
    && chown -R mongodb:mongodb /etc/mongo/extensions /usr/local/lib/mongo-extensions /data/db /data/configdb 2>/dev/null || \
    chown -R 999:999 /etc/mongo/extensions /usr/local/lib/mongo-extensions /data/db /data/configdb
USER 999
EXPOSE 27017
CMD ["mongod", "--bind_ip_all", "--wiredTigerCacheSizeGB", "0.25", "--setParameter", "featureFlagExtensionsAPI=true", "--setParameter", "featureFlagExtensionsApiSignatureValidation=false", "--extensionsConfigPath", "/etc/mongo/extensions", "--loadExtensions", "fibonacci"]
