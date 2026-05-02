# syntax=docker/dockerfile:1
# MongoDB 8.3 + $readLocalJsonl example. Build from repo root context.
ARG MONGO_IMAGE=mongo:8.3-rc-noble

FROM rust:bookworm AS builder
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY extension-sys-mongodb ./extension-sys-mongodb
COPY extension-sdk-mongodb ./extension-sdk-mongodb
COPY e2e-tests/e2e-extension ./e2e-tests/e2e-extension
COPY e2e-tests/mongo-extension-fuzz ./e2e-tests/mongo-extension-fuzz
COPY examples/fibonacci/fibonacci-extension ./examples/fibonacci/fibonacci-extension
COPY examples/http-fetch/http-fetch-extension ./examples/http-fetch/http-fetch-extension
COPY examples/data-federation/data-federation-extension ./examples/data-federation/data-federation-extension
RUN cargo build -p data_federation_extension --release

FROM ${MONGO_IMAGE}
USER root
RUN mkdir -p /etc/mongo/extensions /usr/local/lib/mongo-extensions /federation-data
COPY --from=builder /build/target/release/libdata_federation_extension.so /usr/local/lib/mongo-extensions/libdata_federation_extension.so
RUN { \
      echo 'sharedLibraryPath: /usr/local/lib/mongo-extensions/libdata_federation_extension.so'; \
      echo 'allowedRoot: /federation-data'; \
      echo 'allowSymlinks: false'; \
      echo 'maxLineBytes: 1048576'; \
      echo 'maxDocumentBytes: 16777216'; \
    } > /etc/mongo/extensions/datafederation.conf \
    && chmod 644 /etc/mongo/extensions/datafederation.conf \
    && chmod 755 /usr/local/lib/mongo-extensions/libdata_federation_extension.so
RUN chown -R mongodb:mongodb /etc/mongo/extensions /usr/local/lib/mongo-extensions /federation-data 2>/dev/null || \
    chown -R 999:999 /etc/mongo/extensions /usr/local/lib/mongo-extensions /federation-data
USER mongodb
EXPOSE 27017
CMD ["mongod", "--bind_ip_all", "--setParameter", "featureFlagExtensionsAPI=true", "--loadExtensions", "datafederation"]
