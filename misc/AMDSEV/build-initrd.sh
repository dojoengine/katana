#!/bin/bash
# ==============================================================================
# BUILD-INITRD.SH
# ==============================================================================
#
# This script downloads all required dependencies and builds a minimal initrd
# for running Katana inside AMD SEV-SNP confidential VMs.
#
# Dependencies downloaded:
#   - busybox-static: Provides shell and basic utilities
#   - linux-modules:       Contains dm-crypt.ko and dm-integrity.ko for LUKS-based
#                          sealed storage. (dm-mod is built into the Ubuntu 6.8
#                          kernel — see modules.builtin — so we don't ship it.)
#   - linux-modules-extra: Contains SEV-SNP kernel modules (tsm.ko, sev-guest.ko)
#   - cryptsetup (source): Built statically inside a pinned Alpine container
#                          for LUKS2 sealed-storage unlock in the measured initrd
#
# Usage:
#   ./build-initrd.sh KATANA_BINARY OUTPUT_INITRD [KERNEL_VERSION]
#
# Environment:
#   SOURCE_DATE_EPOCH                REQUIRED. Unix timestamp for reproducible builds.
#   BUSYBOX_PKG_VERSION              REQUIRED. Exact apt package version (e.g., 1:1.36.1-6ubuntu3.1).
#   BUSYBOX_PKG_SHA256               REQUIRED. SHA256 checksum of the busybox .deb package.
#   KERNEL_MODULES_EXTRA_PKG_VERSION REQUIRED. Exact apt package version.
#   KERNEL_MODULES_EXTRA_PKG_SHA256  REQUIRED. SHA256 checksum of the modules .deb package.
#
# ==============================================================================

set -euo pipefail
# File modes are part of the cpio archive and therefore the SEV-SNP launch measurement.
umask 022

REQUIRED_APPLETS=(sh mount umount sleep kill cat mkdir ln mknod ip insmod poweroff sync \
                   tr grep rm mkfifo)
SYMLINK_APPLETS=(sh mount umount mkdir mknod switch_root ip insmod sleep kill cat ln poweroff sync \
                  tr grep rm mkfifo)
# `mkfs.ext2` is not a busybox-static applet on Ubuntu, so a static binary is
# built from e2fsprogs source inside the cryptsetup builder container and
# installed as `/bin/mkfs.ext2`. `blkid` is similarly absent from busybox-
# static; the init avoids it via a try-mount-then-mkfs fallback.

usage() {
    echo "Usage: $0 KATANA_BINARY OUTPUT_INITRD [KERNEL_VERSION]"
    echo ""
    echo "Self-contained initrd builder for Katana TEE VM with AMD SEV-SNP support."
    echo "Downloads all required dependencies (busybox, kernel modules) automatically."
    echo ""
    echo "ARGUMENTS:"
    echo "  KATANA_BINARY    Path to the katana binary (statically linked recommended)"
    echo "  OUTPUT_INITRD    Output path for the generated initrd.img"
    echo "  KERNEL_VERSION   Kernel version for module lookup (or set KERNEL_VERSION env var)"
    echo ""
    echo "ENVIRONMENT VARIABLES (all required for the canonical sealed build):"
    echo "  SOURCE_DATE_EPOCH                Unix timestamp for reproducible builds"
    echo "  BUSYBOX_PKG_VERSION              Exact apt package version (e.g., 1:1.36.1-6ubuntu3.1)"
    echo "  BUSYBOX_PKG_SHA256               SHA256 checksum of the busybox .deb package"
    echo "  KERNEL_MODULES_PKG_VERSION       Exact apt package version for linux-modules"
    echo "  KERNEL_MODULES_PKG_SHA256        SHA256 checksum of the linux-modules .deb"
    echo "  KERNEL_MODULES_EXTRA_PKG_VERSION Exact apt package version for linux-modules-extra"
    echo "  KERNEL_MODULES_EXTRA_PKG_SHA256  SHA256 checksum of the linux-modules-extra .deb"
    echo "  CRYPTSETUP_VERSION               Exact cryptsetup source release (e.g., 2.7.5)"
    echo "  CRYPTSETUP_SHA256                SHA256 checksum of the cryptsetup source tarball"
    echo "  LVM2_VERSION                     Exact LVM2 source release (e.g., 2.03.23)"
    echo "                                   used to build static libdevmapper.a"
    echo "  LVM2_SHA256                      SHA256 checksum of the LVM2 source tarball"
    echo "  E2FSPROGS_VERSION                Exact e2fsprogs source release (e.g., 1.47.0)"
    echo "                                   used to build static mkfs.ext2"
    echo "  E2FSPROGS_SHA256                 SHA256 checksum of the e2fsprogs source tarball"
    echo "  CRYPTSETUP_BUILDER_IMAGE         Pinned container image digest used to build"
    echo "                                   cryptsetup statically (e.g., alpine@sha256:...)"
    echo ""
    echo "All of these have canonical defaults in misc/AMDSEV/build-config; the"
    echo "expected invocation is to source that file and run this script."
    echo ""
    echo "OPTIONAL ENVIRONMENT VARIABLES:"
    echo "  KATANA_UNSEALED_BUILD            Set to 1 to opt OUT of the sealed build."
    echo "                                   Produces an unsealed-only initrd that mounts"
    echo "                                   /dev/sda as plain ext4: no cryptsetup, no"
    echo "                                   dm-* modules, no snp-derivekey. Used by CI"
    echo "                                   on hosts without Docker and for cheap dev"
    echo "                                   iteration."
    echo "  CRYPTSETUP_BUILDER               Container runtime to use (default: docker;"
    echo "                                   can be set to podman or another compatible CLI)"
    echo "  SNP_DERIVEKEY_BINARY             Path to a pre-built static snp-derivekey"
    echo "                                   binary. Required for sealed-mode boot to work"
    echo "                                   at runtime; if absent, the initrd builds but"
    echo "                                   sealed boot will fatal_boot at first cryptsetup"
    echo "                                   call. Build with:"
    echo "                                     cargo build -p katana-tee --features snp \\"
    echo "                                                 --bin snp-derivekey --release \\"
    echo "                                                 --target x86_64-unknown-linux-musl"
    echo ""
    echo "EXAMPLES:"
    echo "  export SOURCE_DATE_EPOCH=\$(date +%s)"
    echo "  export BUSYBOX_PKG_VERSION='1:1.36.1-6ubuntu3.1'"
    echo "  export BUSYBOX_PKG_SHA256='abc123...'"
    echo "  export KERNEL_MODULES_PKG_VERSION='6.8.0-90.99'"
    echo "  export KERNEL_MODULES_PKG_SHA256='aaa111...'"
    echo "  export KERNEL_MODULES_EXTRA_PKG_VERSION='6.8.0-90.99'"
    echo "  export KERNEL_MODULES_EXTRA_PKG_SHA256='def456...'"
    echo "  export CRYPTSETUP_VERSION='2.7.5'"
    echo "  export CRYPTSETUP_SHA256='ghi789...'"
    echo "  export CRYPTSETUP_BUILDER_IMAGE='alpine@sha256:jkl012...'"
    echo "  $0 ./katana ./initrd.img 6.8.0-90"
    exit 1
}

log_section() {
    echo ""
    echo "=========================================="
    echo "$*"
    echo "=========================================="
}

log_info() {
    echo "  [INFO] $*"
}

log_ok() {
    echo "  [OK] $*"
}

log_warn() {
    echo "  [WARN] $*"
}

die() {
    echo "ERROR: $*" >&2
    exit 1
}

install_sev_module() {
    local module_name="$1"
    local source_path="$2"
    local destination_path="$3"

    if [[ -f "${source_path}.zst" ]]; then
        zstd -dq "${source_path}.zst" -o "$destination_path"
        log_ok "$module_name installed (decompressed)"
    elif [[ -f "$source_path" ]]; then
        cp "$source_path" "$destination_path"
        log_ok "$module_name installed"
    else
        die "$module_name not found in linux-modules-extra package"
    fi
}

