# syntax=docker/dockerfile:1
ARG MONGO_IMAGE=mongodb/mongodb-community-server:9.0-ubi9
FROM rust:1.98.1-trixie AS builder
ENV CARGO_BUILD_JOBS=2
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY extension-sdk-mongodb ./extension-sdk-mongodb
COPY extension-sys-mongodb ./extension-sys-mongodb
COPY e2e-tests ./e2e-tests
COPY .github/testing/abi-harness ./.github/testing/abi-harness
COPY .github/testing/sdk-benchmarks ./.github/testing/sdk-benchmarks
COPY examples ./examples
COPY opensearch-extension-core ./opensearch-extension-core
RUN cargo build --locked --release -p sdk_load_test
RUN cargo build --locked --release -p sdk_performance_fixture \
    && cp target/release/libsdk_performance_fixture.so /source.so
RUN cargo build --locked --release -p sdk_performance_fixture --features router \
    && cp target/release/libsdk_performance_fixture.so /router-source.so
RUN cargo build --locked --release -p sdk_performance_fixture --no-default-features --features transform \
    && cp target/release/libsdk_performance_fixture.so /transform.so
RUN cargo build --locked --release -p sdk_performance_fixture --no-default-features --features transform,router \
    && cp target/release/libsdk_performance_fixture.so /router-transform.so

FROM ${MONGO_IMAGE} AS mongo-base
USER root
RUN mkdir -p /data/db /data/configdb /etc/mongo/extensions /usr/local/lib/mongo-extensions \
    && chown -R 999:999 /data/db /data/configdb
RUN printf 'sharedLibraryPath: /usr/local/lib/mongo-extensions/source.so\n' > /etc/mongo/extensions/perf_source.conf \
    && printf 'sharedLibraryPath: /usr/local/lib/mongo-extensions/transform.so\n' > /etc/mongo/extensions/perf_transform.conf
USER 999

FROM mongo-base AS replica-set
COPY --from=builder /source.so /usr/local/lib/mongo-extensions/source.so
COPY --from=builder /transform.so /usr/local/lib/mongo-extensions/transform.so
ENTRYPOINT ["mongod"]

FROM mongo-base AS mongos
COPY --from=builder /router-source.so /usr/local/lib/mongo-extensions/source.so
COPY --from=builder /router-transform.so /usr/local/lib/mongo-extensions/transform.so
ENTRYPOINT ["mongos"]

FROM debian:trixie-slim AS driver
COPY --from=builder /build/target/release/sdk_load_test /usr/local/bin/sdk_load_test
ENTRYPOINT ["sdk_load_test"]
