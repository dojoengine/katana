# VM Image Dockerfile for Katana TEE
#
# Creates a reproducible, bootable VM image containing the Katana binary for SEV-SNP TEEs.
# Produces: disk.raw, vmlinuz, ovmf.fd, initrd.img
#
# Usage:
#   docker build -f vm-image.Dockerfile \
#     --build-arg SOURCE_DATE_EPOCH=$(git log -1 --format=%ct) \
#     --build-context katana-binary=path/to/katana \
#     -t katana-vm-image .
#
# Extract artifacts:
#   docker create --name vm katana-vm-image
#   docker cp vm:/disk.raw ./disk.raw
#   docker cp vm:/vmlinuz ./vmlinuz
#   docker cp vm:/ovmf.fd ./ovmf.fd
#   docker cp vm:/initrd.img ./initrd.img
#   docker rm vm

# Stage 1: Download pinned Ubuntu packages
FROM ubuntu:24.04@sha256:c35e29c9450151419d9448b0fd75374fec4fff364a27f176fb458d472dfc9e54 AS package-fetcher

ARG SOURCE_DATE_EPOCH
ENV SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH} \
    DEBIAN_FRONTEND=noninteractive \
    TZ=UTC \
    LANG=C.UTF-8

RUN test -n "$SOURCE_DATE_EPOCH" || (echo "ERROR: SOURCE_DATE_EPOCH build arg is required" && exit 1)

# Install minimal tools for downloading packages
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    wget \
    && rm -rf /var/lib/apt/lists/*

# Download current available package versions using apt-get
# This ensures we get working versions; versions will be documented in build output
WORKDIR /packages

# Update package lists
RUN apt-get update

# Download packages (using apt-get download gets current versions)
RUN apt-get download linux-image-generic ovmf busybox-static cpio && \
    KERNEL_VER=$(ls linux-image-generic_*.deb | grep -oP '[0-9]+\.[0-9]+\.[0-9]+-[0-9]+' | head -1) && \
    apt-get download linux-image-unsigned-${KERNEL_VER}-generic

# List downloaded packages for documentation
RUN ls -lh *.deb

# Clean up
RUN rm -rf /var/lib/apt/lists/*

# Stage 2: Extract packages and prepare components
FROM ubuntu:24.04@sha256:c35e29c9450151419d9448b0fd75374fec4fff364a27f176fb458d472dfc9e54 AS component-builder

ARG SOURCE_DATE_EPOCH
ENV SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH} \
    DEBIAN_FRONTEND=noninteractive \
    TZ=UTC \
    LANG=C.UTF-8

# Install tools needed for extraction and image building
RUN apt-get update && apt-get install -y --no-install-recommends \
    dpkg \
    cpio \
    gzip \
    findutils \
    && rm -rf /var/lib/apt/lists/*

# Copy downloaded packages
COPY --from=package-fetcher /packages /packages

# Extract packages
WORKDIR /extracted

# Extract OVMF
RUN dpkg-deb -x /packages/ovmf_*.deb /extracted

# Extract kernel packages
RUN dpkg-deb -x /packages/linux-image-unsigned-*.deb /extracted
RUN dpkg-deb -x /packages/linux-image-generic_*.deb /extracted

# Extract busybox-static
RUN dpkg-deb -x /packages/busybox-static_*.deb /extracted

# Copy Katana binary from build context
# For now using regular COPY (build-context requires BuildKit)
COPY katana-binary /katana-binary
RUN chmod +x /katana-binary

# Debug: Show what was extracted
RUN echo "=== Extracted structure ===" && \
    find /extracted -name "*.fd" -o -name "vmlinuz*" -o -name "busybox" | head -20

# Organize extracted files
RUN mkdir -p /components && \
    cp /extracted/usr/share/OVMF/OVMF_CODE_4M.fd /components/ovmf.fd && \
    cp /extracted/boot/vmlinuz-* /components/vmlinuz && \
    cp /extracted/usr/bin/busybox /components/busybox && \
    cp /katana-binary /components/katana

# Stage 3: Build initrd
FROM component-builder AS initrd-builder

# Copy initrd build script
COPY tee/scripts/create-initrd.sh /scripts/create-initrd.sh
RUN chmod +x /scripts/create-initrd.sh

# Build initrd with Katana embedded
WORKDIR /build
RUN /scripts/create-initrd.sh /components/katana /components/initrd.img

# Stage 4: Create VM disk image
FROM ubuntu:24.04@sha256:c35e29c9450151419d9448b0fd75374fec4fff364a27f176fb458d472dfc9e54 AS image-builder

ARG SOURCE_DATE_EPOCH
ENV SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH} \
    DEBIAN_FRONTEND=noninteractive \
    TZ=UTC \
    LANG=C.UTF-8

# Install tools for disk image creation
RUN apt-get update && apt-get install -y --no-install-recommends \
    gdisk \
    dosfstools \
    e2fsprogs \
    util-linux \
    findutils \
    coreutils \
    parted \
    kpartx \
    && rm -rf /var/lib/apt/lists/*

# Copy components from previous stages
COPY --from=initrd-builder /components /components

# Copy build script and config
COPY tee/scripts/build-vm-image.sh /scripts/build-vm-image.sh
COPY tee/configs/cmdline.txt /configs/cmdline.txt
RUN chmod +x /scripts/build-vm-image.sh

# Build VM disk image
# NOTE: This requires loop device access, so must be run with docker run --privileged
# During build, we skip this and just prepare the components
WORKDIR /output

# Copy all components to output
RUN cp /components/vmlinuz /output/vmlinuz && \
    cp /components/ovmf.fd /output/ovmf.fd && \
    cp /components/initrd.img /output/initrd.img

# Entry point for creating disk image (run with --privileged)
# Example: docker run --privileged -v $(pwd)/output:/mnt katana-vm-image
CMD ["/scripts/build-vm-image.sh", "--output", "/mnt/disk.raw", "--kernel", "/output/vmlinuz", "--initrd", "/output/initrd.img", "--cmdline-file", "/configs/cmdline.txt", "--size", "2G"]

# Stage 5: Final image with artifacts (for extraction)
FROM scratch AS final
# Note: disk.raw is created separately with docker run --privileged
COPY --from=image-builder /output/vmlinuz /vmlinuz
COPY --from=image-builder /output/ovmf.fd /ovmf.fd
COPY --from=image-builder /output/initrd.img /initrd.img