# Show help if requested or insufficient arguments
if [[ $# -lt 2 ]] || [[ "${1:-}" == "-h" ]] || [[ "${1:-}" == "--help" ]]; then
    usage
fi

to_abs_path() {
    local path="$1"
    if [[ "$path" = /* ]]; then
        printf '%s\n' "$path"
    else
        printf '%s/%s\n' "$(pwd -P)" "$path"
    fi
}

KATANA_BINARY="$(to_abs_path "$1")"
OUTPUT_INITRD="$(to_abs_path "$2")"
KERNEL_VERSION="${3:-${KERNEL_VERSION:?KERNEL_VERSION must be set or passed as third argument}}"
OUTPUT_DIR="$(dirname "$OUTPUT_INITRD")"

log_section "Building Initrd"
echo "Configuration:"
echo "  Katana binary:         $KATANA_BINARY"
echo "  Output initrd:         $OUTPUT_INITRD"
echo "  Kernel version:        $KERNEL_VERSION"
echo "  SOURCE_DATE_EPOCH:     ${SOURCE_DATE_EPOCH:-<not set>}"
echo ""
echo "Package versions:"
echo "  busybox-static:        ${BUSYBOX_PKG_VERSION:-<not set>}"
echo "  linux-modules:         ${KERNEL_MODULES_PKG_VERSION:-<not set>}"
echo "  linux-modules-extra:   ${KERNEL_MODULES_EXTRA_PKG_VERSION:-<not set>}"
echo "  cryptsetup (source):   ${CRYPTSETUP_VERSION:-<not set>}"
echo "  LVM2 (source):         ${LVM2_VERSION:-<not set>}"
echo "  cryptsetup builder:    ${CRYPTSETUP_BUILDER_IMAGE:-<not set>}"

if [[ -z "${SOURCE_DATE_EPOCH:-}" ]]; then
    die "SOURCE_DATE_EPOCH must be set for reproducible builds"
fi

if [[ ! -f "$KATANA_BINARY" ]]; then
    die "Katana binary not found: $KATANA_BINARY"
fi
if [[ ! -x "$KATANA_BINARY" ]]; then
    die "Katana binary is not executable: $KATANA_BINARY"
fi

if [[ ! -d "$OUTPUT_DIR" ]]; then
    mkdir -p "$OUTPUT_DIR" || die "Cannot create output directory: $OUTPUT_DIR"
fi
if [[ ! -w "$OUTPUT_DIR" ]]; then
    die "Output directory is not writable: $OUTPUT_DIR"
fi

REQUIRED_TOOLS=(apt-get dpkg-deb sha256sum cpio gzip zstd find sort touch du mktemp awk grep tr curl tar)
for tool in "${REQUIRED_TOOLS[@]}"; do
    command -v "$tool" >/dev/null 2>&1 || die "Required tool not found: $tool"
done

# Sealed-storage is the canonical build. The pinned env vars come from
# `misc/AMDSEV/build-config`; sourcing that file is the standard way to invoke
# this script (see `.github/workflows/amdsev-initrd-test.yml`).
#
# Opt out by setting `KATANA_UNSEALED_BUILD=1` in the environment. Used by
# CI on hosts without Docker and for cheap dev-iteration builds. The result
# is an unsealed-only initrd: no cryptsetup, no dm-* modules, no
# snp-derivekey. The init mounts /dev/sda as plain ext4.
#
# Setting some but not all sealed env vars is an error — almost certainly a
# misconfiguration of the operator's build environment rather than an
# intentional partial build.
if [[ "${KATANA_UNSEALED_BUILD:-0}" -eq 1 ]]; then
    SEALED_STORAGE_BUILD=0
else
    SEALED_STORAGE_BUILD=1
    : "${CRYPTSETUP_VERSION:?canonical sealed build requires CRYPTSETUP_VERSION (source build-config or set KATANA_UNSEALED_BUILD=1 to opt out)}"
    : "${CRYPTSETUP_SHA256:?canonical sealed build requires CRYPTSETUP_SHA256}"
    : "${CRYPTSETUP_BUILDER_IMAGE:?canonical sealed build requires CRYPTSETUP_BUILDER_IMAGE}"
    : "${KERNEL_MODULES_PKG_VERSION:?canonical sealed build requires KERNEL_MODULES_PKG_VERSION}"
    : "${KERNEL_MODULES_PKG_SHA256:?canonical sealed build requires KERNEL_MODULES_PKG_SHA256}"
    : "${LVM2_VERSION:?canonical sealed build requires LVM2_VERSION}"
    : "${LVM2_SHA256:?canonical sealed build requires LVM2_SHA256}"
    : "${E2FSPROGS_VERSION:?canonical sealed build requires E2FSPROGS_VERSION}"
    : "${E2FSPROGS_SHA256:?canonical sealed build requires E2FSPROGS_SHA256}"
fi

# Static cryptsetup is built inside a pinned container. Verify the chosen
# runtime is installed now so we fail fast, not after an hour of downloading.
# Only required when sealed-storage build is requested.
if [[ "$SEALED_STORAGE_BUILD" -eq 1 ]]; then
    CRYPTSETUP_BUILDER="${CRYPTSETUP_BUILDER:-docker}"
    command -v "$CRYPTSETUP_BUILDER" >/dev/null 2>&1 \
        || die "Container runtime '$CRYPTSETUP_BUILDER' not found. Install docker/podman or set CRYPTSETUP_BUILDER."
fi

# Path to a pre-built static snp-derivekey binary. The init's unseal flow
# spawns this helper to read 32 bytes of SNP-derived key into the LUKS
# keyfile FIFO; without it the cryptsetup call blocks indefinitely.
# Required for sealed builds. Build with:
#   cargo build -p katana-tee --features snp --bin snp-derivekey \
#       --profile performance --target x86_64-unknown-linux-musl
SNP_DERIVEKEY_BINARY="${SNP_DERIVEKEY_BINARY:-}"
if [[ "$SEALED_STORAGE_BUILD" -eq 1 ]]; then
    [[ -n "$SNP_DERIVEKEY_BINARY" ]] \
        || die "SEALED_STORAGE_BUILD=1 but SNP_DERIVEKEY_BINARY is unset (build via build.sh which auto-builds it, or pass the path explicitly)"
    # Resolve relative paths up front: the install step runs after
    # `cd "$INITRD_DIR"` and a relative path would no longer reach the host
    # binary from there. Same treatment as KATANA_BINARY above.
    SNP_DERIVEKEY_BINARY="$(to_abs_path "$SNP_DERIVEKEY_BINARY")"
    [[ -x "$SNP_DERIVEKEY_BINARY" ]] \
        || die "SNP_DERIVEKEY_BINARY=$SNP_DERIVEKEY_BINARY does not exist or is not executable"
fi

log_ok "Preflight validation complete (sealed-storage build: $([ "$SEALED_STORAGE_BUILD" -eq 1 ] && echo yes || echo no))"

WORK_DIR="$(mktemp -d)"
cleanup() {
    local exit_code=$?
    if [[ -d "$WORK_DIR" ]]; then
        rm -rf "$WORK_DIR"
    fi
    exit "$exit_code"
}
trap cleanup EXIT INT TERM

log_info "Working directory: $WORK_DIR"

# ==============================================================================
# SECTION 1: Download Required Packages
# ==============================================================================

log_section "Download Required Packages"
PACKAGES_DIR="$WORK_DIR/packages"
mkdir -p "$PACKAGES_DIR"

pushd "$PACKAGES_DIR" >/dev/null

: "${BUSYBOX_PKG_VERSION:?BUSYBOX_PKG_VERSION not set - required for reproducible builds}"
: "${KERNEL_MODULES_EXTRA_PKG_VERSION:?KERNEL_MODULES_EXTRA_PKG_VERSION not set - required for reproducible builds}"

log_info "Downloading busybox-static=${BUSYBOX_PKG_VERSION}"
apt-get download "busybox-static=${BUSYBOX_PKG_VERSION}"

if [[ "$SEALED_STORAGE_BUILD" -eq 1 ]]; then
    log_info "Downloading linux-modules-${KERNEL_VERSION}-generic=${KERNEL_MODULES_PKG_VERSION}"
    apt-get download "linux-modules-${KERNEL_VERSION}-generic=${KERNEL_MODULES_PKG_VERSION}"
fi

log_info "Downloading linux-modules-extra-${KERNEL_VERSION}-generic=${KERNEL_MODULES_EXTRA_PKG_VERSION}"
apt-get download "linux-modules-extra-${KERNEL_VERSION}-generic=${KERNEL_MODULES_EXTRA_PKG_VERSION}"

echo ""
echo "Downloaded packages:"
ls -lh *.deb

: "${BUSYBOX_PKG_SHA256:?BUSYBOX_PKG_SHA256 not set - required for reproducible builds}"
: "${KERNEL_MODULES_EXTRA_PKG_SHA256:?KERNEL_MODULES_EXTRA_PKG_SHA256 not set - required for reproducible builds}"

log_info "Verifying busybox-static checksum"
ACTUAL_SHA256="$(sha256sum busybox-static_*.deb | awk '{print $1}')"
if [[ "$ACTUAL_SHA256" != "$BUSYBOX_PKG_SHA256" ]]; then
    die "busybox-static checksum mismatch (expected $BUSYBOX_PKG_SHA256, got $ACTUAL_SHA256)"
fi
log_ok "busybox-static checksum verified"

if [[ "$SEALED_STORAGE_BUILD" -eq 1 ]]; then
    # linux-modules-*.deb and linux-modules-extra-*.deb share the `linux-modules-*`
    # filename prefix, so the glob has to be tight enough to pick exactly one file.
    log_info "Verifying linux-modules checksum"
    MODULES_DEB="$(ls linux-modules-"${KERNEL_VERSION}"-generic_*.deb)"
    ACTUAL_SHA256="$(sha256sum "$MODULES_DEB" | awk '{print $1}')"
    if [[ "$ACTUAL_SHA256" != "$KERNEL_MODULES_PKG_SHA256" ]]; then
        die "linux-modules checksum mismatch (expected $KERNEL_MODULES_PKG_SHA256, got $ACTUAL_SHA256)"
    fi
    log_ok "linux-modules checksum verified"
fi

log_info "Verifying linux-modules-extra checksum"
ACTUAL_SHA256="$(sha256sum linux-modules-extra-*.deb | awk '{print $1}')"
if [[ "$ACTUAL_SHA256" != "$KERNEL_MODULES_EXTRA_PKG_SHA256" ]]; then
    die "linux-modules-extra checksum mismatch (expected $KERNEL_MODULES_EXTRA_PKG_SHA256, got $ACTUAL_SHA256)"
fi
log_ok "linux-modules-extra checksum verified"

popd >/dev/null

# ==============================================================================
# SECTION 2: Extract Packages
# ==============================================================================

log_section "Extract Packages"
EXTRACTED_DIR="$WORK_DIR/extracted"
mkdir -p "$EXTRACTED_DIR"

log_info "Extracting busybox-static"
dpkg-deb -x "$PACKAGES_DIR"/busybox-static_*.deb "$EXTRACTED_DIR"

if [[ "$SEALED_STORAGE_BUILD" -eq 1 ]]; then
    log_info "Extracting linux-modules"
    dpkg-deb -x "$PACKAGES_DIR"/linux-modules-"${KERNEL_VERSION}"-generic_*.deb "$EXTRACTED_DIR"
fi

log_info "Extracting linux-modules-extra"
dpkg-deb -x "$PACKAGES_DIR"/linux-modules-extra-*.deb "$EXTRACTED_DIR"
log_ok "Packages extracted"

# ==============================================================================
# SECTION 3: Build Static cryptsetup
# ==============================================================================
# Build a statically-linked cryptsetup binary inside a pinned Alpine
# container. Alpine's musl + `*-static` packages yield a single binary with
# no runtime library dependencies, which is what we need inside the initrd.
#
# The container image is pinned by sha256 digest — same reproducibility bar
# as the apt packages above. The cryptsetup source tarball is pinned by
# version + SHA256.
#
# Crypto backend: OpenSSL (openssl-libs-static). LUKS2 argon2id KDF uses
# libargon2 (also static). Optional features (asciidoc docs, ssh token
# plugin, external tokens, i18n) are disabled — not needed for our narrow
# open/format/close use in the initrd.
#
# The output binary ends up at $WORK_DIR/cryptsetup/cryptsetup-static and is
# copied into the initrd by the "Install Static cryptsetup" subsection in
# the Initrd Structure step below.

if [[ "$SEALED_STORAGE_BUILD" -ne 1 ]]; then
    log_section "Build Static cryptsetup (SKIPPED — unsealed-only build)"
else
log_section "Build Static cryptsetup"

CRYPTSETUP_DIR="$WORK_DIR/cryptsetup"
mkdir -p "$CRYPTSETUP_DIR"
pushd "$CRYPTSETUP_DIR" >/dev/null

# kernel.org tarball URLs are organised under major.minor (e.g. v2.7).
CRYPTSETUP_MAJOR_MINOR="$(printf '%s' "$CRYPTSETUP_VERSION" | awk -F. '{print $1"."$2}')"
CRYPTSETUP_URL="https://www.kernel.org/pub/linux/utils/cryptsetup/v${CRYPTSETUP_MAJOR_MINOR}/cryptsetup-${CRYPTSETUP_VERSION}.tar.xz"
CRYPTSETUP_TARBALL="cryptsetup-${CRYPTSETUP_VERSION}.tar.xz"

log_info "Downloading $CRYPTSETUP_URL"
curl -fLsS -o "$CRYPTSETUP_TARBALL" "$CRYPTSETUP_URL"

log_info "Verifying cryptsetup source checksum"
ACTUAL_SHA256="$(sha256sum "$CRYPTSETUP_TARBALL" | awk '{print $1}')"
if [[ "$ACTUAL_SHA256" != "$CRYPTSETUP_SHA256" ]]; then
    die "cryptsetup source checksum mismatch (expected $CRYPTSETUP_SHA256, got $ACTUAL_SHA256)"
fi
log_ok "cryptsetup source checksum verified"

log_info "Extracting cryptsetup source"
tar -xf "$CRYPTSETUP_TARBALL"

LVM2_TARBALL="LVM2.${LVM2_VERSION}.tgz"
LVM2_URL="https://mirrors.kernel.org/sourceware/lvm2/${LVM2_TARBALL}"
log_info "Downloading $LVM2_URL"
curl -fLsS -o "$LVM2_TARBALL" "$LVM2_URL"

log_info "Verifying LVM2 source checksum"
ACTUAL_SHA256="$(sha256sum "$LVM2_TARBALL" | awk '{print $1}')"
if [[ "$ACTUAL_SHA256" != "$LVM2_SHA256" ]]; then
    die "LVM2 source checksum mismatch (expected $LVM2_SHA256, got $ACTUAL_SHA256)"
fi
log_ok "LVM2 source checksum verified"

log_info "Extracting LVM2 source"
tar -xzf "$LVM2_TARBALL"

E2FSPROGS_TARBALL="e2fsprogs-${E2FSPROGS_VERSION}.tar.xz"
E2FSPROGS_MAJOR_MINOR="$(printf '%s' "$E2FSPROGS_VERSION" | awk -F. '{print $1"."$2}')"
E2FSPROGS_URL="https://mirrors.kernel.org/pub/linux/kernel/people/tytso/e2fsprogs/v${E2FSPROGS_VERSION}/${E2FSPROGS_TARBALL}"
log_info "Downloading $E2FSPROGS_URL"
curl -fLsS -o "$E2FSPROGS_TARBALL" "$E2FSPROGS_URL"

log_info "Verifying e2fsprogs source checksum"
ACTUAL_SHA256="$(sha256sum "$E2FSPROGS_TARBALL" | awk '{print $1}')"
if [[ "$ACTUAL_SHA256" != "$E2FSPROGS_SHA256" ]]; then
    die "e2fsprogs source checksum mismatch (expected $E2FSPROGS_SHA256, got $ACTUAL_SHA256)"
fi
log_ok "e2fsprogs source checksum verified"

log_info "Extracting e2fsprogs source"
tar -xf "$E2FSPROGS_TARBALL"

log_info "Building statically inside $CRYPTSETUP_BUILDER_IMAGE"
# Three-stage build inside the container:
#
#   Stage 1: build libdevmapper.a from LVM2 source. Alpine 3.20 ships only a
#   shared libdevmapper.so; cryptsetup needs the static .a to link with
#   `-all-static`. We build LVM2's device-mapper subset and install
#   /usr/lib/libdevmapper.a + /usr/include/libdevmapper.h.
#
#   Stage 2: cryptsetup configure + make against that newly-installed .a.
#   Output binary is at the source root (cryptsetup 2.x layout, NOT src/).
#
#   Stage 3: build static mke2fs from e2fsprogs source. Ubuntu's busybox-
#   static does not include `mkfs.ext2`, and Alpine's e2fsprogs-static
#   ships only static libraries (no binaries). The init's unseal flow
#   needs mkfs.ext2 to format the decrypted mapper on first boot.
#
# The container runs as root (apk add requires it). Once the build is done,
# chown the output binaries to the invoking host user so subsequent host-side
# steps — including the trap's rm -rf "$WORK_DIR" — don't trip over root-
# owned files. SOURCE_DATE_EPOCH is forwarded so any timestamps embedded in
# the binary match the host's reproducibility anchor.
#
# `bash` is required because cryptsetup's tests/generate-symbols-list (run
# during `make all`) has a `#!/bin/bash` shebang and Alpine's busybox sh is
# not bash.
HOST_UID="$(id -u)"
HOST_GID="$(id -g)"
"$CRYPTSETUP_BUILDER" run --rm \
    -v "$CRYPTSETUP_DIR:/build" \
    -w "/build" \
    -e "SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH}" \
    -e "HOST_UID=${HOST_UID}" \
    -e "HOST_GID=${HOST_GID}" \
    -e "CRYPTSETUP_VERSION=${CRYPTSETUP_VERSION}" \
    -e "LVM2_VERSION=${LVM2_VERSION}" \
    -e "E2FSPROGS_VERSION=${E2FSPROGS_VERSION}" \
    "$CRYPTSETUP_BUILDER_IMAGE" \
    sh -euc '
        # libblkid.a / libuuid.a are part of util-linux-static (verified
        # against the pinned alpine@sha256:1e42bbe… image — those .a files
        # live at /usr/lib/libblkid.a, /usr/lib/libuuid.a, owned by
        # util-linux-static-2.40.1-r1). There are no separate
        # libblkid-static / libuuid-static packages in Alpine.
        apk add --no-cache \
            bash \
            build-base linux-headers pkgconf \
            openssl-dev openssl-libs-static \
            popt-dev popt-static \
            json-c-dev \
            util-linux-dev util-linux-static \
            argon2-dev argon2-static

        # Stage 1: static libdevmapper.a from LVM2.
        cd "/build/LVM2.${LVM2_VERSION}"
        ./configure \
            --enable-static_link \
            --disable-selinux --disable-readline \
            --disable-udev_sync --disable-udev_rules \
            --disable-blkid_wiping
        make -j"$(nproc)" device-mapper
        cp libdm/ioctl/libdevmapper.a /usr/lib/libdevmapper.a
        cp libdm/libdevmapper.h /usr/include/libdevmapper.h

        # Stage 2: cryptsetup, statically linked.
        cd "/build/cryptsetup-${CRYPTSETUP_VERSION}"
        ./configure \
            --disable-shared \
            --enable-static \
            --with-crypto_backend=openssl \
            --disable-asciidoc \
            --disable-ssh-token \
            --disable-external-tokens \
            --disable-nls
        make -j"$(nproc)" LDFLAGS="-all-static"
        # cryptsetup 2.x lays the binary at the source-tree root, not in src/.
        strip ./cryptsetup
        cp ./cryptsetup /build/cryptsetup-static

        # Stage 3: static mkfs.ext2 from e2fsprogs.
        cd "/build/e2fsprogs-${E2FSPROGS_VERSION}"
        ./configure \
            --enable-static --disable-shared \
            --disable-elf-shlibs --disable-nls --disable-rpath \
            --disable-tdb \
            LDFLAGS="-static"
        make -j"$(nproc)"
        strip ./misc/mke2fs
        cp ./misc/mke2fs /build/mkfs.ext2-static

        chown "${HOST_UID}:${HOST_GID}" /build/cryptsetup-static /build/mkfs.ext2-static
        # Intermediate build artefacts stay root-owned inside /build. The host
        # owns $CRYPTSETUP_DIR itself, so the trap'"'"'s rm -rf can still unlink
        # them; but make the leaf directories writable by the host user so any
        # follow-up inspection (find, ls) does not hit permission errors.
        chown -R "${HOST_UID}:${HOST_GID}" /build
    '

for out in cryptsetup-static mkfs.ext2-static; do
    [[ -x "$CRYPTSETUP_DIR/$out" ]] \
        || die "$out static build did not produce a binary at $CRYPTSETUP_DIR/$out"
done

log_info "Verifying static linkage"
for out in cryptsetup-static mkfs.ext2-static; do
    LDD_OUT="$(ldd "$CRYPTSETUP_DIR/$out" 2>&1 || true)"
    if echo "$LDD_OUT" | grep -qE "not a dynamic executable|statically linked"; then
        log_ok "$out is statically linked"
    else
        log_warn "$out may not be fully static:"
        echo "$LDD_OUT" | sed 's/^/    /'
        die "$out must be statically linked to run in the initrd"
    fi
done

log_info "Normalising timestamps for reproducibility"
touch -d "@${SOURCE_DATE_EPOCH}" \
    "$CRYPTSETUP_DIR/cryptsetup-static" \
    "$CRYPTSETUP_DIR/mkfs.ext2-static"

popd >/dev/null
fi  # SEALED_STORAGE_BUILD

# ==============================================================================
# SECTION 4: Build Initrd Structure
# ==============================================================================

log_section "Build Initrd Structure"
INITRD_DIR="$WORK_DIR/initrd"
mkdir -p "$INITRD_DIR"/{bin,dev,proc,sys,tmp,etc,lib/modules,mnt,run/cryptsetup}

cd "$INITRD_DIR"

# ------------------------------------------------------------------------------
# Install Busybox
# ------------------------------------------------------------------------------
log_info "Installing busybox"
BUSYBOX_BIN="$EXTRACTED_DIR/usr/bin/busybox"
[[ -f "$BUSYBOX_BIN" ]] || die "busybox binary not found in extracted package: $BUSYBOX_BIN"

cp "$BUSYBOX_BIN" bin/busybox
chmod +x bin/busybox

if ! bin/busybox --help >/dev/null 2>&1; then
    die "Copied busybox binary is not functional"
fi

AVAILABLE_APPLETS="$(bin/busybox --list 2>/dev/null || true)"
[[ -n "$AVAILABLE_APPLETS" ]] || die "Could not read busybox applet list"

MISSING_APPLETS=()
for applet in "${REQUIRED_APPLETS[@]}"; do
    if ! echo "$AVAILABLE_APPLETS" | grep -Fqx "$applet"; then
        MISSING_APPLETS+=("$applet")
    fi
done

if [[ ${#MISSING_APPLETS[@]} -gt 0 ]]; then
    die "busybox is missing required applets: ${MISSING_APPLETS[*]}"
fi

for cmd in "${SYMLINK_APPLETS[@]}"; do
    if echo "$AVAILABLE_APPLETS" | grep -Fqx "$cmd"; then
        ln -sf busybox "bin/$cmd"
    fi
done
log_ok "Busybox installed and applets validated"

# ------------------------------------------------------------------------------
# Install Static cryptsetup + mkfs.ext2 (sealed-storage build only)
# ------------------------------------------------------------------------------
# Built in SECTION 3 via a pinned Alpine container. Both binaries are fully
# static, so no .so files need to be vendored alongside them. cryptsetup
# unlocks the LUKS volume; mkfs.ext2 formats the decrypted mapper on first
# boot (Ubuntu's busybox-static does not include the mkfs.ext2 applet).
if [[ "$SEALED_STORAGE_BUILD" -eq 1 ]]; then
    log_info "Installing static cryptsetup"
    [[ -x "$CRYPTSETUP_DIR/cryptsetup-static" ]] \
        || die "cryptsetup-static not found at $CRYPTSETUP_DIR/cryptsetup-static (SECTION 3 did not run?)"
    cp "$CRYPTSETUP_DIR/cryptsetup-static" bin/cryptsetup
    chmod +x bin/cryptsetup
    if ! bin/cryptsetup --version >/dev/null 2>&1; then
        die "Installed cryptsetup binary is not functional"
    fi
    log_ok "cryptsetup installed"

    log_info "Installing static mkfs.ext2"
    [[ -x "$CRYPTSETUP_DIR/mkfs.ext2-static" ]] \
        || die "mkfs.ext2-static not found at $CRYPTSETUP_DIR/mkfs.ext2-static (SECTION 3 did not run?)"
    cp "$CRYPTSETUP_DIR/mkfs.ext2-static" bin/mkfs.ext2
    chmod +x bin/mkfs.ext2
    log_ok "mkfs.ext2 installed"
fi

# ------------------------------------------------------------------------------
# Install snp-derivekey (sealed-storage build only)
# ------------------------------------------------------------------------------
# The preflight already hard-failed if the binary was missing; install it.
if [[ "$SEALED_STORAGE_BUILD" -eq 1 ]]; then
    log_info "Installing snp-derivekey"
    cp "$SNP_DERIVEKEY_BINARY" bin/snp-derivekey
    chmod +x bin/snp-derivekey
    log_ok "snp-derivekey installed"
fi

# ------------------------------------------------------------------------------
# Install SEV-SNP Kernel Modules
# ------------------------------------------------------------------------------
log_info "Installing SEV-SNP kernel modules"
MODULES_DIR="$EXTRACTED_DIR/lib/modules/$KERNEL_VERSION-generic/kernel/drivers/virt/coco"

if [[ -d "$MODULES_DIR" ]]; then
    install_sev_module "tsm.ko" "$MODULES_DIR/tsm.ko" "lib/modules/tsm.ko"
    install_sev_module "sev-guest.ko" "$MODULES_DIR/sev-guest/sev-guest.ko" "lib/modules/sev-guest.ko"
else
    die "Modules directory not found: $MODULES_DIR"
fi

# ------------------------------------------------------------------------------
# Install Device-Mapper + dm-integrity transitive deps (sealed-storage only)
# ------------------------------------------------------------------------------
# `dm-integrity` depends (per `depmod` against linux-modules-6.8.0-90 amd64)
# on `async_xor`, `async_tx`, `dm-bufio`, and `xor` — all loadable modules
# that live in the linux-modules deb. `dm-crypt` only needs dm-mod, which is
# kernel-builtin in the Ubuntu 6.8 kernel.
#
# Without these deps installed and loaded in dependency order, dm-integrity
# `insmod` fails with `Unknown symbol dm_bufio_*` / `Unknown symbol async_xor`
# and the unseal flow blocks indefinitely on the cryptsetup FIFO.
#
# Hard-fail at build time if any are missing.
if [[ "$SEALED_STORAGE_BUILD" -eq 1 ]]; then
    log_info "Installing device-mapper + transitive deps for dm-integrity"
    KVER_ROOT="$EXTRACTED_DIR/lib/modules/$KERNEL_VERSION-generic"

    if [[ ! -d "$KVER_ROOT" ]]; then
        die "kernel modules root not found at $KVER_ROOT (sealed-storage build requires the linux-modules deb)"
    fi

    # Order here is the order the init script will insmod them. Matches the
    # depmod-resolved chain: leaf deps first, dm-integrity last.
    #
    # The crypto modules (cryptd / crypto_simd / aesni-intel / sha256-ssse3)
    # are required because dm-crypt allocates its skcipher with
    # CRYPTO_ALG_ALLOCATES_MEMORY, which excludes the kernel-builtin
    # `aes_generic` (it allocates memory in the hot path). Without an
    # AESNI-backed `xts(aes)`, dm-crypt fails with `Error allocating crypto
    # tfm (-ENOENT)` at the first luksFormat. Same constraint applies to
    # `hmac(sha256)` for dm-integrity; sha256-ssse3 covers it. AMD EPYC
    # hosts (the only ones that run SEV-SNP) always have AESNI + SSSE3, so
    # these modules always load successfully.
    DM_MODULES=(
        "kernel/crypto/xor.ko"
        "kernel/crypto/async_tx/async_tx.ko"
        "kernel/crypto/async_tx/async_xor.ko"
        "kernel/crypto/cryptd.ko"
        "kernel/crypto/crypto_simd.ko"
        "kernel/arch/x86/crypto/aesni-intel.ko"
        "kernel/arch/x86/crypto/sha256-ssse3.ko"
        "kernel/crypto/authenc.ko"
        "kernel/drivers/md/dm-bufio.ko"
        "kernel/drivers/md/dm-crypt.ko"
        "kernel/drivers/md/dm-integrity.ko"
    )

    for rel in "${DM_MODULES[@]}"; do
        name="$(basename "${rel%.ko}")"
        dest="lib/modules/${name}.ko"
        src_zst="$KVER_ROOT/${rel}.zst"
        src_raw="$KVER_ROOT/$rel"
        if [[ -f "$src_zst" ]]; then
            zstd -dq "$src_zst" -o "$dest"
            log_ok "${name}.ko installed (decompressed)"
        elif [[ -f "$src_raw" ]]; then
            cp "$src_raw" "$dest"
            log_ok "${name}.ko installed"
        else
            die "${name}.ko not found in linux-modules (searched $src_raw and ${src_raw}.zst)"
        fi
    done
fi

# ------------------------------------------------------------------------------
# Install Katana Binary
# ------------------------------------------------------------------------------
log_info "Installing Katana binary"
cp "$KATANA_BINARY" bin/katana
chmod +x bin/katana
log_ok "Katana installed"

# ------------------------------------------------------------------------------
# Create Init Script
# ------------------------------------------------------------------------------
log_info "Creating init script"
cat > init <<'INIT_EOF'
#!/bin/sh
# Katana TEE VM Init Script

set -eu
export PATH=/bin

# log writes to stderr so command substitution like `$(strip_db_args ...)`
# captures only the function's real output. Both stdout and stderr are
# redirected to /dev/console below, so operator UX is unchanged.
log() { echo "[init] $*" >&2; }

KATANA_PID=""
KATANA_DB_DIR="/mnt/data/katana-db"
SHUTTING_DOWN=0
KATANA_EXIT_CODE="never"
CONTROL_PORT_NAME="org.katana.control.0"
CONTROL_PORT_LINK="/dev/virtio-ports/org.katana.control.0"

# Sealed-storage state (populated from /proc/cmdline; see parse_cmdline_vars).
# SEALED_MODE=1 means an encrypted /dev/sda backs /mnt/data and the derived key
# must match; SEALED_MODE=0 keeps the legacy plain-ext4 behaviour.
EXPECTED_LUKS_UUID=""
SEALED_MODE=0
LUKS_MAPPER_NAME="katana-data"
LUKS_MAPPER_DEV="/dev/mapper/${LUKS_MAPPER_NAME}"
LUKS_DEVICE="/dev/sda"
LUKS_OPENED=0

fatal_boot() {
    log "ERROR: $*"
    teardown_and_halt
}

# Unified teardown path, safe to call from any phase.
# Each step is idempotent — tolerates being called before the mount / before
# the LUKS device is opened / before Katana has started. No step is allowed to
# block; every unmount / luksClose runs with timeouts or `|| true` so a stuck
# filesystem cannot prevent the VM from powering off.
teardown_and_halt() {
    if [ "$SHUTTING_DOWN" -eq 1 ]; then
        # Re-entry (e.g. fatal_boot called from within shutdown_handler).
        # Keep spinning; the first caller is already driving the teardown.
        while true; do sleep 1; done
    fi
    SHUTTING_DOWN=1

    # Log the cause if a caller passed one. Without this, the failing
    # subsystem's stderr line (e.g. cryptsetup's "No key available with this
    # passphrase.") is the last thing in the log before the teardown starts,
    # losing the operator-facing diagnostic that names the actual condition.
    if [ "$#" -gt 0 ]; then
        log "FATAL: $*"
    fi

    log "Teardown: stopping katana (if running)..."
    if [ -n "${KATANA_PID:-}" ] && kill -0 "$KATANA_PID" 2>/dev/null; then
        kill -TERM "$KATANA_PID" 2>/dev/null || true
        TIMEOUT=30
        while [ "$TIMEOUT" -gt 0 ] && kill -0 "$KATANA_PID" 2>/dev/null; do
            sleep 1
            TIMEOUT=$((TIMEOUT - 1))
        done
        if kill -0 "$KATANA_PID" 2>/dev/null; then
            log "Teardown: forcing kill of katana"
            kill -KILL "$KATANA_PID" 2>/dev/null || true
        fi
    fi

    log "Teardown: syncing and unmounting..."
    sync || true
    umount /mnt/data 2>/dev/null || true

    if [ "$LUKS_OPENED" -eq 1 ]; then
        log "Teardown: closing LUKS mapper $LUKS_MAPPER_NAME"
        /bin/cryptsetup luksClose "$LUKS_MAPPER_NAME" 2>/dev/null || true
        LUKS_OPENED=0
    fi

    umount /tmp 2>/dev/null || true
    umount /dev 2>/dev/null || true
    umount /sys/kernel/config 2>/dev/null || true
    umount /sys 2>/dev/null || true
    umount /proc 2>/dev/null || true

    log "Teardown: poweroff"
    poweroff -f
    while true; do sleep 1; done
}

# Parse KATANA_EXPECTED_LUKS_UUID out of /proc/cmdline. It is produced by
# start-vm.sh and lives inside the measured kernel command line — any tamper
# changes the launch measurement.
parse_cmdline_vars() {
    if [ ! -r /proc/cmdline ]; then
        log "WARNING: /proc/cmdline not readable; sealed-storage vars unset"
        return 0
    fi
    # Space-separated `key=value` tokens; iterate and pick out the one we care
    # about. We deliberately avoid `grep -oP` (GNU-only) to stay portable
    # against busybox variants.
    for tok in $(cat /proc/cmdline); do
        case "$tok" in
            KATANA_EXPECTED_LUKS_UUID=*)
                EXPECTED_LUKS_UUID="${tok#KATANA_EXPECTED_LUKS_UUID=}"
                ;;
        esac
    done
    if [ -n "$EXPECTED_LUKS_UUID" ]; then
        SEALED_MODE=1
        log "Sealed mode enabled: EXPECTED_LUKS_UUID=$EXPECTED_LUKS_UUID"
    else
        log "Sealed mode NOT enabled (KATANA_EXPECTED_LUKS_UUID unset)"
    fi
}

# Load device-mapper + dm-integrity transitive deps in dependency order.
# `dm-mod` is built into the Ubuntu 6.8 kernel (see modules.builtin); the
# rest are loadable modules shipped in the initrd. Order matters:
#
#   xor, async_tx → async_xor → dm-bufio → dm-crypt → dm-integrity
#
# Hard-fails on any insmod error so a misconfigured initrd surfaces here
# rather than wedging cryptsetup later. Only invoked from the sealed-mode
# branch — an unsealed initrd doesn't ship these modules and never reaches
# this function.
load_dm_modules() {
    # Order matches the build-initrd module-install list. cryptd /
    # crypto_simd / aesni-intel must come before dm-crypt because the
    # AESNI-backed xts(aes) is the only `aes` impl that satisfies
    # dm-crypt's CRYPTO_ALG_ALLOCATES_MEMORY constraint. sha256-ssse3
    # covers the same constraint for dm-integrity's hmac(sha256).
    for mod in xor async_tx async_xor cryptd crypto_simd aesni-intel sha256-ssse3 authenc dm-bufio dm-crypt dm-integrity; do
        if [ ! -f "/lib/modules/${mod}.ko" ]; then
            teardown_and_halt "load_dm_modules: /lib/modules/${mod}.ko missing"
        fi
        if ! /bin/insmod "/lib/modules/${mod}.ko"; then
            teardown_and_halt "load_dm_modules: insmod ${mod}.ko failed"
        fi
        log "Loaded ${mod}.ko"
    done
}

# Drop any --data-dir / --db-dir / --db-* flags from a space-separated arg
# string. This prevents an operator with access to the virtio-serial control
# channel from pointing Katana at a data directory outside the sealed mount,
# escaping the sealing guarantee. Runs inside the measured initrd, so the
# defense itself is pinned.
#
# `--data-dir` is the canonical katana flag; `--db-dir` is its alias (see
# crates/cli/src/options.rs). Both must be filtered. The `--db-*` wildcard
# additionally catches any future db-namespaced flags. We do not use a
# `--data-*` wildcard because unrelated future flags could legitimately
# start with `--data-`.
#
# Handles:  `--data-dir value`     (two tokens — next token consumed)
#           `--data-dir=value`     (one token)
#           `--db-dir value`       (alias, two tokens)
#           `--db-dir=value`       (alias, one token)
#           `--db-anything[=val]`  (catch-all for future --db-* flags)
strip_db_args() {
    SKIP_NEXT=0
    OUT=""
    for tok in $*; do
        if [ "$SKIP_NEXT" -eq 1 ]; then
            SKIP_NEXT=0
            log "strip_db_args: dropped value token '$tok'"
            continue
        fi
        case "$tok" in
            --data-dir=*|--db-*=*)
                log "strip_db_args: dropped '$tok'"
                continue
                ;;
            --data-dir|--db-*)
                log "strip_db_args: dropped '$tok' (and next token)"
                SKIP_NEXT=1
                continue
                ;;
        esac
        OUT="${OUT}${OUT:+ }${tok}"
    done
    echo "$OUT"
}

# Sealed-storage unlock. Called only when SEALED_MODE=1.
#
# Flow:
#   1. require /dev/sev-guest (else fatal)
#   2. if /dev/sda has no LUKS header, luksFormat with the expected UUID
#      (first-boot / post-wipe auto-provisioning — same measurement as
#      subsequent boots, so the derived key matches)
#   3. require the header's UUID to match the expected UUID
#      (else fatal: different disk mounted)
#   4. luksOpen (else fatal: chip or measurement drift — derived key
#      differs from what sealed the header)
#   5. if the decrypted mapper has no filesystem, mkfs.ext2
#   6. mount /dev/mapper/<name> at /mnt/data
#
# On first boot or after a hostile header-wipe, steps 2 and 5 both fire and
# the disk is recreated from scratch. An attacker who wipes the header does
# not gain state-substitution (they cannot produce blocks the verifier
# accepts unless the verifier independently pins the chain anchor); they
# only downgrade the chain to a fresh start. See docs/amdsev.md trust model.
#
# Note: busybox ships mkfs.ext2 but not mkfs.ext4. MDBX is indifferent; we
# accept ext2 for the sealed mount. Upgrading to a statically-built mke2fs
# later is an isolated follow-up.
KEY_FIFO="/tmp/katana-luks.key"

_unseal_spawn_key_writer() {
    rm -f "$KEY_FIFO"
    mkfifo -m 0600 "$KEY_FIFO" || teardown_and_halt "unseal: mkfifo failed"
    /bin/snp-derivekey > "$KEY_FIFO" &
    KEY_PID=$!
}

_unseal_wait_key_writer() {
    wait "$KEY_PID" 2>/dev/null || true
    KEY_PID=""
    rm -f "$KEY_FIFO"
}

unseal_and_mount() {
    [ -c /dev/sev-guest ] || teardown_and_halt "sealed mode requires /dev/sev-guest"
    [ -b "$LUKS_DEVICE" ] || teardown_and_halt "sealed mode: $LUKS_DEVICE not found"

    # cryptsetup needs /run/cryptsetup for its lock file. The initrd build
    # creates this directory, but `mkdir -p` defensively in case we ever boot
    # an older initrd (or someone reuses unseal_and_mount in another context).
    mkdir -p /run/cryptsetup

    HAS_LUKS_HEADER=0
    if /bin/cryptsetup isLuks "$LUKS_DEVICE" 2>/dev/null; then
        HAS_LUKS_HEADER=1
    fi

    # First boot / post-wipe: format the blank disk with the expected UUID
    # under the current measurement's derived key. Subsequent boots with the
    # same measurement re-derive the same key and open cleanly.
    if [ "$HAS_LUKS_HEADER" -eq 0 ]; then
        log "No LUKS header on $LUKS_DEVICE; formatting with UUID=$EXPECTED_LUKS_UUID"
        _unseal_spawn_key_writer
        if ! /bin/cryptsetup --batch-mode \
                --type luks2 \
                --cipher aes-xts-plain64 --key-size 512 \
                --hash sha256 \
                --uuid "$EXPECTED_LUKS_UUID" \
                --integrity hmac-sha256 \
                --pbkdf pbkdf2 --pbkdf-force-iterations 1000 \
                --key-file "$KEY_FIFO" \
                luksFormat "$LUKS_DEVICE"; then
            _unseal_wait_key_writer
            teardown_and_halt "luksFormat failed"
        fi
        _unseal_wait_key_writer
    fi

    # Enforce header UUID matches the measured expectation.
    DISK_UUID="$(/bin/cryptsetup luksUUID "$LUKS_DEVICE" 2>/dev/null || true)"
    if [ "$DISK_UUID" != "$EXPECTED_LUKS_UUID" ]; then
        teardown_and_halt "sealed mode: disk UUID '$DISK_UUID' does not match expected '$EXPECTED_LUKS_UUID' (disk swapped?)"
    fi

    # Open. A failure here almost always means the derived key differs from
    # what the disk was sealed with — i.e. different chip, or the measured
    # image changed between sealing and now.
    _unseal_spawn_key_writer
    if ! /bin/cryptsetup --key-file "$KEY_FIFO" luksOpen "$LUKS_DEVICE" "$LUKS_MAPPER_NAME"; then
        _unseal_wait_key_writer
        teardown_and_halt "luksOpen failed — chip mismatch or measurement drift"
    fi
    _unseal_wait_key_writer
    LUKS_OPENED=1

    # Filesystem on the decrypted mapper. Try-mount first; if that fails the
    # mapper is empty (fresh luksFormat above, or a prior boot crashed before
    # mkfs ran) — format and re-try. Avoids a dependency on `blkid`, which is
    # not in Ubuntu's busybox-static.
    if ! /bin/mount -t ext2 "$LUKS_MAPPER_DEV" /mnt/data 2>/dev/null; then
        log "Mount failed; assuming empty mapper. Creating ext2 filesystem on $LUKS_MAPPER_DEV"
        /bin/mkfs.ext2 -F "$LUKS_MAPPER_DEV" >/dev/null 2>&1 \
            || teardown_and_halt "mkfs on decrypted volume failed"
        /bin/mount -t ext2 "$LUKS_MAPPER_DEV" /mnt/data \
            || teardown_and_halt "failed to mount $LUKS_MAPPER_DEV after mkfs"
    fi
    log "Sealed storage mounted at /mnt/data"
}

load_sev_module() {
    MODULE_PATH="$1"
    MODULE_NAME="${MODULE_PATH##*/}"

    if [ ! -f "$MODULE_PATH" ]; then
        log "WARNING: SEV-SNP module not included in initrd: $MODULE_PATH"
        return 1
    fi

    if MODULE_ERROR="$(/bin/insmod "$MODULE_PATH" 2>&1)"; then
        log "Loaded $MODULE_NAME"
        return 0
    fi

    log "WARNING: failed to load $MODULE_NAME"
    if [ -n "$MODULE_ERROR" ]; then
        printf '%s\n' "$MODULE_ERROR" | while IFS= read -r line; do
            [ -n "$line" ] && log "insmod $MODULE_NAME: $line"
        done
    fi
    return 1
}

refresh_katana_state() {
    if [ -n "$KATANA_PID" ] && ! kill -0 "$KATANA_PID" 2>/dev/null; then
        if wait "$KATANA_PID"; then
            KATANA_EXIT_CODE=0
        else
            KATANA_EXIT_CODE=$?
        fi
        log "Katana exited with code $KATANA_EXIT_CODE"
        KATANA_PID=""
    fi
}

respond_control() {
    printf '%s\n' "$1" >&3 2>/dev/null || true
}

resolve_control_port() {
    mkdir -p /dev/virtio-ports
    for name_file in /sys/class/virtio-ports/*/name; do
        [ -f "$name_file" ] || continue

        PORT_NAME_VALUE="$(cat "$name_file" 2>/dev/null || true)"
        if [ "$PORT_NAME_VALUE" != "$CONTROL_PORT_NAME" ]; then
            continue
        fi

        PORT_DIR="${name_file%/name}"
        PORT_DEV="/dev/${PORT_DIR##*/}"
        if [ -e "$PORT_DEV" ]; then
            ln -sf "$PORT_DEV" "$CONTROL_PORT_LINK"
            echo "$CONTROL_PORT_LINK"
            return 0
        fi
    done
    return 1
}

handle_control_command() {
    RAW_CMD="$1"
    CMD="${RAW_CMD%% *}"
    CMD_PAYLOAD=""
    if [ "$CMD" != "$RAW_CMD" ]; then
        CMD_PAYLOAD="${RAW_CMD#* }"
    fi

    case "$CMD" in
        start)
            refresh_katana_state
            if [ -n "$KATANA_PID" ] && kill -0 "$KATANA_PID" 2>/dev/null; then
                respond_control "err already-running pid=$KATANA_PID"
                return 0
            fi

            KATANA_ARGS=""
            if [ -n "$CMD_PAYLOAD" ]; then
                RAW_ARGS="$(echo "$CMD_PAYLOAD" | tr ',' ' ')"
                # Defense in depth: a later clap flag on the command line
                # would override the --db-dir we bake in below, letting an
                # operator point Katana at a directory outside the sealed
                # mount. Strip any --db-* from attacker-influenced input.
                KATANA_ARGS="$(strip_db_args $RAW_ARGS)"
            fi

            log "Starting katana asynchronously..."
            # shellcheck disable=SC2086
            /bin/katana --db-dir="$KATANA_DB_DIR" $KATANA_ARGS &
            KATANA_PID=$!
            KATANA_EXIT_CODE="running"
            respond_control "ok started pid=$KATANA_PID"
            ;;

        status)
            refresh_katana_state
            if [ -n "$KATANA_PID" ] && kill -0 "$KATANA_PID" 2>/dev/null; then
                respond_control "running pid=$KATANA_PID"
            else
                respond_control "stopped exit=$KATANA_EXIT_CODE"
            fi
            ;;

        "")
            ;;

        *)
            respond_control "err unknown-command"
            ;;
    esac
}

