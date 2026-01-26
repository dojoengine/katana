# VM Image Dockerfile for Katana TEE (AMD SEV-SNP Attestation Build)
#
# Creates reproducible boot components using AMD's OVMF fork with SEV-specific
# attestation features for Katana in AMD SEV-SNP TEEs.
#
# Key differences from standard OVMF:
# - SecretPei/SecretDxe: SEV secret injection support
# - BlobVerifierLibSevHashes: Hash-based blob verification
# - Embedded GRUB with LUKS/cryptodisk support
# - SNP-specific memory layout and work areas
#
# Usage:
#   docker build -f vm-image.Dockerfile \
#     --build-arg SOURCE_DATE_EPOCH=$(git log -1 --format=%ct) \
#     -t katana-vm-image .

# =============================================================================
# Stage 1: Download pinned Ubuntu packages (kernel, busybox)
# =============================================================================
FROM ubuntu:24.04@sha256:c35e29c9450151419d9448b0fd75374fec4fff364a27f176fb458d472dfc9e54 AS package-fetcher

ARG SOURCE_DATE_EPOCH
ENV SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH} \
    DEBIAN_FRONTEND=noninteractive \
    TZ=UTC \
    LANG=C.UTF-8

RUN test -n "$SOURCE_DATE_EPOCH" || (echo "ERROR: SOURCE_DATE_EPOCH build arg is required" && exit 1)

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    wget \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /packages
RUN apt-get update

# Download kernel and busybox (OVMF built from AMD source)
RUN apt-get download linux-image-generic busybox-static cpio && \
    KERNEL_VER=$(ls linux-image-generic_*.deb | grep -oP '[0-9]+\.[0-9]+\.[0-9]+-[0-9]+' | head -1) && \
    apt-get download linux-image-unsigned-${KERNEL_VER}-generic

