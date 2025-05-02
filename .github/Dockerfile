# See here for image contents: https://hub.docker.com/layers/library/rust/1.81.0-slim/images/sha256-ba4ee661bb466ab49f6ceb8c6d9e9f9784bba7c6e45225187cd3c7fb1fbc12ce

ARG RUST_VERSION
ARG CLIPPY_VERSION
FROM rust:${RUST_VERSION}-slim

# https://docs.docker.com/build/building/variables/#scoping
# Consume the build arguments in the build stage
ARG RUST_VERSION
ARG CLIPPY_VERSION

# Install additional packages
RUN apt-get update && export DEBIAN_FRONTEND=noninteractive \
	&& apt-get -y install --no-install-recommends protobuf-compiler libprotobuf-dev libclang-dev libzstd-dev make pkg-config libssl-dev

RUN apt-get install -y gh libgmp3-dev software-properties-common curl git unzip

RUN curl -L https://foundry.paradigm.xyz/ | bash && . /root/.bashrc && foundryup
ENV PATH="${PATH}:/root/.foundry/bin"

# Install cargo-binstall
RUN curl -L --proto '=https' --tlsv1.2 -sSf https://raw.githubusercontent.com/cargo-bins/cargo-binstall/main/install-from-binstall-release.sh | bash

RUN rustup toolchain install ${RUST_VERSION} && \
	rustup default ${RUST_VERSION} && \
	rustup component add clippy && \
	rustup component add rustfmt && \
	rustup component add llvm-tools-preview

RUN rustup toolchain install ${CLIPPY_VERSION} && \
	rustup component add rustfmt clippy --toolchain ${CLIPPY_VERSION}

RUN rustup target add x86_64-pc-windows-msvc && \
	rustup target add wasm32-unknown-unknown

RUN cargo install cargo-get cargo-nextest cargo-llvm-cov cargo-release

# Platform specific tooling
ARG TARGETPLATFORM
RUN if [ "$TARGETPLATFORM" = "linux/arm64" ] ; then \
	curl -L https://github.com/Orange-OpenSource/hurl/releases/download/4.1.0/hurl-4.1.0-aarch64-unknown-linux-gnu.tar.gz -o hurl.tar.gz && \
	tar -xzf hurl.tar.gz && \
	mv hurl-4.1.0-aarch64-unknown-linux-gnu/hurl /usr/local/bin/ && \
	rm -r hurl-4.1.0-aarch64-unknown-linux-gnu && \
	rm hurl.tar.gz && \
	rustup component add llvm-tools-preview --toolchain ${RUST_VERSION}-aarch64-unknown-linux-gnu; \
	elif [ "$TARGETPLATFORM" = "linux/amd64" ] ; then \
	curl -L https://github.com/Orange-OpenSource/hurl/releases/download/4.1.0/hurl-4.1.0-x86_64-unknown-linux-gnu.tar.gz -o hurl.tar.gz && \
	tar -xzf hurl.tar.gz && \
	mv hurl-4.1.0-x86_64-unknown-linux-gnu/hurl /usr/local/bin/ && \
	rm -r hurl-4.1.0-x86_64-unknown-linux-gnu && \
	rm hurl.tar.gz && \
	rustup component add llvm-tools-preview --toolchain ${RUST_VERSION}-x86_64-unknown-linux-gnu && \
	rustup target add x86_64-fortanix-unknown-sgx --toolchain ${CLIPPY_VERSION}; \
	fi

RUN curl --proto '=https' --tlsv1.2 -sSf https://docs.swmansion.com/scarb/install.sh | bash

RUN chown -R root:root /usr/local/cargo
RUN chmod -R 700 /usr/local/cargo

# Set environment variables for LLVM
ENV MLIR_SYS_190_PREFIX=/usr/lib/llvm-19
ENV	LLVM_SYS_191_PREFIX=/usr/lib/llvm-19
ENV TABLEGEN_190_PREFIX=/usr/lib/llvm-19

# Install LLVM and Cairo native dependencies
RUN apt-get install -y \
	g++ \
	llvm-19 \
	llvm-19-dev \
	llvm-19-runtime \
	clang-19 \
	clang-tools-19 \
	lld-19 \
	libpolly-19-dev \
	libmlir-19-dev \
	mlir-19-tools

# Install bun for Explorer
RUN apt-get install -y unzip
RUN curl -fsSL https://bun.sh/install | bash
ENV PATH="/root/.bun/bin:$PATH"

ENV PYENV_ROOT="/root/.pyenv"
ENV PATH="/root/.pyenv/bin:$PATH"
RUN curl -fsSL https://pyenv.run | bash
RUN echo 'export PYENV_ROOT="/root/.pyenv"' >> /root/.bashrc && \
	echo 'export PATH="$PYENV_ROOT/bin:$PATH"' >> /root/.bashrc && \
	echo 'eval "$(pyenv init -)"' >> /root/.bashrc && \
	echo 'eval "$(pyenv virtualenv-init -)"' >> /root/.bashrc

# Install caddy
RUN apt install -y debian-keyring debian-archive-keyring apt-transport-https curl && \
	curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/gpg.key' | gpg --dearmor -o /usr/share/keyrings/caddy-stable-archive-keyring.gpg && \
	curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/debian.deb.txt' | tee /etc/apt/sources.list.d/caddy-stable.list && \
	apt update && \
	apt install caddy

# Add shims to PATH for non-login shell usage
ENV PATH="/root/.pyenv/shims:$PATH"

# Install Katana manually from GitHub
RUN cargo install --git https://github.com/dojoengine/katana katana --features native --locked