shutdown_handler() {
    log "Received shutdown signal"
    teardown_and_halt
}

trap shutdown_handler TERM INT

# Mount essential filesystems
/bin/mount -t proc proc /proc || log "WARNING: failed to mount /proc"
/bin/mount -t sysfs sysfs /sys || log "WARNING: failed to mount /sys"

# Mount /dev
if ! /bin/mount -t devtmpfs devtmpfs /dev 2>/dev/null; then
    /bin/mount -t tmpfs tmpfs /dev || log "WARNING: failed to mount /dev"
fi
/bin/mount -t tmpfs tmpfs /tmp 2>/dev/null || true

# Create essential device nodes
[ -c /dev/null ] || /bin/mknod /dev/null c 1 3 || true
[ -c /dev/console ] || /bin/mknod /dev/console c 5 1 || true
[ -c /dev/tty ] || /bin/mknod /dev/tty c 5 0 || true
[ -c /dev/urandom ] || /bin/mknod /dev/urandom c 1 9 || true

# Route all logs to the serial console
exec 0</dev/console
exec 1>/dev/console
exec 2>/dev/console

# Load SEV-SNP kernel modules
log "Loading SEV-SNP kernel modules..."
load_sev_module /lib/modules/tsm.ko || true
load_sev_module /lib/modules/sev-guest.ko || true
sleep 1