RUN ls -lh *.deb && rm -rf /var/lib/apt/lists/*

# =============================================================================
# Stage 2: Build AMD SEV-specific OVMF from source
# =============================================================================
FROM ubuntu:24.04@sha256:c35e29c9450151419d9448b0fd75374fec4fff364a27f176fb458d472dfc9e54 AS ovmf-builder

ARG SOURCE_DATE_EPOCH
ARG OVMF_BRANCH=snp-latest
ARG OVMF_COMMIT=fbe0805b2091393406952e84724188f8c1941837
ENV SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH} \
    DEBIAN_FRONTEND=noninteractive \
    TZ=UTC \
    LANG=C.UTF-8

# Install EDK2 build dependencies + GRUB tools for AmdSevX64 prebuild
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential \
    git \
    python3 \
    python3-venv \
    uuid-dev \
    iasl \
    nasm \
    ca-certificates \
    # Required for AmdSev GRUB prebuild (grub.sh)
    grub-efi-amd64-bin \
    grub2-common \
    dosfstools \
    mtools \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build

# Clone AMD's OVMF fork with SEV-SNP support
RUN git clone --single-branch -b ${OVMF_BRANCH} --depth 100 \
    https://github.com/AMDESE/ovmf.git ovmf && \
    cd ovmf && \
    if [ -n "${OVMF_COMMIT}" ]; then git checkout ${OVMF_COMMIT}; fi && \
    git submodule update --init --recursive

# Patch grub.sh to remove GRUB modules not available in Ubuntu 24.04:
# - linuxefi: deprecated, merged into linux module in GRUB 2.12+
# - sevsecret: not included in Ubuntu's grub-efi-amd64-bin package
RUN cd ovmf && \
    sed -i '/linuxefi/d' OvmfPkg/AmdSev/Grub/grub.sh && \
    sed -i '/sevsecret/d' OvmfPkg/AmdSev/Grub/grub.sh

# Build BaseTools
RUN cd ovmf && make -C BaseTools -j$(nproc)

# Build AMD SEV-specific OVMF (AmdSevX64.dsc)
# This includes:
# - SecretPei/SecretDxe for SEV secret injection
# - BlobVerifierLibSevHashes for attestation
# - Embedded GRUB with LUKS/cryptodisk support
# - SNP-specific memory regions (GHCB, secrets page, CPUID page)
RUN cd ovmf && \
    . ./edksetup.sh && \
    build -q --cmd-len=64436 \
    -n $(nproc) \
    -t GCC5 \
    -a X64 \
    -p OvmfPkg/AmdSev/AmdSevX64.dsc \
    -b RELEASE

# Copy built artifacts
# AmdSev build outputs to Build/AmdSev/
RUN mkdir -p /output && \
    cp /build/ovmf/Build/AmdSev/RELEASE_GCC5/FV/OVMF.fd /output/ovmf.fd && \
    cp /build/ovmf/Build/AmdSev/RELEASE_GCC5/FV/OVMF_CODE.fd /output/ovmf_code.fd 2>/dev/null || true && \
    cp /build/ovmf/Build/AmdSev/RELEASE_GCC5/FV/OVMF_VARS.fd /output/ovmf_vars.fd 2>/dev/null || true

# Record build info for reproducibility
RUN cd /build/ovmf && \
    echo "OVMF_COMMIT=$(git rev-parse HEAD)" > /output/build-info.txt && \
    echo "OVMF_BRANCH=${OVMF_BRANCH}" >> /output/build-info.txt && \
    echo "BUILD_DATE=$(date -u +%Y-%m-%dT%H:%M:%SZ)" >> /output/build-info.txt && \
    echo "DSC=OvmfPkg/AmdSev/AmdSevX64.dsc" >> /output/build-info.txt

# =============================================================================
# Stage 3: Extract packages and prepare components
# =============================================================================
FROM ubuntu:24.04@sha256:c35e29c9450151419d9448b0fd75374fec4fff364a27f176fb458d472dfc9e54 AS component-builder

ARG SOURCE_DATE_EPOCH
ENV SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH} \
    DEBIAN_FRONTEND=noninteractive \
    TZ=UTC \
    LANG=C.UTF-8

RUN apt-get update && apt-get install -y --no-install-recommends \
    dpkg \
    cpio \
    gzip \
    findutils \
    && rm -rf /var/lib/apt/lists/*

COPY --from=package-fetcher /packages /packages
COPY --from=ovmf-builder /output /ovmf-output

WORKDIR /extracted

# Extract kernel
RUN dpkg-deb -x /packages/linux-image-unsigned-*.deb /extracted && \
    dpkg-deb -x /packages/linux-image-generic_*.deb /extracted

# Extract busybox
RUN dpkg-deb -x /packages/busybox-static_*.deb /extracted

# Copy Katana binary
COPY katana-binary /katana-binary
RUN chmod +x /katana-binary

# Organize components
RUN mkdir -p /components && \
    cp /ovmf-output/ovmf.fd /components/ovmf.fd && \
    cp /extracted/boot/vmlinuz-* /components/vmlinuz && \
    cp /extracted/usr/bin/busybox /components/busybox && \
    cp /katana-binary /components/katana && \
    cp /ovmf-output/build-info.txt /components/

# =============================================================================
# Stage 4: Build initrd
# =============================================================================
FROM component-builder AS initrd-builder

COPY tee/scripts/create-initrd.sh /scripts/create-initrd.sh
RUN chmod +x /scripts/create-initrd.sh

WORKDIR /build
RUN /scripts/create-initrd.sh /components/katana /build/initrd.img && \
    cp /build/initrd.img /components/initrd.img

# =============================================================================
# Stage 5: Final output
# =============================================================================
FROM scratch AS final

COPY --from=initrd-builder /components/vmlinuz /output/vmlinuz
COPY --from=initrd-builder /components/ovmf.fd /output/ovmf.fd
COPY --from=initrd-builder /components/initrd.img /output/initrd.img
COPY --from=initrd-builder /components/build-info.txt /output/build-info.txt
