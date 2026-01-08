# VM Image Dockerfile for Katana TEE
#
# Creates reproducible boot components for Katana in AMD SEV-SNP TEEs using direct kernel boot.
# Direct kernel boot ensures the kernel, initrd (containing Katana), and cmdline are included
# in the SEV-SNP launch measurement, preventing binary replacement attacks.
#
# Produces: vmlinuz, initrd.img, ovmf.fd
#
# Usage:
#   docker build -f vm-image.Dockerfile \
#     --build-arg SOURCE_DATE_EPOCH=$(git log -1 --format=%ct) \
#     --build-arg KATANA_BINARY=./katana \
#     -t katana-vm-image .
#
# Extract artifacts:
#   docker create --name vm katana-vm-image
#   docker cp vm:/output/vmlinuz ./vmlinuz
#   docker cp vm:/output/initrd.img ./initrd.img
#   docker cp vm:/output/ovmf.fd ./ovmf.fd
#   docker rm vm
#
# Boot with QEMU (direct kernel boot):
#   qemu-system-x86_64 -m 4G -smp 4 \
#     -bios ovmf.fd \
#     -kernel vmlinuz \
#     -initrd initrd.img \
#     -append "console=ttyS0 katana.args=--http.addr=0.0.0.0" \
#     -cpu EPYC-v4 \
#     -machine q35,confidential-guest-support=sev0 \
#     -object sev-snp-guest,id=sev0,cbitpos=51,reduced-phys-bits=1

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
RUN /scripts/create-initrd.sh /components/katana /build/initrd.img && \
    cp /build/initrd.img /components/initrd.img

# Stage 4: Final output stage
FROM scratch AS final

# Export boot components for direct kernel boot
# These three files contain everything needed for measured boot:
# - ovmf.fd: UEFI firmware (measured)
# - vmlinuz: Linux kernel (measured)
# - initrd.img: Contains Katana binary + minimal init (measured)
COPY --from=initrd-builder /components/vmlinuz /output/vmlinuz
COPY --from=initrd-builder /components/ovmf.fd /output/ovmf.fd
COPY --from=initrd-builder /components/initrd.img /output/initrd.img
