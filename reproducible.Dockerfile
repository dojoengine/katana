# Reproducible build Dockerfile for Katana TEE
#
# Produces bit-for-bit identical builds across different machines.
# Uses a two-stage build: first stage vendors dependencies, second builds offline.
#
# Usage:
#   docker build -f reproducible.Dockerfile \
#     --build-arg SOURCE_DATE_EPOCH=$(git log -1 --format=%ct) \
#     -t katana-reproducible .
#   docker create --name extract katana-reproducible
#   docker cp extract:/katana ./katana-reproducible
#   docker rm extract

# Stage 1: Vendor dependencies
# Pin Rust image by digest (rust:1.86.0-slim-bookworm for amd64)
FROM rust@sha256:a31942999645514ff53f470d395a9b3f06e05149faa845732d0cdf132767dcbd AS vendorer

WORKDIR /src

# Copy everything needed for vendoring
COPY . .

# Generate vendor directory and cargo config
RUN mkdir -p .cargo && cargo vendor vendor/ > .cargo/config.toml

# Stage 2: Build
FROM rust@sha256:a31942999645514ff53f470d395a9b3f06e05149faa845732d0cdf132767dcbd AS builder

# Install musl toolchain and clang (needed for bindgen)
RUN apt-get update && apt-get install -y --no-install-recommends \
	musl-tools \
	musl-dev \
	clang \
	libclang-dev \
	&& rm -rf /var/lib/apt/lists/*

RUN rustup target add x86_64-unknown-linux-musl

WORKDIR /build

# SOURCE_DATE_EPOCH should be passed as build arg (e.g., git commit timestamp)
# Use: docker build --build-arg SOURCE_DATE_EPOCH=$(git log -1 --format=%ct) ...
ARG SOURCE_DATE_EPOCH
RUN test -n "$SOURCE_DATE_EPOCH" || (echo "ERROR: SOURCE_DATE_EPOCH build arg is required" && exit 1)

# Reproducibility environment variables
# Added -C link-arg=-s to strip symbols for bit-for-bit identity
ENV SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH} \
	RUSTFLAGS="--remap-path-prefix=/build=/build --remap-path-prefix=/cargo=/cargo -C target-feature=+crt-static -C link-arg=-s" \
	CARGO_HOME=/cargo \
	LANG=C.UTF-8 \
	LC_ALL=C.UTF-8 \
	TZ=UTC

# Copy source and vendored deps from stage 1
COPY --from=vendorer /src .

# Build using the vendored dependencies (--offline)
# Exclude explorer feature (requires bun to build UI assets)
RUN cargo build \
	--offline \
	--locked \
	--target x86_64-unknown-linux-musl \
	--profile performance \
	--no-default-features \
	--features "cartridge,client,init-slot,jemalloc" \
	--bin katana

RUN cp /build/target/x86_64-unknown-linux-musl/performance/katana /katana

FROM scratch AS final
COPY --from=builder /katana /katana
ENTRYPOINT ["/katana"]
