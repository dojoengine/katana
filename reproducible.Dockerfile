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
# Added -C link-arg=-s to strip symbols for bit-for-bit identity
ENV SOURCE_DATE_EPOCH=1735689600 \
	RUSTFLAGS="--remap-path-prefix=/build=/build --remap-path-prefix=/cargo=/cargo -C target-feature=+crt-static -C link-arg=-s" \
	CARGO_HOME=/cargo \
	LANG=C.UTF-8 \
	LC_ALL=C.UTF-8 \
	TZ=UTC

# Copy everything (including the 'vendor' folder and '.cargo/config.toml')
COPY . .

# Build using the vendored dependencies (--offline)
# and your custom performance profile
RUN cargo build \
	--offline \
	--locked \
	--target x86_64-unknown-linux-musl \
	--profile performance \
	--bin katana

RUN cp /build/target/x86_64-unknown-linux-musl/performance/katana /katana

FROM scratch AS final
COPY --from=builder /katana /katana
ENTRYPOINT ["/katana"]
