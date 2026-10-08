# syntax=docker/dockerfile:1
ARG MONGO_IMAGE=mongodb/mongodb-community-server:9.0-ubi9
FROM rust:bookworm AS builder
WORKDIR /build
COPY Cargo.lock ./
COPY extension-sys-mongodb ./extension-sys-mongodb
COPY extension-sdk-mongodb ./extension-sdk-mongodb
COPY e2e-tests/router-lookup-extension ./e2e-tests/router-lookup-extension
RUN <<'EOF'
printf '%s\n' '[workspace]' 'resolver = "2"' \
  'members = ["extension-sys-mongodb", "extension-sdk-mongodb", "e2e-tests/router-lookup-extension"]' \
  '[workspace.package]' 'version = "0.1.0"' 'edition = "2021"' 'license = "MIT"' > Cargo.toml
EOF
RUN cargo build -p router_lookup_extension --release

FROM ${MONGO_IMAGE}
USER root
RUN mkdir -p /etc/mongo/extensions /usr/local/lib/mongo-extensions
COPY --from=builder /build/target/release/librouter_lookup_extension.so /usr/local/lib/mongo-extensions/
RUN printf '%s\n' 'sharedLibraryPath: /usr/local/lib/mongo-extensions/librouter_lookup_extension.so' \
    > /etc/mongo/extensions/router_lookup.conf \
    && chmod 644 /etc/mongo/extensions/router_lookup.conf \
    && chmod 755 /usr/local/lib/mongo-extensions/librouter_lookup_extension.so
USER 999
ENTRYPOINT ["mongos"]
