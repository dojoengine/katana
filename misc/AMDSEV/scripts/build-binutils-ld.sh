#!/bin/bash
# ==============================================================================
# BUILD-BINUTILS-LD.SH
# ==============================================================================
#
# Build the static GNU `ld` binary the initrd needs for in-guest cairo-native
# AOT linking, inside a pinned Alpine container.
#
# Why the guest needs a linker at all: the bundled katana is the cairo-native
# release build. With --enable-native-compilation, it AOT-compiles each
# contract class at runtime and shells out to `ld` (PATH lookup; the guest
# init exports PATH=/bin) to link the emitted object into a dlopen-able .so:
#
#   ld --hash-style=gnu -shared -L/lib/../lib64 -L/usr/lib/../lib64 \
#      -o <out.so> -lc <in.o>
#
# That argv is an internal detail of cairo-native (src/ffi.rs in the locked
# cairo-native crate) — re-verify it when bumping cairo-native, since a future
# version could add flags or switch linkers. The `-lc` input is provided by
# the /lib64/libc.so linker script that build-initrd.sh generates.
#
# `make all-ld` builds only bfd + libiberty + ld (skips gas, gold, binutils
# proper, gprofng). The static link needs TWO make phases:
#
#   make configure-host              runs every host sub-configure with clean
#                                    LDFLAGS — a command-line LDFLAGS override
#                                    would leak into the sub-configures' link
#                                    tests, and gcc does not understand the
#                                    libtool-only `-all-static`, so configure
#                                    would bail with GCC_NO_EXECUTABLES.
#   make all-ld LDFLAGS=-all-static  the sub-configures are already cached, so
#                                    the override now only reaches the actual
#                                    (libtool-driven) link of ld — `-all-static`
#                                    is the libtool spelling for a fully static
#                                    executable. (A plain `-static` does NOT
#                                    work: libtool swallows it as "prefer
#                                    static libtool libs" and still links
#                                    libc/libz dynamically.)
#
# Release tarballs ship pregenerated ldgram.c/ldlex.c, so no flex/bison is
# needed. MAKEINFO=true skips doc generation (no texinfo in the container).
#
# Usage:
#   ./build-binutils-ld.sh OUTPUT_DIR
#
# The script writes one file:
#   $OUTPUT_DIR/ld
# Statically linked (verified via ldd). It gets baked into the initrd at
# /bin/ld by build-initrd.sh, which expects $LD_BINARY to point at it (or an
# operator-supplied equivalent).
#
# Environment (all required; build-config provides defaults):
#   SOURCE_DATE_EPOCH         Reproducibility anchor.
#   BINUTILS_VERSION          e.g. 2.44
#   BINUTILS_SHA256           sha256 of binutils-$VERSION.tar.xz
#   CRYPTSETUP_BUILDER_IMAGE  pinned alpine@sha256:... container image
#                             (shared with build-cryptsetup.sh)
#
# Optional:
#   CRYPTSETUP_BUILDER        container CLI (default: docker)
#
# ==============================================================================

set -euo pipefail

