#!/bin/bash
# ==============================================================================
# BUILD-CRYPTSETUP.SH
# ==============================================================================
#
# Build the static `cryptsetup` and `mkfs.ext2` binaries the sealed-storage
# initrd needs, inside a pinned Alpine container.
#
# Three-stage build inside the container:
#
#   Stage 1: build libdevmapper.a from LVM2 source. Alpine 3.20 ships only a
#            shared libdevmapper.so; cryptsetup needs the static .a to link
#            with `-all-static`. We build LVM2's device-mapper subset and
#            install /usr/lib/libdevmapper.a + /usr/include/libdevmapper.h.
#
#   Stage 2: cryptsetup configure + make against that newly-installed .a.
#            Output binary is at the source root (cryptsetup 2.x layout,
#            NOT src/).
#
#   Stage 3: build static mke2fs from e2fsprogs source. Ubuntu's busybox-
#            static does not include `mkfs.ext2`, and Alpine's
#            e2fsprogs-static ships only static libraries (no binaries).
#            The init's unseal flow needs mkfs.ext2 to format the decrypted
#            mapper on first boot.
#
# `bash` is required inside the container because cryptsetup's
# tests/generate-symbols-list (run during `make all`) has a `#!/bin/bash`
# shebang and Alpine's busybox sh is not bash.
#
# Usage:
#   ./build-cryptsetup.sh OUTPUT_DIR
#
# The script writes two files:
#   $OUTPUT_DIR/cryptsetup
#   $OUTPUT_DIR/mkfs.ext2
# Both are statically linked (verified via ldd). They get baked into the
# initrd by build-initrd.sh, which expects $CRYPTSETUP_BINARY and
# $MKFS_EXT2_BINARY to point at them (or operator-supplied equivalents).
#
# Environment (all required; build-config provides defaults):
#   SOURCE_DATE_EPOCH         Reproducibility anchor.
#   CRYPTSETUP_VERSION        e.g. 2.7.5
#   CRYPTSETUP_SHA256         sha256 of cryptsetup-$VERSION.tar.xz
#   LVM2_VERSION              e.g. 2.03.23
#   LVM2_SHA256               sha256 of LVM2.$VERSION.tgz
#   E2FSPROGS_VERSION         e.g. 1.47.0
#   E2FSPROGS_SHA256          sha256 of e2fsprogs-$VERSION.tar.xz
#   CRYPTSETUP_BUILDER_IMAGE  pinned alpine@sha256:... container image
#
# Optional:
#   CRYPTSETUP_BUILDER        container CLI (default: docker)
#
# ==============================================================================

set -euo pipefail

usage() {
    echo "Usage: $0 OUTPUT_DIR"
    echo ""
    echo "Builds static cryptsetup + mkfs.ext2 inside a pinned Alpine container."
    echo "Writes \$OUTPUT_DIR/cryptsetup and \$OUTPUT_DIR/mkfs.ext2."
    echo ""
    echo "All env vars listed in the script header are required; source"
    echo "misc/AMDSEV/build-config to get the canonical pinned values."
    exit 1
}

