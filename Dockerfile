# syntax=docker/dockerfile:1.6

ARG RUST_VERSION=nightly
FROM rustlang/rust:${RUST_VERSION} AS builder

ENV CARGO_NET_GIT_FETCH_WITH_CLI=true
WORKDIR /presenter

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libssl-dev \
    ca-certificates \
    protobuf-compiler \
    && rm -rf /var/lib/apt/lists/*

# Copy manifests first for dependency resolution
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates

RUN cargo fetch

# Build the required binaries in release mode
RUN cargo build --release --bin presenter-server --bin import_propresenter --bin ingest_bibles

# --- Runtime image ---------------------------------------------------------
FROM debian:unstable-slim AS runtime

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    openssl \
    curl \
    android-tools-adb \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /presenter

COPY --from=builder /presenter/target/release/presenter-server /usr/local/bin/presenter-server
COPY --from=builder /presenter/target/release/import_propresenter /usr/local/bin/import_propresenter
COPY --from=builder /presenter/target/release/ingest_bibles /usr/local/bin/ingest_bibles

COPY docker/demo-entrypoint.sh /usr/local/bin/demo-entrypoint.sh
RUN chmod +x /usr/local/bin/demo-entrypoint.sh

ENV PRESENTER_PORT=8080 \
    PRESENTER_DB_URL=sqlite:///data/presenter.db \
    PRESENTER_IMPORT_ROOT=/imports \
    PRESENTER_DATA_DIR=/data

EXPOSE 8080

ENTRYPOINT ["/usr/local/bin/demo-entrypoint.sh"]
CMD ["presenter-server"]