# Check for TEE attestation interfaces
TEE_DEVICE_FOUND=0

mkdir -p /sys/kernel/config
if /bin/mount -t configfs configfs /sys/kernel/config 2>/dev/null; then
    [ -d /sys/kernel/config/tsm/report ] && TEE_DEVICE_FOUND=1 && log "ConfigFS TSM interface available"
fi

if [ -c /dev/sev-guest ]; then
    TEE_DEVICE_FOUND=1
    log "SEV-SNP device available at /dev/sev-guest"
elif [ -f /sys/devices/virtual/misc/sev-guest/dev ]; then
    SEV_DEV="$(cat /sys/devices/virtual/misc/sev-guest/dev)"
    /bin/mknod /dev/sev-guest c "${SEV_DEV%%:*}" "${SEV_DEV##*:}" && TEE_DEVICE_FOUND=1 && log "Created /dev/sev-guest"
fi

for tpm in /dev/tpm0 /dev/tpmrm0; do
    [ -c "$tpm" ] && TEE_DEVICE_FOUND=1 && log "TPM device available at $tpm"
done

if [ "$TEE_DEVICE_FOUND" -eq 0 ]; then
    log "WARNING: No TEE attestation interface found"
fi

# Configure networking (QEMU user-mode defaults)
log "Configuring network..."
/bin/ip link set lo up 2>/dev/null || true
if [ -d /sys/class/net/eth0 ]; then
    /bin/ip link set eth0 up 2>/dev/null || true
    /bin/ip addr add 10.0.2.15/24 dev eth0 2>/dev/null || true
    /bin/ip route add default via 10.0.2.2 2>/dev/null || true
    echo "nameserver 10.0.2.3" > /etc/resolv.conf
    log "Network configured: eth0 = 10.0.2.15"
