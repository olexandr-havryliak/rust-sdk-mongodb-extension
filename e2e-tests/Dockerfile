# syntax=docker/dockerfile:1
# Default MongoDB 8.3 line (release candidate on noble). Override: docker compose build --build-arg MONGO_IMAGE=...
ARG MONGO_IMAGE=mongo:8.3-rc-noble

# `bookworm` tracks stable Rust (new enough for current `bson`/`time` deps).
FROM rust:bookworm AS builder
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY extension-sys-mongodb ./extension-sys-mongodb
COPY extension-sdk-mongodb ./extension-sdk-mongodb
COPY e2e-tests/e2e-extension ./e2e-tests/e2e-extension
COPY e2e-tests/mongo-extension-fuzz ./e2e-tests/mongo-extension-fuzz
COPY examples/fibonacci/fibonacci-extension ./examples/fibonacci/fibonacci-extension
COPY examples/http-fetch/http-fetch-extension ./examples/http-fetch/http-fetch-extension
RUN cargo build -p e2e_extension --release

FROM ${MONGO_IMAGE}
USER root
RUN mkdir -p /etc/mongo/extensions /usr/local/lib/mongo-extensions
COPY --from=builder /build/target/release/libe2e_extension.so /usr/local/lib/mongo-extensions/libe2e_extension.so
RUN { \
      echo 'sharedLibraryPath: /usr/local/lib/mongo-extensions/libe2e_extension.so'; \
      echo 'e2eExtensionParam: from_extension_yaml'; \
    } > /etc/mongo/extensions/e2e.conf \
    && chmod 644 /etc/mongo/extensions/e2e.conf \
    && chmod 755 /usr/local/lib/mongo-extensions/libe2e_extension.so
# Official image runs as `mongodb` (uid 999).
RUN chown -R mongodb:mongodb /etc/mongo/extensions /usr/local/lib/mongo-extensions 2>/dev/null || \
    chown -R 999:999 /etc/mongo/extensions /usr/local/lib/mongo-extensions
USER mongodb
EXPOSE 27017
CMD ["mongod", "--bind_ip_all", "--setParameter", "featureFlagExtensionsAPI=true", "--loadExtensions", "e2e"]