if [[ $# -lt 1 ]] || [[ "${1:-}" == "-h" ]] || [[ "${1:-}" == "--help" ]]; then
    usage
fi

log_section() { echo ""; echo "=========================================="; echo "$*"; echo "=========================================="; }
log_info()    { echo "  [INFO] $*"; }
log_ok()      { echo "  [OK] $*"; }
log_warn()    { echo "  [WARN] $*"; }
die()         { echo "ERROR: $*" >&2; exit 1; }

to_abs_path() {
    local path="$1"
    if [[ "$path" = /* ]]; then
        printf '%s\n' "$path"
    else
        printf '%s/%s\n' "$(pwd -P)" "$path"
    fi
}

OUTPUT_DIR="$(to_abs_path "$1")"

# Required env vars (build-config supplies all of these).
: "${SOURCE_DATE_EPOCH:?SOURCE_DATE_EPOCH must be set}"
: "${CRYPTSETUP_VERSION:?CRYPTSETUP_VERSION must be set}"
: "${CRYPTSETUP_SHA256:?CRYPTSETUP_SHA256 must be set}"
: "${LVM2_VERSION:?LVM2_VERSION must be set}"
: "${LVM2_SHA256:?LVM2_SHA256 must be set}"
: "${E2FSPROGS_VERSION:?E2FSPROGS_VERSION must be set}"
: "${E2FSPROGS_SHA256:?E2FSPROGS_SHA256 must be set}"
: "${CRYPTSETUP_BUILDER_IMAGE:?CRYPTSETUP_BUILDER_IMAGE must be set (pinned alpine@sha256:... digest)}"

CRYPTSETUP_BUILDER="${CRYPTSETUP_BUILDER:-docker}"
command -v "$CRYPTSETUP_BUILDER" >/dev/null 2>&1 \
    || die "Container runtime '$CRYPTSETUP_BUILDER' not found. Install docker/podman or set CRYPTSETUP_BUILDER."

REQUIRED_TOOLS=(curl tar sha256sum awk ldd)
for tool in "${REQUIRED_TOOLS[@]}"; do
    command -v "$tool" >/dev/null 2>&1 || die "Required host tool not found: $tool"
done

mkdir -p "$OUTPUT_DIR"
[[ -w "$OUTPUT_DIR" ]] || die "Output directory is not writable: $OUTPUT_DIR"

log_section "Building static cryptsetup + mkfs.ext2"
echo "Configuration:"
echo "  Output dir:        $OUTPUT_DIR"
echo "  cryptsetup:        ${CRYPTSETUP_VERSION}"
echo "  LVM2:              ${LVM2_VERSION}"
echo "  e2fsprogs:         ${E2FSPROGS_VERSION}"
echo "  Container image:   ${CRYPTSETUP_BUILDER_IMAGE}"
echo "  Container runtime: ${CRYPTSETUP_BUILDER}"
echo "  SOURCE_DATE_EPOCH: ${SOURCE_DATE_EPOCH}"

WORK_DIR="$(mktemp -d)"
cleanup() {
    local exit_code=$?
    [[ -d "$WORK_DIR" ]] && rm -rf "$WORK_DIR"
    exit "$exit_code"
}
trap cleanup EXIT INT TERM

log_info "Working directory: $WORK_DIR"

# ==============================================================================
# Download + verify sources (host-side)
# ==============================================================================

log_section "Download Sources"
pushd "$WORK_DIR" >/dev/null

# kernel.org tarball URLs are organised under major.minor (e.g. v2.7).
CRYPTSETUP_MAJOR_MINOR="$(printf '%s' "$CRYPTSETUP_VERSION" | awk -F. '{print $1"."$2}')"
CRYPTSETUP_URL="https://www.kernel.org/pub/linux/utils/cryptsetup/v${CRYPTSETUP_MAJOR_MINOR}/cryptsetup-${CRYPTSETUP_VERSION}.tar.xz"
CRYPTSETUP_TARBALL="cryptsetup-${CRYPTSETUP_VERSION}.tar.xz"

log_info "Downloading $CRYPTSETUP_URL"
curl -fLsS -o "$CRYPTSETUP_TARBALL" "$CRYPTSETUP_URL"
ACTUAL_SHA256="$(sha256sum "$CRYPTSETUP_TARBALL" | awk '{print $1}')"
[[ "$ACTUAL_SHA256" == "$CRYPTSETUP_SHA256" ]] \
    || die "cryptsetup checksum mismatch (expected $CRYPTSETUP_SHA256, got $ACTUAL_SHA256)"
log_ok "cryptsetup source verified"

LVM2_TARBALL="LVM2.${LVM2_VERSION}.tgz"
LVM2_URL="https://mirrors.kernel.org/sourceware/lvm2/${LVM2_TARBALL}"
log_info "Downloading $LVM2_URL"
curl -fLsS -o "$LVM2_TARBALL" "$LVM2_URL"
ACTUAL_SHA256="$(sha256sum "$LVM2_TARBALL" | awk '{print $1}')"
[[ "$ACTUAL_SHA256" == "$LVM2_SHA256" ]] \
    || die "LVM2 checksum mismatch (expected $LVM2_SHA256, got $ACTUAL_SHA256)"
log_ok "LVM2 source verified"

E2FSPROGS_TARBALL="e2fsprogs-${E2FSPROGS_VERSION}.tar.xz"
E2FSPROGS_URL="https://mirrors.kernel.org/pub/linux/kernel/people/tytso/e2fsprogs/v${E2FSPROGS_VERSION}/${E2FSPROGS_TARBALL}"
log_info "Downloading $E2FSPROGS_URL"
curl -fLsS -o "$E2FSPROGS_TARBALL" "$E2FSPROGS_URL"
ACTUAL_SHA256="$(sha256sum "$E2FSPROGS_TARBALL" | awk '{print $1}')"
[[ "$ACTUAL_SHA256" == "$E2FSPROGS_SHA256" ]] \
    || die "e2fsprogs checksum mismatch (expected $E2FSPROGS_SHA256, got $ACTUAL_SHA256)"
log_ok "e2fsprogs source verified"

log_info "Extracting sources"
tar -xf "$CRYPTSETUP_TARBALL"
tar -xzf "$LVM2_TARBALL"
tar -xf "$E2FSPROGS_TARBALL"

# ==============================================================================
# Containerised build (Alpine + musl + *-static apk packages)
# ==============================================================================

log_section "Build Inside Container"
log_info "Image: $CRYPTSETUP_BUILDER_IMAGE"

# The container runs as root (apk add requires it). Once the build is done,
# chown the output binaries to the invoking host user so the cleanup trap's
# rm -rf "$WORK_DIR" doesn't trip over root-owned files.
HOST_UID="$(id -u)"
HOST_GID="$(id -g)"

"$CRYPTSETUP_BUILDER" run --rm \
    -v "$WORK_DIR:/build" \
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

        # Stage 3: static mke2fs from e2fsprogs.
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
        # owns $WORK_DIR itself, so the trap'"'"'s rm -rf can still unlink
        # them; but make the leaf directories writable by the host user so any
        # follow-up inspection (find, ls) does not hit permission errors.
        chown -R "${HOST_UID}:${HOST_GID}" /build
    '

popd >/dev/null

# ==============================================================================
# Verify + install to OUTPUT_DIR
# ==============================================================================

log_section "Verify + Install"

for src in cryptsetup-static mkfs.ext2-static; do
    [[ -x "$WORK_DIR/$src" ]] \
        || die "container build did not produce $WORK_DIR/$src"
done

log_info "Verifying static linkage"
for src in cryptsetup-static mkfs.ext2-static; do
    LDD_OUT="$(ldd "$WORK_DIR/$src" 2>&1 || true)"
    if echo "$LDD_OUT" | grep -qE "not a dynamic executable|statically linked"; then
        log_ok "$src is statically linked"
    else
        log_warn "$src may not be fully static:"
        echo "$LDD_OUT" | sed 's/^/    /'
        die "$src must be statically linked to run in the initrd"
    fi
done

log_info "Normalising timestamps for reproducibility"
touch -d "@${SOURCE_DATE_EPOCH}" \
    "$WORK_DIR/cryptsetup-static" \
    "$WORK_DIR/mkfs.ext2-static"

log_info "Installing into $OUTPUT_DIR"
install -m 0755 "$WORK_DIR/cryptsetup-static"  "$OUTPUT_DIR/cryptsetup"
install -m 0755 "$WORK_DIR/mkfs.ext2-static"   "$OUTPUT_DIR/mkfs.ext2"
touch -d "@${SOURCE_DATE_EPOCH}" \
    "$OUTPUT_DIR/cryptsetup" \
    "$OUTPUT_DIR/mkfs.ext2"

echo ""
echo "=========================================="
echo "[OK] Built static binaries"
echo "=========================================="
echo "  $OUTPUT_DIR/cryptsetup"
echo "  $OUTPUT_DIR/mkfs.ext2"
echo "=========================================="
