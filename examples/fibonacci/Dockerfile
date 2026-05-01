# syntax=docker/dockerfile:1
# MongoDB 8.3 + Fibonacci example extension. Build from repo root context.
ARG MONGO_IMAGE=mongo:8.3-rc-noble

FROM rust:bookworm AS builder
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY extension-sys-mongodb ./extension-sys-mongodb
COPY extension-sdk-mongodb ./extension-sdk-mongodb
# Workspace lists `e2e-tests/e2e-extension`; copy it so `cargo metadata` resolves (we only build fibonacci).
COPY e2e-tests/e2e-extension ./e2e-tests/e2e-extension
COPY e2e-tests/mongo-extension-fuzz ./e2e-tests/mongo-extension-fuzz
COPY examples/fibonacci/fibonacci-extension ./examples/fibonacci/fibonacci-extension
COPY examples/http-fetch/http-fetch-extension ./examples/http-fetch/http-fetch-extension
RUN cargo build -p fibonacci_extension --release

FROM ${MONGO_IMAGE}
USER root
RUN mkdir -p /etc/mongo/extensions /usr/local/lib/mongo-extensions
COPY --from=builder /build/target/release/libfibonacci_extension.so /usr/local/lib/mongo-extensions/libfibonacci_extension.so
RUN printf '%s\n' 'sharedLibraryPath: /usr/local/lib/mongo-extensions/libfibonacci_extension.so' > /etc/mongo/extensions/fibonacci.conf \
    && chmod 644 /etc/mongo/extensions/fibonacci.conf \
    && chmod 755 /usr/local/lib/mongo-extensions/libfibonacci_extension.so
RUN chown -R mongodb:mongodb /etc/mongo/extensions /usr/local/lib/mongo-extensions 2>/dev/null || \
    chown -R 999:999 /etc/mongo/extensions /usr/local/lib/mongo-extensions
USER mongodb
EXPOSE 27017
CMD ["mongod", "--bind_ip_all", "--setParameter", "featureFlagExtensionsAPI=true", "--loadExtensions", "fibonacci"]
