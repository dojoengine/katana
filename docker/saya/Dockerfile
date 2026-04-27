# Stage 1
FROM rust:alpine AS build

RUN apk add --update alpine-sdk linux-headers openssl-dev openssl-libs-static tini python3 python3-dev py3-pip gmp-dev git

WORKDIR /src
COPY ./rust-toolchain.toml /src/

# Cache Docker layer for nightly toolchain installation
RUN cargo --version

COPY . /src/

# Install cairo-lang for apollo_starknet_os_program build script (starknet_api transitive dep).
# requirements.txt in the repo root pins cairo-lang and its full dependency set.
RUN python3 -m venv /cairo_venv && \
    . /cairo_venv/bin/activate && \
    pip install --no-cache-dir -r requirements.txt

# Build bin/persistent (STARK/Atlantic/swiftness settlement daemon)
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/src/bin/persistent/target \
    . /cairo_venv/bin/activate && \
    cargo build --release --manifest-path bin/persistent/Cargo.toml

RUN --mount=type=cache,target=/src/bin/persistent/target \
    mkdir -p ./build && \
    cp ./bin/persistent/target/release/saya ./build/

# Build bin/ops (contract deployment utilities)
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/src/bin/ops/target \
    . /cairo_venv/bin/activate && \
    cargo build --release --manifest-path bin/ops/Cargo.toml

RUN --mount=type=cache,target=/src/bin/ops/target \
    cp ./bin/ops/target/release/saya-ops ./build/

# Generate layout_bridge program from cairo-lang source
ARG CAIRO_VERSION=0.14.0.1
RUN git clone --recursive https://github.com/starkware-libs/cairo-lang \
        -b v${CAIRO_VERSION} --depth 1 /cairo-lang-src && \
    sed -i s/all_cairo/dynamic/g \
        /cairo-lang-src/src/starkware/cairo/cairo_verifier/layouts/all_cairo/cairo_verifier.cairo && \
    mkdir -p /programs && \
    . /cairo_venv/bin/activate && \
    cd /cairo-lang-src/src && \
    cairo-compile --no_debug_info --proof_mode \
        --output /programs/layout_bridge.json \
        starkware/cairo/cairo_verifier/layouts/all_cairo/cairo_verifier.cairo

# Stage 2
FROM alpine

LABEL org.opencontainers.image.source=https://github.com/dojoengine/saya

COPY --from=build /sbin/tini /tini
COPY --from=build /src/build/saya /usr/bin/
COPY --from=build /src/build/saya-ops /usr/bin/
COPY --from=build /programs /programs

ENV LAYOUT_BRIDGE_PROGRAM=/programs/layout_bridge.json

ENTRYPOINT ["/tini", "--"]
