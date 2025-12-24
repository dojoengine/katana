# Reproducible build Dockerfile for Katana TEE
#
# Produces bit-for-bit identical builds across different machines.
#
# Usage:
#   docker build -f reproducible.Dockerfile -t katana-reproducible .
#   docker create --name extract katana-reproducible
#   docker cp extract:/katana ./katana-reproducible
#   docker rm extract

# Pin Rust image by digest (rust:1.86.0-slim-bookworm for amd64)
FROM rust@sha256:a044f7ab9a762f95be2ee7eb2c49e4d4a4ec60011210de9f7da01d552cae3a55 AS builder

# Install musl toolchain for static linking
RUN apt-get update && apt-get install -y --no-install-recommends \
	musl-tools \
	musl-dev \
	&& rm -rf /var/lib/apt/lists/*

RUN rustup target add x86_64-unknown-linux-musl

WORKDIR /build

# Reproducibility environment variables
ENV SOURCE_DATE_EPOCH=1735689600 \
	RUSTFLAGS="--remap-path-prefix=/build=/build --remap-path-prefix=/root/.cargo=/cargo -C target-feature=+crt-static" \
	CARGO_HOME=/cargo \
	LANG=C.UTF-8

# Copy source (respects .dockerignore)
COPY . .

# Build static binary
RUN cargo build \
	--locked \
	--target x86_64-unknown-linux-musl \
	--bin katana \
	--profile performance

RUN cp /build/target/x86_64-unknown-linux-musl/performance/katana /katana

# Minimal final stage
FROM scratch AS final
COPY --from=builder /katana /katana
