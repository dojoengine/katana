# Multi-stage Dockerfile optimized for layer caching

ARG RUST_VERSION
ARG CLIPPY_VERSION

# ===== System Dependencies Stage =====
# This layer rarely changes, so it will be cached
FROM rust:${RUST_VERSION}-slim as system-deps

ARG RUST_VERSION
ARG CLIPPY_VERSION

# Install system packages in a single layer
RUN apt-get update && export DEBIAN_FRONTEND=noninteractive && \
	apt-get -y install --no-install-recommends \
	# Build dependencies
	protobuf-compiler libprotobuf-dev libclang-dev libzstd-dev \
	make pkg-config libssl-dev g++ \
	# Headless chromium shared libs
	ca-certificates fonts-liberation libasound2 libatk-bridge2.0-0 \
	libatk1.0-0 libc6 libcairo2 libcups2 libdbus-1-3 libexpat1 \
	libfontconfig1 libgbm1 libgcc1 libglib2.0-0 libgtk-3-0 \
	libnspr4 libnss3 libpango-1.0-0 libpangocairo-1.0-0 libstdc++6 \
	libx11-6 libx11-xcb1 libxcb1 libxcomposite1 libxcursor1 \
	libxdamage1 libxext6 libxfixes3 libxi6 libxrandr2 libxrender1 \
	libxss1 libxtst6 lsb-release wget xdg-utils \
	# Additional tools
	gh libgmp3-dev software-properties-common curl git unzip bc \
	debian-keyring debian-archive-keyring apt-transport-https \
	# LLVM dependencies
	llvm-19 llvm-19-dev llvm-19-runtime clang-19 clang-tools-19 \
	lld-19 libpolly-19-dev libmlir-19-dev mlir-19-tools

RUN	rm -rf /var/lib/apt/lists/*

# ===== Rust Toolchain Stage =====
# This layer changes only when Rust versions change
FROM system-deps as rust-toolchain

# Install foundry
RUN curl -L https://foundry.paradigm.xyz/ | bash && . /root/.bashrc && foundryup
ENV PATH="${PATH}:/root/.foundry/bin"

# Install cargo-binstall
RUN curl -L --proto '=https' --tlsv1.2 -sSf https://raw.githubusercontent.com/cargo-bins/cargo-binstall/main/install-from-binstall-release.sh | bash

# Install Rust toolchains and components
RUN rustup toolchain install ${RUST_VERSION} && \
	rustup default ${RUST_VERSION} && \
	rustup component add clippy rustfmt llvm-tools-preview

RUN rustup toolchain install ${CLIPPY_VERSION} && \
	rustup component add rustfmt clippy --toolchain ${CLIPPY_VERSION}

RUN rustup target add x86_64-pc-windows-msvc wasm32-unknown-unknown

# Platform specific components
ARG TARGETPLATFORM
RUN if [ "$TARGETPLATFORM" = "linux/arm64" ] ; then \
	rustup component add llvm-tools-preview --toolchain ${RUST_VERSION}-aarch64-unknown-linux-gnu; \
	elif [ "$TARGETPLATFORM" = "linux/amd64" ] ; then \
	rustup component add llvm-tools-preview --toolchain ${RUST_VERSION}-x86_64-unknown-linux-gnu && \
	rustup target add x86_64-fortanix-unknown-sgx --toolchain ${CLIPPY_VERSION}; \
	fi

# Install cargo tools
RUN cargo install cargo-get@1.4.0 cargo-nextest@0.9.92 cargo-llvm-cov@0.6.17 cargo-release@0.25.18 --locked --force

# ===== External Tools Stage =====
# These tools change less frequently
FROM rust-toolchain as external-tools

# Install hurl (platform specific)
ARG TARGETPLATFORM
RUN if [ "$TARGETPLATFORM" = "linux/arm64" ] ; then \
	curl -L https://github.com/Orange-OpenSource/hurl/releases/download/4.1.0/hurl-4.1.0-aarch64-unknown-linux-gnu.tar.gz -o hurl.tar.gz && \
	tar -xzf hurl.tar.gz && \
	mv hurl-4.1.0-aarch64-unknown-linux-gnu/hurl /usr/local/bin/ && \
	rm -r hurl-4.1.0-aarch64-unknown-linux-gnu && rm hurl.tar.gz; \
	elif [ "$TARGETPLATFORM" = "linux/amd64" ] ; then \
	curl -L https://github.com/Orange-OpenSource/hurl/releases/download/4.1.0/hurl-4.1.0-x86_64-unknown-linux-gnu.tar.gz -o hurl.tar.gz && \
	tar -xzf hurl.tar.gz && \
	mv hurl-4.1.0-x86_64-unknown-linux-gnu/hurl /usr/local/bin/ && \
	rm -r hurl-4.1.0-x86_64-unknown-linux-gnu && rm hurl.tar.gz; \
	fi

# Install scarb
RUN curl --proto '=https' --tlsv1.2 -sSf https://docs.swmansion.com/scarb/install.sh | bash -s -- -v 2.8.2
ENV PATH="/root/.local/bin:$PATH"

# Install bun
RUN curl -fsSL https://bun.sh/install | bash
ENV PATH="/root/.bun/bin:$PATH"

# Install pyenv
ENV PYENV_ROOT="/root/.pyenv"
ENV PATH="/root/.pyenv/bin:$PATH"
RUN curl -fsSL https://pyenv.run | bash
ENV PATH="/root/.pyenv/shims:$PATH"

# Install caddy
RUN curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/gpg.key' | gpg --dearmor -o /usr/share/keyrings/caddy-stable-archive-keyring.gpg && \
	curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/debian.deb.txt' | tee /etc/apt/sources.list.d/caddy-stable.list && \
	apt update && apt install caddy && rm -rf /var/lib/apt/lists/*

# Set permissions
RUN chown -R root:root /usr/local/cargo && chmod -R 700 /usr/local/cargo

# Set LLVM environment variables
ENV MLIR_SYS_190_PREFIX=/usr/lib/llvm-19
ENV LLVM_SYS_191_PREFIX=/usr/lib/llvm-19
ENV TABLEGEN_190_PREFIX=/usr/lib/llvm-19

# ===== Dependency Builder Stage =====
# This stage builds only Rust dependencies, separate from source code
FROM external-tools as builder

WORKDIR /katana
COPY . .

# Build katana
RUN cargo install --path bin/katana --features native --locked
# Verify the binary works
RUN katana --version

CMD ["katana"]
