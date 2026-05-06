# syntax=docker/dockerfile:1

ARG RUST_IMAGE=rust:1.89.0-bookworm@sha256:948f9b08a66e7fe01b03a98ef1c7568292e07ec2e4fe90d88c07bb14563c84ff

FROM ${RUST_IMAGE} AS build

ARG SOURCE_DATE_EPOCH

ENV CARGO_TERM_COLOR=always \
    DEBIAN_FRONTEND=noninteractive \
    LANG=C.UTF-8 \
    LC_ALL=C.UTF-8 \
    SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH} \
    TZ=UTC

RUN test -n "$SOURCE_DATE_EPOCH"

RUN apt-get update && \
    apt-get install -y --no-install-recommends \
        binutils \
        ca-certificates \
        clang \
        cmake \
        file \
        gcc \
        git \
        libclang-dev \
        make \
        pkg-config \
        protobuf-compiler \
        zstd && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /workspace/katana
COPY . .

RUN cd / && git config --global --add safe.directory /workspace/katana
RUN ./scripts/build-gnu.sh

RUN cp target/x86_64-unknown-linux-gnu/performance/katana /katana && \
    chmod 0755 /katana && \
    sha256sum /katana > /katana.sha256 && \
    sha384sum /katana > /katana.sha384 && \
    GLIBC_MIN_REQUIRED="$(objdump -T /katana | grep -oE 'GLIBC_[0-9]+(\.[0-9]+)+' | sed 's/GLIBC_//' | sort -uV | tail -1)" && \
    { \
        echo "SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH}"; \
        echo "GLIBC_BUILD_VERSION=$(dpkg-query -W -f='${Version}' libc6)"; \
        echo "GLIBC_MIN_REQUIRED=${GLIBC_MIN_REQUIRED}"; \
        rustc --version --verbose; \
        cargo --version; \
        dpkg-query -W -f='${Package}=${Version}\n' \
            binutils \
            ca-certificates \
            clang \
            cmake \
            file \
            gcc \
            git \
            libc6 \
            libclang-dev \
            make \
            pkg-config \
            protobuf-compiler \
            zstd; \
    } > /katana.build-info && \
    file /katana && \
    readelf -l /katana | grep 'Requesting program interpreter'

FROM scratch AS artifact

COPY --from=build /katana /katana
COPY --from=build /katana.build-info /katana.build-info
COPY --from=build /katana.sha256 /katana.sha256
COPY --from=build /katana.sha384 /katana.sha384