else
    log "WARNING: eth0 interface not found; skipping static network setup"
fi

# Parse sealed-storage vars out of the measured kernel cmdline.
parse_cmdline_vars

# Attach /dev/sda — either through LUKS (sealed) or directly (legacy).
if [ ! -b /dev/sda ]; then
    fatal_boot "required storage device /dev/sda not found"
fi
log "Found storage device /dev/sda"
mkdir -p /mnt/data

if [ "$SEALED_MODE" -eq 1 ]; then
    # dm-crypt / dm-integrity and the async_xor / dm-bufio chain are only
    # shipped in sealed-mode initrds, so load them only when sealed-mode is
    # active. An unsealed-only initrd never reaches load_dm_modules.
    load_dm_modules
    unseal_and_mount
else
    # Legacy path: plain ext4 on /dev/sda. Kept for backward compat with
    # non-sealed boot flows (CI, dev). A quote produced from this path is
    # signed over state read from an unencrypted disk — the launch
    # measurement is honest, but the operator could have substituted the
    # database between restarts. Verifiers should pin the sealed-mode
    # measurement and reject quotes from this one for production use.
    if ! /bin/mount -t ext4 /dev/sda /mnt/data 2>/dev/null; then
        fatal_boot "failed to mount /dev/sda (unsealed)"
    fi
    log "Unsealed storage mounted at /mnt/data"