usage() {
    echo "Usage: $0 OUTPUT_DIR"
    echo ""
    echo "Builds a static GNU ld inside a pinned Alpine container."
    echo "Writes \$OUTPUT_DIR/ld."
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
: "${BINUTILS_VERSION:?BINUTILS_VERSION must be set}"
: "${BINUTILS_SHA256:?BINUTILS_SHA256 must be set}"
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

log_section "Building static GNU ld"
echo "Configuration:"
echo "  Output dir:        $OUTPUT_DIR"
echo "  binutils:          ${BINUTILS_VERSION}"
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
# Download + verify source (host-side)
# ==============================================================================

log_section "Download Sources"
pushd "$WORK_DIR" >/dev/null

BINUTILS_TARBALL="binutils-${BINUTILS_VERSION}.tar.xz"
BINUTILS_URL="https://ftp.gnu.org/gnu/binutils/${BINUTILS_TARBALL}"

log_info "Downloading $BINUTILS_URL"
curl -fLsS -o "$BINUTILS_TARBALL" "$BINUTILS_URL"
ACTUAL_SHA256="$(sha256sum "$BINUTILS_TARBALL" | awk '{print $1}')"
[[ "$ACTUAL_SHA256" == "$BINUTILS_SHA256" ]] \
    || die "binutils checksum mismatch (expected $BINUTILS_SHA256, got $ACTUAL_SHA256)"
log_ok "binutils source verified"

log_info "Extracting source"
tar -xf "$BINUTILS_TARBALL"

# ==============================================================================
# Containerised build (Alpine + musl)
# ==============================================================================

log_section "Build Inside Container"
log_info "Image: $CRYPTSETUP_BUILDER_IMAGE"

# The container runs as root (apk add requires it). Once the build is done,
# chown the output binary to the invoking host user so the cleanup trap's
# rm -rf "$WORK_DIR" doesn't trip over root-owned files.
HOST_UID="$(id -u)"
HOST_GID="$(id -g)"

"$CRYPTSETUP_BUILDER" run --rm \
    -v "$WORK_DIR:/build" \
    -w "/build" \
    -e "SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH}" \
    -e "HOST_UID=${HOST_UID}" \
    -e "HOST_GID=${HOST_GID}" \
    -e "BINUTILS_VERSION=${BINUTILS_VERSION}" \
    "$CRYPTSETUP_BUILDER_IMAGE" \
    sh -euc '
        apk add --no-cache build-base zlib-dev zlib-static

        cd "/build/binutils-${BINUTILS_VERSION}"
        # --target=x86_64-unknown-linux-gnu: self-documenting; the default
        #   emulation is elf_x86_64 either way, and the built-in SEARCH_DIRs
        #   are irrelevant — cairo-native passes explicit -L paths and the
        #   -lc input lives at /lib64/libc.so in the initrd.
        # --disable-plugins: no dlopen-based LTO plugin machinery in a
        #   static binary.
        # --without-zstd: bfd only uses zstd for compressed debug sections,
        #   which cairo-native-emitted objects never carry — avoids a
        #   libzstd static dep.
        ./configure \
            --target=x86_64-unknown-linux-gnu \
            --disable-nls \
            --disable-werror \
            --disable-plugins \
            --disable-gprofng \
            --disable-gdb --disable-gdbserver --disable-sim \
            --disable-shared --enable-static \
            --without-zstd \
            --with-system-zlib
        # Two-phase make — see the header comment: configure-host first with
        # clean LDFLAGS (gcc chokes on the libtool-only -all-static in
        # configure link tests), then the real build with -all-static so the
        # libtool link of ld is fully static.
        make -j"$(nproc)" configure-host MAKEINFO=true
        make -j"$(nproc)" all-ld LDFLAGS="-all-static" MAKEINFO=true
        strip ld/ld-new
        cp ld/ld-new /build/ld-static

        chown "${HOST_UID}:${HOST_GID}" /build/ld-static
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

[[ -x "$WORK_DIR/ld-static" ]] \
    || die "container build did not produce $WORK_DIR/ld-static"

log_info "Verifying static linkage"
LDD_OUT="$(ldd "$WORK_DIR/ld-static" 2>&1 || true)"
if echo "$LDD_OUT" | grep -qE "not a dynamic executable|statically linked"; then
    log_ok "ld is statically linked"
else
    log_warn "ld may not be fully static:"
    echo "$LDD_OUT" | sed 's/^/    /'
    die "ld must be statically linked to run in the initrd"
fi

log_info "Verifying functionality"
"$WORK_DIR/ld-static" --version | head -1 | grep -q "GNU ld" \
    || die "built ld does not identify as GNU ld"
"$WORK_DIR/ld-static" --help 2>/dev/null | grep -q "elf_x86_64" \
    || die "built ld does not support the elf_x86_64 emulation"
log_ok "ld is functional ($("$WORK_DIR/ld-static" --version | head -1))"

log_info "Normalising timestamps for reproducibility"
touch -d "@${SOURCE_DATE_EPOCH}" "$WORK_DIR/ld-static"

log_info "Installing into $OUTPUT_DIR"
install -m 0755 "$WORK_DIR/ld-static" "$OUTPUT_DIR/ld"
touch -d "@${SOURCE_DATE_EPOCH}" "$OUTPUT_DIR/ld"

echo ""
echo "=========================================="
echo "[OK] Built static binary"
echo "=========================================="
echo "  $OUTPUT_DIR/ld"
echo "=========================================="
