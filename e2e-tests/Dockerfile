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
COPY examples/data-federation/data-federation-extension ./examples/data-federation/data-federation-extension
RUN cargo build -p e2e_extension -p fibonacci_extension -p data_federation_extension --release

FROM ${MONGO_IMAGE}
USER root
RUN mkdir -p /etc/mongo/extensions /usr/local/lib/mongo-extensions
COPY --from=builder /build/target/release/libe2e_extension.so /usr/local/lib/mongo-extensions/libe2e_extension.so
COPY --from=builder /build/target/release/libfibonacci_extension.so /usr/local/lib/mongo-extensions/libfibonacci_extension.so
COPY --from=builder /build/target/release/libdata_federation_extension.so /usr/local/lib/mongo-extensions/libdata_federation_extension.so
RUN mkdir -p /federation-data \
    && { \
      echo 'sharedLibraryPath: /usr/local/lib/mongo-extensions/libe2e_extension.so'; \
      echo 'e2eExtensionParam: from_extension_yaml'; \
    } > /etc/mongo/extensions/e2e.conf \
    && printf '%s\n' 'sharedLibraryPath: /usr/local/lib/mongo-extensions/libfibonacci_extension.so' > /etc/mongo/extensions/fibonacci.conf \
    && { \
      echo 'sharedLibraryPath: /usr/local/lib/mongo-extensions/libdata_federation_extension.so'; \
      echo 'allowedRoot: /federation-data'; \
      echo 'allowSymlinks: false'; \
      echo 'maxLineBytes: 1048576'; \
      echo 'maxDocumentBytes: 16777216'; \
    } > /etc/mongo/extensions/datafederation.conf \
    && chmod 644 /etc/mongo/extensions/e2e.conf /etc/mongo/extensions/fibonacci.conf /etc/mongo/extensions/datafederation.conf \
    && chmod 755 /usr/local/lib/mongo-extensions/libe2e_extension.so /usr/local/lib/mongo-extensions/libfibonacci_extension.so /usr/local/lib/mongo-extensions/libdata_federation_extension.so
# Official image runs as `mongodb` (uid 999).
RUN chown -R mongodb:mongodb /etc/mongo/extensions /usr/local/lib/mongo-extensions /federation-data 2>/dev/null || \
    chown -R 999:999 /etc/mongo/extensions /usr/local/lib/mongo-extensions /federation-data
USER mongodb
EXPOSE 27017
CMD ["mongod", "--bind_ip_all", "--setParameter", "featureFlagExtensionsAPI=true", "--loadExtensions", "e2e,fibonacci,datafederation"]