fi

mkdir -p "$KATANA_DB_DIR"

# Start async control loop for Katana startup/status commands.
log "Waiting for control channel ($CONTROL_PORT_NAME)..."
CONTROL_PORT=""
while [ -z "$CONTROL_PORT" ]; do
    CONTROL_PORT="$(resolve_control_port || true)"
    [ -n "$CONTROL_PORT" ] || sleep 1
done
log "Control channel ready: $CONTROL_PORT"

while true; do
    refresh_katana_state

    if ! exec 3<>"$CONTROL_PORT"; then
        log "WARNING: failed to open control channel, retrying..."
        sleep 1
        continue
    fi

    while IFS= read -r CONTROL_CMD <&3; do
        handle_control_command "$CONTROL_CMD"
    done

    exec 3>&- 3<&-
    sleep 1
done
INIT_EOF

chmod +x init
log_ok "Init script created"

# ------------------------------------------------------------------------------
# Create Minimal /etc Files
# ------------------------------------------------------------------------------
log_info "Creating /etc files"
echo "root:x:0:0:root:/:/bin/sh" > etc/passwd
echo "root:x:0:" > etc/group
log_ok "/etc files created"

# ==============================================================================
# SECTION 5: Create CPIO Archive
# ==============================================================================

log_section "Create CPIO Archive"
echo "Initrd contents:"
find . \( -type f -o -type l \) | sort
echo ""
echo "Total size before compression:"
du -sh .
echo ""

