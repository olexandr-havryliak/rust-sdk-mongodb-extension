FROM rust:1.98.1-trixie

ENV CARGO_BUILD_JOBS=2
RUN apt-get update && apt-get install -y --no-install-recommends clang llvm python3 \
    && rm -rf /var/lib/apt/lists/*
RUN rustup component add clippy rustfmt \
    && cargo install cargo-nextest --version 0.9.143 --locked \
    && cargo install cargo-audit --version 0.22.2 --locked
RUN rustup toolchain install nightly-2026-10-01 --profile minimal --component rust-src
RUN apt-get update && apt-get install -y --no-install-recommends libclang-rt-19-dev python3-yaml \
    && rm -rf /var/lib/apt/lists/*
ENV PYTHONDONTWRITEBYTECODE=1
WORKDIR /build