log_info "Normalizing file modes"
find . -type d -exec chmod 0755 {} +
find . -type f -exec chmod 0644 {} +
chmod 0755 bin/busybox bin/katana init
if [[ "$SEALED_STORAGE_BUILD" -eq 1 ]]; then
    chmod 0755 bin/cryptsetup bin/mkfs.ext2
    [[ -n "$SNP_DERIVEKEY_BINARY" ]] && chmod 0755 bin/snp-derivekey
fi
chmod 1777 tmp

log_info "Setting timestamps to SOURCE_DATE_EPOCH (${SOURCE_DATE_EPOCH})"
find . -exec touch -h -d "@${SOURCE_DATE_EPOCH}" {} +

CPIO_FLAGS=(--create --format=newc --null --owner=0:0 --quiet)
if cpio --help 2>&1 | grep -q -- "--reproducible"; then
    CPIO_FLAGS+=(--reproducible)
else
    log_warn "cpio does not support --reproducible; continuing without it"
fi

log_info "Creating cpio archive"
find . -print0 | LC_ALL=C sort -z | cpio "${CPIO_FLAGS[@]}" | gzip -n > "$OUTPUT_INITRD"
touch -d "@${SOURCE_DATE_EPOCH}" "$OUTPUT_INITRD"

# ==============================================================================
# SECTION 6: Final Validation
# ==============================================================================

log_section "Final Validation"
[[ -f "$OUTPUT_INITRD" ]] || die "Output file was not created: $OUTPUT_INITRD"
[[ -s "$OUTPUT_INITRD" ]] || die "Output file is empty: $OUTPUT_INITRD"
if ! gzip -t "$OUTPUT_INITRD" 2>/dev/null; then
    die "Output file is not valid gzip: $OUTPUT_INITRD"
fi
log_ok "Output artifact validated"

echo ""
echo "=========================================="
echo "[OK] Initrd created successfully!"
echo "=========================================="
echo "Output file: $OUTPUT_INITRD"
echo "Size:        $(du -h "$OUTPUT_INITRD" | cut -f1)"
echo "SHA256:      $(sha256sum "$OUTPUT_INITRD" | cut -d' ' -f1)"
echo "=========================================="
