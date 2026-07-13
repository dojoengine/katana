#!/bin/bash
# ==============================================================================
# BUILD-INITRD.SH
# ==============================================================================
#
# This script downloads all required dependencies and builds a minimal initrd
# for running Katana inside AMD SEV-SNP confidential VMs.
#
# Dependencies downloaded:
#   - busybox-static:         Provides shell and basic utilities
#   - linux-modules:          Contains dm-crypt.ko and dm-integrity.ko for LUKS-based
#                             sealed storage. (dm-mod is built into the Ubuntu 6.8
#                             kernel — see modules.builtin — so we don't ship it.)
#   - glibc runtime packages: Provides dynamic linker and shared libraries
#   - linux-modules-extra:    Contains SEV-SNP kernel modules (tsm.ko, sev-guest.ko)
#                             and qemu_fw_cfg.ko (host-supplied Katana args via
#                             QEMU fw_cfg — see load_fw_cfg_args in the init).
#                             The chain config disk's virtio_blk + ext2/ext4
#                             drivers are kernel-builtins (see modules.builtin),
#                             so mount_chain_disk in the init can mount /dev/vda
#                             directly without insmod.
#
# Sealed-storage builds also consume two pre-built static binaries —
# `cryptsetup` and `mkfs.ext2` — supplied via the CRYPTSETUP_BINARY and
# MKFS_EXT2_BINARY env vars. Building them is the job of
# `misc/AMDSEV/build-cryptsetup.sh`; `misc/AMDSEV/build.sh` calls that script
# automatically when the operator did not pass `--cryptsetup`/`--mkfs-ext2`
# paths. Decoupling the source build from this script means an unsealed-only
# initrd run never needs Docker on the host.
#
# Usage:
#   ./build-initrd.sh KATANA_BINARY OUTPUT_INITRD [KERNEL_VERSION]
#
# Environment:
#   SOURCE_DATE_EPOCH                REQUIRED. Unix timestamp for reproducible builds.
#   BUSYBOX_PKG_VERSION              REQUIRED. Exact apt package version (e.g., 1:1.36.1-6ubuntu3.1).
#   BUSYBOX_PKG_SHA256               REQUIRED. SHA256 checksum of the busybox .deb package.
#   GLIBC_RUNTIME_PACKAGES           REQUIRED for dynamic Katana. Space-separated apt package specs.
#   GLIBC_RUNTIME_PACKAGE_SHA256S    REQUIRED for dynamic Katana. Space-separated package=sha256 entries.
#   KERNEL_MODULES_EXTRA_PKG_VERSION REQUIRED. Exact apt package version.
#   KERNEL_MODULES_EXTRA_PKG_SHA256  REQUIRED. SHA256 checksum of the modules .deb package.
#
# ==============================================================================

set -euo pipefail
# File modes are part of the cpio archive and therefore the SEV-SNP launch measurement.
umask 022

REQUIRED_APPLETS=(sh mount umount sleep kill cat mkdir ln mknod ip insmod poweroff sync \
                   tr grep rm mkfifo cp dd)
SYMLINK_APPLETS=(sh mount umount mkdir mknod switch_root ip insmod sleep kill cat ln poweroff sync \
                  tr grep rm mkfifo cp dd)
OPTIONAL_RUNTIME_LIBS=(libnss_dns.so.2 libnss_files.so.2 libresolv.so.2)
# `mkfs.ext2` is not a busybox-static applet on Ubuntu, so a static binary
# (built by build-cryptsetup.sh from e2fsprogs source) is supplied via
# MKFS_EXT2_BINARY and installed as `/bin/mkfs.ext2`. `blkid` is similarly
# absent from busybox-static; the init avoids it via a try-mount-then-mkfs
# fallback.

usage() {
    echo "Usage: $0 KATANA_BINARY OUTPUT_INITRD [KERNEL_VERSION]"
    echo ""
    echo "Self-contained initrd builder for Katana TEE VM with AMD SEV-SNP support."
    echo "Downloads all required dependencies (busybox, glibc runtime, kernel modules) automatically."
    echo ""
    echo "ARGUMENTS:"
    echo "  KATANA_BINARY    Path to the katana binary (glibc dynamic or static)"
    echo "  OUTPUT_INITRD    Output path for the generated initrd.img"
    echo "  KERNEL_VERSION   Kernel version for module lookup (or set KERNEL_VERSION env var)"
    echo ""
    echo "ENVIRONMENT VARIABLES (all required for the canonical sealed build):"
    echo "  SOURCE_DATE_EPOCH                Unix timestamp for reproducible builds"
    echo "  BUSYBOX_PKG_VERSION              Exact apt package version (e.g., 1:1.36.1-6ubuntu3.1)"
    echo "  BUSYBOX_PKG_SHA256               SHA256 checksum of the busybox .deb package"
    echo "  GLIBC_RUNTIME_PACKAGES           Space-separated apt package specs for dynamic Katana"
    echo "  GLIBC_RUNTIME_PACKAGE_SHA256S    Space-separated package=sha256 entries"
    echo "  KERNEL_MODULES_PKG_VERSION       Exact apt package version for linux-modules"
    echo "  KERNEL_MODULES_PKG_SHA256        SHA256 checksum of the linux-modules .deb"
    echo "  KERNEL_MODULES_EXTRA_PKG_VERSION Exact apt package version for linux-modules-extra"
    echo "  KERNEL_MODULES_EXTRA_PKG_SHA256  SHA256 checksum of the linux-modules-extra .deb"
    echo "  CRYPTSETUP_BINARY                Path to a pre-built static cryptsetup binary"
    echo "                                   (build via misc/AMDSEV/build-cryptsetup.sh)"
    echo "  MKFS_EXT2_BINARY                 Path to a pre-built static mkfs.ext2 binary"
    echo "                                   (built by the same script)"
    echo "  SNP_DERIVEKEY_BINARY             Path to a pre-built static snp-derivekey"
    echo "                                   binary (required for sealed boot to work at"
    echo "                                   runtime). Build with:"
    echo "                                     cargo build -p katana-tee --features snp \\"
    echo "                                                 --bin snp-derivekey --release \\"
    echo "                                                 --target x86_64-unknown-linux-musl"
    echo ""
    echo "All of these have canonical defaults / auto-build paths via"
    echo "misc/AMDSEV/build.sh; the expected invocation is to source"
    echo "misc/AMDSEV/build-config and run build.sh, not this script directly."
    echo ""
    echo "OPTIONAL ENVIRONMENT VARIABLES:"
    echo "  PAYMASTER_BINARY                 Path to a prebuilt paymaster-service binary"
    echo "                                   (katana release asset). Bundled at /bin in"
    echo "                                   the initrd so the guest supports --paymaster."
    echo "  VRF_BINARY                       Path to a prebuilt vrf-server binary (katana"
    echo "                                   release asset). Bundled at /bin so the guest"
    echo "                                   supports --vrf. Both-or-neither with"
    echo "                                   PAYMASTER_BINARY."
    echo "  KATANA_UNSEALED_BUILD            Set to 1 to opt OUT of the sealed build."
    echo "                                   Produces an unsealed-only initrd that mounts"
    echo "                                   /dev/sda as plain ext4: no cryptsetup, no"
    echo "                                   dm-* modules, no snp-derivekey. Used by CI"
    echo "                                   on hosts without Docker and for cheap dev"
    echo "                                   iteration."
    echo ""
    echo "EXAMPLES:"
    echo "  export SOURCE_DATE_EPOCH=\$(date +%s)"
    echo "  export BUSYBOX_PKG_VERSION='1:1.36.1-6ubuntu3.1'"
    echo "  export BUSYBOX_PKG_SHA256='abc123...'"
    echo "  export GLIBC_RUNTIME_PACKAGES='libc6=2.39-0ubuntu8.7 libgcc-s1=14.2.0-4ubuntu2~24.04.1'"
    echo "  export GLIBC_RUNTIME_PACKAGE_SHA256S='libc6=abc123... libgcc-s1=def456...'"
    echo "  export KERNEL_MODULES_PKG_VERSION='6.8.0-90.99'"
    echo "  export KERNEL_MODULES_PKG_SHA256='aaa111...'"
    echo "  export KERNEL_MODULES_EXTRA_PKG_VERSION='6.8.0-90.99'"
    echo "  export KERNEL_MODULES_EXTRA_PKG_SHA256='def456...'"
    echo "  export CRYPTSETUP_BINARY=./target/cryptsetup-static/cryptsetup"
    echo "  export MKFS_EXT2_BINARY=./target/cryptsetup-static/mkfs.ext2"
    echo "  export SNP_DERIVEKEY_BINARY=./target/.../snp-derivekey"
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
echo "  glibc runtime:         ${GLIBC_RUNTIME_PACKAGES:-<not set>}"
echo "  linux-modules-extra:   ${KERNEL_MODULES_EXTRA_PKG_VERSION:-<not set>}"
echo "Static binaries (sealed-mode only):"
echo "  cryptsetup binary:     ${CRYPTSETUP_BINARY:-<not set>}"
echo "  mkfs.ext2 binary:      ${MKFS_EXT2_BINARY:-<not set>}"
echo "  snp-derivekey binary:  ${SNP_DERIVEKEY_BINARY:-<not set>}"
echo "Sidecar binaries (optional):"
echo "  paymaster-service:     ${PAYMASTER_BINARY:-<not set>}"
echo "  vrf-server:            ${VRF_BINARY:-<not set>}"

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

REQUIRED_TOOLS=(apt-get dpkg-deb sha256sum cpio gzip zstd find sort touch du mktemp awk grep tr readelf readlink xargs)
for tool in "${REQUIRED_TOOLS[@]}"; do
    command -v "$tool" >/dev/null 2>&1 || die "Required tool not found: $tool"
done

# Sealed-storage is the canonical build. Required env vars come from
# `misc/AMDSEV/build-config` (kernel module pins) plus three pre-built static
# binary paths produced by `misc/AMDSEV/build-cryptsetup.sh` and
# `cargo build -p katana-tee --features snp` respectively. Both source builds
# are wired up by `misc/AMDSEV/build.sh`; sourcing build-config and running
# build.sh is the standard invocation (see `.github/workflows/amdsev-initrd-test.yml`).
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
    : "${KERNEL_MODULES_PKG_VERSION:?canonical sealed build requires KERNEL_MODULES_PKG_VERSION (source build-config or set KATANA_UNSEALED_BUILD=1 to opt out)}"
    : "${KERNEL_MODULES_PKG_SHA256:?canonical sealed build requires KERNEL_MODULES_PKG_SHA256}"
fi

# Pre-built static binary paths. The install step below runs after
# `cd "$INITRD_DIR"` and a relative path would no longer reach the host
# binary from there, so resolve to absolute up front (same treatment as
# KATANA_BINARY above).
#
#   CRYPTSETUP_BINARY / MKFS_EXT2_BINARY  — produced by build-cryptsetup.sh
#   SNP_DERIVEKEY_BINARY                  — produced by `cargo build -p katana-tee
#                                           --features snp --bin snp-derivekey`
#
# All three are required for sealed builds. The init's unseal flow spawns
# snp-derivekey to read 32 bytes of SNP-derived key into the LUKS keyfile
# FIFO; without it the cryptsetup call blocks indefinitely.
CRYPTSETUP_BINARY="${CRYPTSETUP_BINARY:-}"
MKFS_EXT2_BINARY="${MKFS_EXT2_BINARY:-}"
SNP_DERIVEKEY_BINARY="${SNP_DERIVEKEY_BINARY:-}"
if [[ "$SEALED_STORAGE_BUILD" -eq 1 ]]; then
    [[ -n "$CRYPTSETUP_BINARY" ]] \
        || die "SEALED_STORAGE_BUILD=1 but CRYPTSETUP_BINARY is unset (build via build.sh which auto-builds it, or pass --cryptsetup PATH)"
    [[ -n "$MKFS_EXT2_BINARY" ]] \
        || die "SEALED_STORAGE_BUILD=1 but MKFS_EXT2_BINARY is unset (build via build.sh which auto-builds it, or pass --mkfs-ext2 PATH)"
    [[ -n "$SNP_DERIVEKEY_BINARY" ]] \
        || die "SEALED_STORAGE_BUILD=1 but SNP_DERIVEKEY_BINARY is unset (build via build.sh which auto-builds it, or pass --snp-derivekey PATH)"

    CRYPTSETUP_BINARY="$(to_abs_path "$CRYPTSETUP_BINARY")"
    MKFS_EXT2_BINARY="$(to_abs_path "$MKFS_EXT2_BINARY")"
    SNP_DERIVEKEY_BINARY="$(to_abs_path "$SNP_DERIVEKEY_BINARY")"

    [[ -x "$CRYPTSETUP_BINARY" ]] \
        || die "CRYPTSETUP_BINARY=$CRYPTSETUP_BINARY does not exist or is not executable"
    [[ -x "$MKFS_EXT2_BINARY" ]] \
        || die "MKFS_EXT2_BINARY=$MKFS_EXT2_BINARY does not exist or is not executable"
    [[ -x "$SNP_DERIVEKEY_BINARY" ]] \
        || die "SNP_DERIVEKEY_BINARY=$SNP_DERIVEKEY_BINARY does not exist or is not executable"
fi

# Cartridge sidecar binaries (paymaster-service + vrf-server), prebuilt release
# assets supplied via PAYMASTER_BINARY / VRF_BINARY (build.sh --paymaster-bin /
# --vrf-bin). Optional: when unset the image builds WITHOUT sidecars and
# katana's --paymaster/--vrf flags fail in the guest (the sidecar resolver
# falls through to an interactive download prompt that dies on the enclave's
# non-TTY stdin). Release builds always supply both (amdsev-release.yml).
# Both-or-neither: --vrf uses the paymaster as relayer/forwarder, so a
# half-bundled image is always a misconfiguration.
PAYMASTER_BINARY="${PAYMASTER_BINARY:-}"
VRF_BINARY="${VRF_BINARY:-}"
if [[ -n "$PAYMASTER_BINARY" || -n "$VRF_BINARY" ]]; then
    [[ -n "$PAYMASTER_BINARY" && -n "$VRF_BINARY" ]] \
        || die "PAYMASTER_BINARY and VRF_BINARY must be supplied together (got only one)"
    PAYMASTER_BINARY="$(to_abs_path "$PAYMASTER_BINARY")"
    VRF_BINARY="$(to_abs_path "$VRF_BINARY")"
    [[ -x "$PAYMASTER_BINARY" ]] \
        || die "PAYMASTER_BINARY=$PAYMASTER_BINARY does not exist or is not executable"
    [[ -x "$VRF_BINARY" ]] \
        || die "VRF_BINARY=$VRF_BINARY does not exist or is not executable"
else
    log_warn "PAYMASTER_BINARY/VRF_BINARY not set — image will NOT support --paymaster/--vrf in the guest"
fi

log_ok "Preflight validation complete (sealed-storage build: $([ "$SEALED_STORAGE_BUILD" -eq 1 ] && echo yes || echo no))"

elf_interpreter() {
    readelf -l "$1" 2>/dev/null |
        awk -F': ' '/Requesting program interpreter/ { gsub(/\]/, "", $2); print $2; exit }'
}

KATANA_INTERPRETER="$(elf_interpreter "$KATANA_BINARY" || true)"
if [[ -n "$KATANA_INTERPRETER" ]]; then
    log_info "Katana is dynamically linked (interpreter: $KATANA_INTERPRETER)"
else
    log_warn "Katana has no ELF interpreter; treating it as statically linked"
fi

# All bundled dynamic ELF executables must share one interpreter — the dynamic
# runtime installer walks a single pinned-glibc package pool. Static binaries
# (no interpreter) are fine alongside dynamic ones.
RUNTIME_INTERPRETER="$KATANA_INTERPRETER"
for extra_bin in "$PAYMASTER_BINARY" "$VRF_BINARY"; do
    [[ -n "$extra_bin" ]] || continue
    extra_interp="$(elf_interpreter "$extra_bin" || true)"
    [[ -n "$extra_interp" ]] || continue
    if [[ -z "$RUNTIME_INTERPRETER" ]]; then
        RUNTIME_INTERPRETER="$extra_interp"
    elif [[ "$extra_interp" != "$RUNTIME_INTERPRETER" ]]; then
        die "interpreter mismatch: $extra_bin wants $extra_interp, other binaries use $RUNTIME_INTERPRETER"
    fi
done

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

find_downloaded_deb() {
    local package="$1"
    local matches=()

    while IFS= read -r deb; do
        matches+=("$deb")
    done < <(find "$PACKAGES_DIR" -maxdepth 1 -type f -name "${package}_*.deb" -print | LC_ALL=C sort)

    if [[ ${#matches[@]} -ne 1 ]]; then
        die "expected exactly one downloaded .deb for ${package}, found ${#matches[@]}"
    fi

    printf '%s\n' "${matches[0]}"
}

expected_runtime_sha256() {
    local package="$1"
    local entry

    for entry in ${GLIBC_RUNTIME_PACKAGE_SHA256S:-}; do
        if [[ "${entry%%=*}" == "$package" ]]; then
            printf '%s\n' "${entry#*=}"
            return 0
        fi
    done

    return 1
}

verify_package_sha256() {
    local package="$1"
    local expected="$2"
    local deb
    local actual

    deb="$(find_downloaded_deb "$package")"
    actual="$(sha256sum "$deb" | awk '{print $1}')"
    if [[ "$actual" != "$expected" ]]; then
        die "${package} checksum mismatch (expected $expected, got $actual)"
    fi
    log_ok "${package} checksum verified"
}

# ==============================================================================
# SECTION 1: Download Required Packages
# ==============================================================================

log_section "Download Required Packages"
PACKAGES_DIR="$WORK_DIR/packages"
mkdir -p "$PACKAGES_DIR"

pushd "$PACKAGES_DIR" >/dev/null

: "${BUSYBOX_PKG_VERSION:?BUSYBOX_PKG_VERSION not set - required for reproducible builds}"
: "${KERNEL_MODULES_EXTRA_PKG_VERSION:?KERNEL_MODULES_EXTRA_PKG_VERSION not set - required for reproducible builds}"
if [[ -n "$RUNTIME_INTERPRETER" ]]; then
    : "${GLIBC_RUNTIME_PACKAGES:?GLIBC_RUNTIME_PACKAGES not set - required for reproducible dynamic binary builds}"
    : "${GLIBC_RUNTIME_PACKAGE_SHA256S:?GLIBC_RUNTIME_PACKAGE_SHA256S not set - required for reproducible dynamic binary builds}"
fi

log_info "Downloading busybox-static=${BUSYBOX_PKG_VERSION}"
apt-get download "busybox-static=${BUSYBOX_PKG_VERSION}"

if [[ "$SEALED_STORAGE_BUILD" -eq 1 ]]; then
    log_info "Downloading linux-modules-${KERNEL_VERSION}-generic=${KERNEL_MODULES_PKG_VERSION}"
    apt-get download "linux-modules-${KERNEL_VERSION}-generic=${KERNEL_MODULES_PKG_VERSION}"
fi

log_info "Downloading linux-modules-extra-${KERNEL_VERSION}-generic=${KERNEL_MODULES_EXTRA_PKG_VERSION}"
apt-get download "linux-modules-extra-${KERNEL_VERSION}-generic=${KERNEL_MODULES_EXTRA_PKG_VERSION}"

if [[ -n "$RUNTIME_INTERPRETER" ]]; then
    for package_spec in $GLIBC_RUNTIME_PACKAGES; do
        package_name="${package_spec%%=*}"
        [[ "$package_name" != "$package_spec" ]] || die "invalid GLIBC_RUNTIME_PACKAGES entry: $package_spec"

        log_info "Downloading ${package_spec}"
        apt-get download "$package_spec"
    done
fi

echo ""
echo "Downloaded packages:"
ls -lh *.deb

: "${BUSYBOX_PKG_SHA256:?BUSYBOX_PKG_SHA256 not set - required for reproducible builds}"
: "${KERNEL_MODULES_EXTRA_PKG_SHA256:?KERNEL_MODULES_EXTRA_PKG_SHA256 not set - required for reproducible builds}"

log_info "Verifying busybox-static checksum"
verify_package_sha256 "busybox-static" "$BUSYBOX_PKG_SHA256"

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
verify_package_sha256 "linux-modules-extra-${KERNEL_VERSION}-generic" "$KERNEL_MODULES_EXTRA_PKG_SHA256"

if [[ -n "$RUNTIME_INTERPRETER" ]]; then
    for package_spec in $GLIBC_RUNTIME_PACKAGES; do
        package_name="${package_spec%%=*}"
        expected_sha="$(expected_runtime_sha256 "$package_name" || true)"
        [[ -n "$expected_sha" ]] || die "missing checksum for ${package_name} in GLIBC_RUNTIME_PACKAGE_SHA256S"

        log_info "Verifying ${package_name} checksum"
        verify_package_sha256 "$package_name" "$expected_sha"
    done
fi

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

if [[ -n "$RUNTIME_INTERPRETER" ]]; then
    for package_spec in $GLIBC_RUNTIME_PACKAGES; do
        package_name="${package_spec%%=*}"
        package_deb="$(find_downloaded_deb "$package_name")"
        log_info "Extracting ${package_name}"
        dpkg-deb -x "$package_deb" "$EXTRACTED_DIR"
    done
fi
log_ok "Packages extracted"

# ==============================================================================
# SECTION 3: Build Initrd Structure
# ==============================================================================

log_section "Build Initrd Structure"
INITRD_DIR="$WORK_DIR/initrd"
mkdir -p "$INITRD_DIR"/{bin,dev,proc,sys,tmp,etc,lib/modules,lib64,usr/lib,mnt,run/cryptsetup}

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
# Both binaries are pre-built (by misc/AMDSEV/build-cryptsetup.sh) and
# supplied via $CRYPTSETUP_BINARY / $MKFS_EXT2_BINARY. They're fully static,
# so no .so files need to be vendored alongside them. cryptsetup unlocks the
# LUKS volume; mkfs.ext2 formats the decrypted mapper on first boot (Ubuntu's
# busybox-static does not include the mkfs.ext2 applet).
if [[ "$SEALED_STORAGE_BUILD" -eq 1 ]]; then
    log_info "Installing static cryptsetup from $CRYPTSETUP_BINARY"
    cp "$CRYPTSETUP_BINARY" bin/cryptsetup
    chmod +x bin/cryptsetup
    if ! bin/cryptsetup --version >/dev/null 2>&1; then
        die "Installed cryptsetup binary is not functional"
    fi
    log_ok "cryptsetup installed"

    log_info "Installing static mkfs.ext2 from $MKFS_EXT2_BINARY"
    cp "$MKFS_EXT2_BINARY" bin/mkfs.ext2
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
# Install Cartridge sidecar binaries (paymaster-service + vrf-server)
# ------------------------------------------------------------------------------
# Installed at /bin so katana's sidecar resolution finds them at the $PATH
# step (the init exports PATH=/bin) — the later resolution steps (katana home
# dir, interactive GitHub download) never run in the enclave.
if [[ -n "$PAYMASTER_BINARY" ]]; then
    log_info "Installing paymaster-service from $PAYMASTER_BINARY"
    cp "$PAYMASTER_BINARY" bin/paymaster-service
    chmod +x bin/paymaster-service
    log_ok "paymaster-service installed"
fi
if [[ -n "$VRF_BINARY" ]]; then
    log_info "Installing vrf-server from $VRF_BINARY"
    cp "$VRF_BINARY" bin/vrf-server
    chmod +x bin/vrf-server
    log_ok "vrf-server installed"
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
# Install QEMU fw_cfg Kernel Module
# ------------------------------------------------------------------------------
# Launch configuration is delivered through two host-supplied channels,
# deliberately OUTSIDE the SEV-SNP launch measurement (unlike the kernel
# cmdline). They differ only in transport, not trust posture:
#
#   1. fw_cfg (qemu_fw_cfg.ko, installed below) — exposes QEMU's fw_cfg
#      device at /sys/firmware/qemu_fw_cfg. Used for the Katana CLI args
#      (opt/org.katana/args, ~one line). Safe under SNP: the driver's read
#      path (fw_cfg_read_blob) is pure port I/O (ioread8_rep, no DMA),
#      which the #VC handler supports. Small payload, no perf issue.
#
#   2. virtio-blk (virtio_blk, built into vmlinuz — no .ko needed) — a
#      read-only virtio-blk ext2 disk packed by start-vm.sh from
#      --chain-dir. Used for the chain config dir (multi-MB genesis.json +
#      config.toml). fw_cfg is unusable here: cp(1) of a sysfs binary
#      attribute issues PAGE_SIZE reads, and the upstream qemu_fw_cfg
#      driver re-reads the whole blob per sysfs read() — so an 18 MB blob
#      costs O(N^2) port I/O and stalls the guest indefinitely under
#      SEV-SNP. virtio-blk uses DMA (via SWIOTLB bounce buffers in shared
#      memory under SNP) for linear throughput. The guest mounts the disk
#      read-only and Katana validates the contents via its chain spec
#      parser — same trust-but-validate posture as fw_cfg, just a
#      different transport.
#
# qemu_fw_cfg ships in linux-modules-extra (installed below). virtio_blk,
# virtio_pci, virtio_scsi, and ext4 (which also handles ext2) are all
# kernel-builtins in the Ubuntu 6.8.0-90 generic kernel
# (see /lib/modules/6.8.0-90-generic/modules.builtin) — no install needed.
log_info "Installing qemu_fw_cfg kernel module"
FIRMWARE_MODULES_DIR="$EXTRACTED_DIR/lib/modules/$KERNEL_VERSION-generic/kernel/drivers/firmware"
install_sev_module "qemu_fw_cfg.ko" "$FIRMWARE_MODULES_DIR/qemu_fw_cfg.ko" "lib/modules/qemu_fw_cfg.ko"

# ------------------------------------------------------------------------------
# Install Device-Mapper + dm-integrity transitive deps (sealed-storage only)
# ------------------------------------------------------------------------------
# Each module ships in `linux-modules-$KVER-generic` from Ubuntu noble. The
# minimal initrd has no kmod / udev / depmod / module-autoloader, so every
# module the LUKS2 + dm-integrity unseal path touches must be (a) present in
# /lib/modules/ and (b) insmod'd by the init script in dependency order.
#
# Module-by-module justification — keep this in sync with the `for mod in …`
# loop in `load_dm_modules` inside the init heredoc:
#
#   xor               Generic XOR primitive. Leaf dependency of async_xor —
#                     async ops use XOR for parity. Self-registers an
#                     optimal checksum implementation at insmod time
#                     ("xor: automatically using best checksumming
#                     function avx" in dmesg).
#
#   async_tx          Async crypto / xfer API. Leaf dependency of async_xor;
#                     prints "async_tx: api initialized" at insmod.
#
#   async_xor         Pulled in by dm-integrity (per depmod against
#                     linux-modules-6.8.0-90 amd64). dm-integrity uses it
#                     for parity-style operations on the integrity tag area.
#                     Without it, `insmod dm-integrity.ko` fails with
#                     `Unknown symbol async_xor`.
#
#   cryptd            Crypto daemon. Lets crypto algorithms run on a kernel
#                     workqueue when the SIMD context isn't available
#                     (interrupts, etc.). Leaf dependency of crypto_simd
#                     and aesni-intel.
#
#   crypto_simd       SIMD glue layer. Wraps SIMD-using algorithms so they
#                     fall back to cryptd when SIMD is unsafe. Required by
#                     aesni-intel.
#
#   aesni-intel       Hardware AES (AES-NI). cryptsetup with
#                     `--cipher aes-xts-plain64` makes dm-crypt request
#                     `aes-xts-plain64` from the kernel crypto API. The
#                     kernel-builtin `aes_generic` does NOT satisfy this
#                     when wrapped under authenc() for an AEAD chain — the
#                     authenc compose only matches AES providers that
#                     don't allocate memory in their hot path. AES-NI does;
#                     aes_generic doesn't. Without aesni-intel loaded, the
#                     dm-crypt table-load fails with:
#                         crypt: Error allocating crypto tfm (-ENOENT)
#                     AMD EPYC (the only CPUs that run SEV-SNP) always
#                     have AES-NI, so this module always loads.
#
#   sha256-ssse3      Hardware-accelerated SHA-256. Same rationale as
#                     aesni-intel for the `hmac(sha256)` half of the
#                     dm-integrity AEAD chain. AMD EPYC always has SSSE3.
#
#   authenc           AEAD-composer template. cryptsetup with
#                     `--integrity hmac-sha256` builds the cipher chain
#                         authenc(hmac(sha256), aes-xts-plain64)
#                     and dm-crypt asks the kernel crypto API for that
#                     compose. The `authenc` template lives in a separate
#                     loadable module (NOT builtin, NOT a transitive dep
#                     of anything else we ship), so without it dm-crypt's
#                     crypto_alloc_* returns -ENOENT even after every
#                     primitive above is loaded. `aead.ko` itself is
#                     already builtin — only the wrapper template needs
#                     shipping.
#
#   dm-bufio          Generic dm buffer cache. Leaf dependency of
#                     dm-integrity (which uses it to manage tag I/O).
#                     Without it, `insmod dm-integrity.ko` fails with
#                     `Unknown symbol dm_bufio_*`.
#
#   dm-crypt          The actual LUKS open/format target.
#
#   dm-integrity      Sector-level HMAC authentication layered under
#                     dm-crypt. Catches offline ciphertext tampering.
#
# `dm-mod` is NOT in this list: it's compiled into the Ubuntu 6.8 kernel
# (modules.builtin shows `kernel/drivers/md/dm-mod.ko` — that path means
# "would be a module if =m, but is =y"). It auto-initialises at boot.
#
# Updating this list when bumping the Ubuntu kernel pin: re-run depmod
# against the new linux-modules-$KVER-generic deb to verify the dm-bufio
# / async_xor chain hasn't shifted, and re-check `aead`/`authenc`/`xts`
# against modules.builtin in case Ubuntu flips one to =y.
if [[ "$SEALED_STORAGE_BUILD" -eq 1 ]]; then
    log_info "Installing device-mapper + dm-integrity transitive deps + crypto SIMD modules"
    KVER_ROOT="$EXTRACTED_DIR/lib/modules/$KERNEL_VERSION-generic"

    if [[ ! -d "$KVER_ROOT" ]]; then
        die "kernel modules root not found at $KVER_ROOT (sealed-storage build requires the linux-modules deb)"
    fi

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
# Runtime library helpers
# ------------------------------------------------------------------------------
elf_needed() {
    readelf -d "$1" 2>/dev/null |
        awk '/\(NEEDED\)/ { sub(/^.*Shared library: \[/, ""); sub(/\].*$/, ""); print }' || true
}

resolve_extracted_path() {
    local path="$1"
    local target
    local depth=0

    while [[ -L "$path" ]]; do
        if [[ $depth -ge 20 ]]; then
            die "too many symlink indirections while resolving $1"
        fi

        target="$(readlink "$path")"
        if [[ "$target" = /* ]]; then
            path="$EXTRACTED_DIR$target"
        else
            path="$(dirname "$path")/$target"
        fi
        depth=$((depth + 1))
    done

    [[ -e "$path" ]] || die "resolved package path does not exist: $path"
    printf '%s\n' "$path"
}

resolve_runtime_library() {
    local soname="$1"
    local roots=()
    local root
    local match=""

    for root in "$EXTRACTED_DIR/lib" "$EXTRACTED_DIR/usr/lib" "$EXTRACTED_DIR/lib64"; do
        [[ -d "$root" ]] && roots+=("$root")
    done
    [[ ${#roots[@]} -gt 0 ]] || return 1

    while IFS= read -r candidate; do
        if [[ -z "$match" ]]; then
            match="$candidate"
        elif [[ "$candidate" != "$match" ]]; then
            log_warn "Multiple runtime libraries named ${soname}; using ${match}"
            break
        fi
    done < <(find "${roots[@]}" \( -type f -o -type l \) -name "$soname" -print | LC_ALL=C sort)

    [[ -n "$match" ]] || return 1
    printf '%s\n' "$match"
}

install_dynamic_runtime() {
    local interpreter="$1"
    local interpreter_source=""
    local runtime_source=""
    local resolved_source=""
    local dest_rel=""
    local dest=""
    local queue_index=0
    local elf=""
    local needed=""
    local optional_lib=""

    declare -A copied_runtime=()
    # Seed the DT_NEEDED walk with every dynamic executable in the image
    # (katana + any bundled sidecars) — a lib needed only by a sidecar (e.g.
    # paymaster-service's libssl.so.3) must be copied too. Static roots are
    # harmless: elf_needed yields nothing for them.
    local elf_queue=()
    local root_rel
    for root_rel in "${DYNAMIC_ELF_ROOTS[@]}"; do
        elf_queue+=("$INITRD_DIR/$root_rel")
    done

    install_runtime_elf() {
        runtime_source="$1"
        dest_rel="$2"

        if [[ -n "${copied_runtime[$dest_rel]:-}" ]]; then
            return 0
        fi

        resolved_source="$(resolve_extracted_path "$runtime_source")"
        dest="$INITRD_DIR/$dest_rel"
        mkdir -p "$(dirname "$dest")"
        cp "$resolved_source" "$dest"
        chmod 0755 "$dest"

        copied_runtime["$dest_rel"]=1
        elf_queue+=("$dest")
        log_info "Installed runtime ELF /${dest_rel}"
    }

    install_runtime_library() {
        needed="$1"
        required="$2"

        runtime_source="$(resolve_runtime_library "$needed" || true)"
        if [[ -z "$runtime_source" ]]; then
            if [[ "$required" -eq 1 ]]; then
                die "runtime library not found in pinned packages: $needed"
            fi

            log_warn "Optional runtime library not found in pinned packages: $needed"
            return 0
        fi

        dest_rel="${runtime_source#"$EXTRACTED_DIR"/}"
        install_runtime_elf "$runtime_source" "$dest_rel"
    }

    log_info "Installing dynamic runtime"
    if [[ -e "$EXTRACTED_DIR$interpreter" ]]; then
        interpreter_source="$EXTRACTED_DIR$interpreter"
    else
        interpreter_source="$(resolve_runtime_library "$(basename "$interpreter")" || true)"
    fi
    [[ -n "$interpreter_source" ]] || die "ELF interpreter not found in pinned packages: $interpreter"
    install_runtime_elf "$interpreter_source" "${interpreter#/}"

    while [[ $queue_index -lt ${#elf_queue[@]} ]]; do
        elf="${elf_queue[$queue_index]}"
        queue_index=$((queue_index + 1))

        while IFS= read -r needed; do
            [[ -n "$needed" ]] || continue
            install_runtime_library "$needed" 1
        done < <(elf_needed "$elf")
    done

    for optional_lib in "${OPTIONAL_RUNTIME_LIBS[@]}"; do
        install_runtime_library "$optional_lib" 0
    done

    while [[ $queue_index -lt ${#elf_queue[@]} ]]; do
        elf="${elf_queue[$queue_index]}"
        queue_index=$((queue_index + 1))

        while IFS= read -r needed; do
            [[ -n "$needed" ]] || continue
            install_runtime_library "$needed" 1
        done < <(elf_needed "$elf")
    done

    log_ok "Dynamic runtime installed"
}

# ------------------------------------------------------------------------------
# Install Katana Binary
# ------------------------------------------------------------------------------
log_info "Installing Katana binary"
cp "$KATANA_BINARY" bin/katana
chmod +x bin/katana
log_ok "Katana installed"

DYNAMIC_ELF_ROOTS=(bin/katana)
[[ -n "$PAYMASTER_BINARY" ]] && DYNAMIC_ELF_ROOTS+=(bin/paymaster-service)
[[ -n "$VRF_BINARY" ]] && DYNAMIC_ELF_ROOTS+=(bin/vrf-server)

if [[ -n "$RUNTIME_INTERPRETER" ]]; then
    install_dynamic_runtime "$RUNTIME_INTERPRETER"

    # Record the actual glibc version from the installed libc.so.6. The package
    # version (e.g. 2.39-0ubuntu8.7) sits in GLIBC_RUNTIME_PACKAGES already; this
    # is the upstream glibc release the runtime exposes (e.g. 2.39).
    libc_so=""
    for candidate in "$INITRD_DIR"/usr/lib/*/libc.so.6 "$INITRD_DIR"/lib/*/libc.so.6 "$INITRD_DIR"/lib64/libc.so.6; do
        if [[ -f "$candidate" ]]; then
            libc_so="$candidate"
            break
        fi
    done
    if [[ -n "$libc_so" ]]; then
        glibc_version="$("$libc_so" 2>/dev/null | sed -n '1{s/.*release version \([0-9.]*\)\.$/\1/p;}' || true)"
        if [[ -n "$glibc_version" ]]; then
            log_info "Detected glibc version: $glibc_version"
            printf '%s\n' "$glibc_version" > "$OUTPUT_DIR/glibc-version.txt"
        else
            log_warn "Could not parse glibc version from $libc_so"
        fi
    fi
fi

# ------------------------------------------------------------------------------
# Create Init Script
# ------------------------------------------------------------------------------
log_info "Creating init script"
cat > init <<'INIT_EOF'
#!/bin/sh
# Katana TEE VM Init Script

set -eu
export PATH=/bin
export LD_LIBRARY_PATH=/lib/x86_64-linux-gnu:/usr/lib/x86_64-linux-gnu:/lib:/usr/lib
# SSL_CERT_FILE: belt-and-braces for OpenSSL-linked sidecars (the CA bundle is
# also installed at each stack's compiled-in default path). HOME: katana's
# sidecar resolver eagerly derives ~/.katana even when the $PATH lookup will
# hit; /etc/passwd already resolves root's home to / via nss_files — this
# export just removes the passwd-lookup dependency.
export SSL_CERT_FILE=/etc/ssl/certs/ca-certificates.crt
export HOME=/

# log writes to stderr so command substitution like `$(strip_reserved_args ...)`
# captures only the function's real output. Both stdout and stderr are
# redirected to /dev/console below, so operator UX is unchanged.
log() { echo "[init] $*" >&2; }

KATANA_PID=""
KATANA_DB_DIR="/mnt/data/katana-db"
SHUTTING_DOWN=0
KATANA_EXIT_CODE="never"
CONTROL_PORT_NAME="org.katana.control.0"
CONTROL_PORT_LINK="/dev/virtio-ports/org.katana.control.0"

# Host-supplied launch configuration, delivered via two channels (both NOT
# part of the SEV-SNP launch measurement — operator-supplied at boot and
# sanitized/validated before use):
#
#   1. fw_cfg (-fw_cfg in start-vm.sh) — read once at boot by
#      load_fw_cfg_args:
#        opt/org.katana/args        whitespace-separated Katana CLI args
#                                   (sanitized with strip_reserved_args)
#
#   2. virtio-blk readonly ext2 disk (-drive ...,readonly=on -device
#      virtio-blk-pci in start-vm.sh) — mounted by mount_chain_disk:
#        chain config dir          materialized at $KATANA_CHAIN_DIR and
#                                  passed to Katana via --chain
#
# A virtio-blk transport is required for the chain dir: the upstream
# qemu_fw_cfg driver re-reads the whole blob per sysfs read(), making
# port-I/O fw_cfg O(N^2) for multi-MB blobs and unusable under SEV-SNP
# (an 18 MB genesis takes >10 minutes).
FW_CFG_BY_NAME="/sys/firmware/qemu_fw_cfg/by_name"
FW_CFG_ARGS_RAW="${FW_CFG_BY_NAME}/opt/org.katana/args/raw"
# start-vm.sh attaches exactly one virtio-blk device when --chain-dir is set;
# the kernel exposes it as /dev/vda. Busybox init has no udev/mdev so
# /dev/disk/by-id/ symlinks aren't available — we use the bare /dev/vda path.
CHAIN_DISK="/dev/vda"
KATANA_CHAIN_DIR="/run/katana-chain"
KATANA_FWCFG_ARGS=""
KATANA_CHAIN_PRESENT=0

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
    # Order MUST match the DM_MODULES list in build-initrd.sh's "Install
    # Device-Mapper + dm-integrity transitive deps" section — that's where
    # the per-module justification lives. Briefly:
    #
    #   xor / async_tx / async_xor   leaf deps of dm-integrity
    #   cryptd / crypto_simd          leaf deps of aesni-intel
    #   aesni-intel                  required AES provider for dm-crypt
    #   sha256-ssse3                 required SHA256 provider for dm-integrity
    #   authenc                      required template for the AEAD chain
    #                                authenc(hmac(sha256), aes-xts-plain64)
    #   dm-bufio                     leaf dep of dm-integrity
    #   dm-crypt / dm-integrity      LUKS + integrity targets
    #
    # dm-mod is kernel-builtin in the Ubuntu 6.8 kernel and not insmod'd here.
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

# Drop flags that init owns from a space-separated arg string: --data-dir /
# --db-dir / --db-* (database location) and --chain (chain config dir). This
# prevents host-supplied launch config (fw_cfg) from pointing Katana at a
# data directory outside the sealed mount or at a chain spec other than the
# one init materialized from fw_cfg. Runs inside the measured initrd, so the
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
#           `--chain value`        (two tokens) / `--chain=value` (one token)
strip_reserved_args() {
    SKIP_NEXT=0
    OUT=""
    # Word-splitting the unquoted $* is the point: callers pass a flat
    # whitespace-separated arg string, not pre-split words. Disable pathname
    # expansion around the loop so glob characters in operator-supplied values
    # (most commonly the `*` in `--http.cors-origins *`) stay literal — without
    # `set -f` the shell would expand `*` to the contents of CWD, which inside
    # the initrd's busybox / rootfs is `bin dev etc init lib …` and ends up
    # being passed to Katana as a noisy list of fake origins.
    set -f
    # shellcheck disable=SC2048
    for tok in $*; do
        if [ "$SKIP_NEXT" -eq 1 ]; then
            SKIP_NEXT=0
            log "strip_reserved_args: dropped value token '$tok'"
            continue
        fi
        case "$tok" in
            --data-dir=*|--db-*=*|--chain=*)
                log "strip_reserved_args: dropped '$tok'"
                continue
                ;;
            --data-dir|--db-*|--chain)
                log "strip_reserved_args: dropped '$tok' (and next token)"
                SKIP_NEXT=1
                continue
                ;;
        esac
        OUT="${OUT}${OUT:+ }${tok}"
    done
    set +f
    echo "$OUT"
}

# Read host-supplied Katana CLI args from QEMU fw_cfg. Loads the
# qemu_fw_cfg module and reads the args entry. Small payload (one line of
# CLI flags) so port-I/O cost is negligible. Args are filtered with
# strip_reserved_args before use so the host can't override --data-dir /
# --db-* / --chain (the init owns those).
#
# Missing module / device / entry is non-fatal: Katana starts with only the
# flags init bakes in (--db-dir, --chain).
load_fw_cfg_args() {
    if [ ! -d "$FW_CFG_BY_NAME" ]; then
        if [ ! -f /lib/modules/qemu_fw_cfg.ko ]; then
            log "WARNING: qemu_fw_cfg.ko not in initrd; no fw_cfg launch args"
            return 0
        fi
        if ! /bin/insmod /lib/modules/qemu_fw_cfg.ko; then
            log "WARNING: insmod qemu_fw_cfg.ko failed; no fw_cfg launch args"
            return 0
        fi
    fi
    if [ ! -d "$FW_CFG_BY_NAME" ]; then
        log "WARNING: $FW_CFG_BY_NAME missing after module load; no fw_cfg launch args"
        return 0
    fi

    if [ -f "$FW_CFG_ARGS_RAW" ]; then
        RAW_FWCFG_ARGS="$(tr '\n\r\t' '   ' < "$FW_CFG_ARGS_RAW")"
        # Quote the raw string so the glob is NOT expanded here at the call
        # site; strip_reserved_args word-splits it internally under `set -f`.
        KATANA_FWCFG_ARGS="$(strip_reserved_args "$RAW_FWCFG_ARGS")"
        log "fw_cfg katana args: ${KATANA_FWCFG_ARGS:-<empty>}"
    else
        log "No fw_cfg args entry (opt/org.katana/args)"
    fi
}

# Mount the host-supplied chain config disk read-only at $KATANA_CHAIN_DIR.
# The disk is a small ext2 image packed by start-vm.sh from --chain-dir and
# attached as a virtio-blk device. virtio_blk + ext4 (which handles ext2)
# are kernel-builtins in the Ubuntu 6.8.0-90 generic kernel — no insmod
# needed; the device shows up automatically once the virtio bus is probed.
#
# Missing device is non-fatal — Katana starts without --chain.
mount_chain_disk() {
    # virtio-blk device probe is async — busybox init has no udev/mdev to
    # populate /dev/disk/by-id, so we poll the underlying /dev/vd* device
    # appearing after virtio bus enumeration. Up to ~5s.
    waited=0
    while [ "$waited" -lt 50 ]; do
        # The first virtio-blk device shows up as /dev/vda. start-vm.sh attaches
        # exactly one virtio-blk device when --chain-dir is provided.
        if [ -b "$CHAIN_DISK" ]; then
            break
        fi
        sleep 0.1
        waited=$((waited + 1))
    done
    if [ ! -b "$CHAIN_DISK" ]; then
        log "No $CHAIN_DISK after virtio_blk load — no chain config disk attached"
        return 0
    fi

    mkdir -p "$KATANA_CHAIN_DIR"
    # ext2 is supported by the kernel for the sealed-mount path already
    # (mkfs.ext2 + mount -t ext2 on /dev/mapper/...), so we know it's loaded.
    if ! /bin/mount -t ext2 -o ro "$CHAIN_DISK" "$KATANA_CHAIN_DIR" 2>/dev/null; then
        log "WARNING: failed to mount $CHAIN_DISK at $KATANA_CHAIN_DIR — no chain config"
        return 0
    fi

    chain_files=$(ls -1 "$KATANA_CHAIN_DIR" 2>/dev/null | tr '\n' ' ')
    log "Chain config disk mounted at $KATANA_CHAIN_DIR (files: ${chain_files:-<empty>})"

    # Only thread --chain to Katana if the dir contains a config.toml. Katana's
    # rollup::read(dir) opens dir/config.toml + dir/genesis.json — without
    # config.toml it errors out and exits. A mount without config.toml is
    # operationally a misconfigured deploy, but treating it as "boot without
    # --chain" lets the operator still get in and fix it. The test-initrd
    # regression test relies on this: it attaches a chain disk with only
    # synthetic padding (no katana-version-compatible chain spec) to exercise
    # the mount path independently of katana's chain-spec schema.
    if [ ! -f "$KATANA_CHAIN_DIR/config.toml" ]; then
        log "WARNING: $KATANA_CHAIN_DIR has no config.toml — Katana will boot without --chain"
        return 0
    fi
    KATANA_CHAIN_PRESENT=1
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

            # Launch configuration comes exclusively from boot-time channels
            # (fw_cfg args + virtio-blk chain disk), read once by init. A
            # payload here means a stale client speaking the old
            # `start <csv-args>` protocol — reject loudly rather than
            # silently ignoring its args.
            if [ -n "$CMD_PAYLOAD" ]; then
                respond_control "err start-takes-no-args (launch config comes from boot-time fw_cfg / chain disk)"
                return 0
            fi

            CHAIN_ARGS=""
            if [ "$KATANA_CHAIN_PRESENT" -eq 1 ]; then
                CHAIN_ARGS="--chain $KATANA_CHAIN_DIR"
            fi

            log "Starting katana asynchronously..."
            # Close FD 3 (the control-channel virtio-serial port) in the
            # backgrounded child. Otherwise katana inherits it and pins the
            # underlying /dev/virtio-ports/* device, so when the outer loop
            # tries to re-open FD 3 after the host disconnects it fails with
            # EBUSY — and a failed `exec` redirect under POSIX `set -e`
            # terminates init, panicking the kernel.
            # Disable pathname expansion around the unquoted arg split: the
            # fw_cfg args must word-split into argv, but a glob in an operator
            # value (e.g. `--http.cors-origins *`) must reach Katana literally,
            # not expand against the rootfs.
            set -f
            # shellcheck disable=SC2086
            /bin/katana --db-dir="$KATANA_DB_DIR" $CHAIN_ARGS $KATANA_FWCFG_ARGS 3<&- 3>&- &
            set +f
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

        stop)
            # Graceful shutdown: without this, the only way to stop the VM
            # is a host-side power cut (QEMU SIGTERM never reaches init, and
            # busybox init has no ACPI handling), which can tear database
            # state still sitting in the guest page cache. Acknowledge
            # FIRST — teardown closes the control channel — then run the
            # same teardown the TERM trap uses: stop katana (TERM, then
            # KILL), sync, unmount /mnt/data, luksClose, poweroff. Never
            # returns; QEMU exits on the guest poweroff.
            respond_control "ok stopping"
            teardown_and_halt
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

# SP1 core executor uses POSIX shared memory (shm_open) for the guest memory image;
# without /dev/shm it panics with "create shm file for memory: NotFound".
mkdir -p /dev/shm
/bin/mount -t tmpfs tmpfs /dev/shm 2>/dev/null || log "WARNING: failed to mount /dev/shm"

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

# Read host-supplied launch config:
#   - CLI args from fw_cfg (small, fast)
#   - chain config dir from a virtio-blk readonly disk (DMA-capable; required
#     for multi-MB blobs that fw_cfg port I/O can't handle, see header comment)
log "Loading launch config args from fw_cfg..."
load_fw_cfg_args
log "Mounting chain config disk (virtio-blk)..."
mount_chain_disk

# Configure networking (QEMU user-mode defaults)
log "Configuring network..."
/bin/ip link set lo up 2>/dev/null || true
if [ -d /sys/class/net/eth0 ]; then
    /bin/ip link set eth0 up 2>/dev/null || true
    /bin/ip addr add 10.0.2.15/24 dev eth0 2>/dev/null || true
    /bin/ip route add default via 10.0.2.2 2>/dev/null || true
    printf "nameserver 1.1.1.1\\nnameserver 8.8.8.8\\nnameserver 10.0.2.3\\n" > /etc/resolv.conf
    # qemu user-net has no IPv6 route; prefer IPv4 so dual-stack hosts (e.g. api.cartridge.gg) connect
    echo "precedence ::ffff:0:0/96  100" > /etc/gai.conf
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
        # Distinguish the most common operator error — a sealed (LUKS) disk
        # booted in unsealed mode — from a genuinely unformatted/corrupt disk,
        # so the serial log names the real fix instead of a bare mount failure.
        # A LUKS2 header begins with the ASCII magic "LUKS" at offset 0; read
        # just those bytes (no blkid/cryptsetup in the unsealed build) and match.
        if /bin/dd if=/dev/sda bs=4 count=1 2>/dev/null | /bin/grep -aq 'LUKS'; then
            log "ERROR: /dev/sda holds a LUKS-encrypted (sealed) disk, but this VM"
            log "       booted in UNSEALED mode (no KATANA_EXPECTED_LUKS_UUID on the"
            log "       measured kernel cmdline) — storage-mode mismatch."
            log "       Fix: boot with --sealed (under the same build that sealed the"
            log "       disk), or provision a fresh disk (delete the old data.img)."
            fatal_boot "mount /dev/sda failed: sealed (LUKS) disk booted in unsealed mode"
        fi
        fatal_boot "failed to mount /dev/sda as ext4 (unsealed) — disk is unformatted or corrupt"
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
cat > etc/hosts <<'HOSTS_EOF'
127.0.0.1 localhost
::1 localhost
HOSTS_EOF
cat > etc/nsswitch.conf <<'NSS_EOF'
passwd: files
group: files
shadow: files
hosts: files dns
networks: files
protocols: files
services: files
ethers: files
rpc: files
NSS_EOF
log_ok "/etc files created"

# ------------------------------------------------------------------------------
# Install CA certificates (both TLS trust stores)
# ------------------------------------------------------------------------------
# Embedded settlement makes outbound HTTPS (settlement RPC, the SP1 prover network,
# AMD KDS) and katana uses TWO TLS stacks with DIFFERENT trust-store paths, so the
# bundle must land in both or settlement fails:
#   - AMD KDS                       -> vendored openssl (OPENSSLDIR=/usr/local/ssl)
#                                      reads /usr/local/ssl/cert.pem
#   - Starknet settlement RPC + L1<->L2 messaging -> jsonrpsee/rustls
#                                      (rustls-native-certs) probes /etc/ssl/certs/
#                                      ca-certificates.crt, /etc/ssl/cert.pem,
#                                      /etc/pki/tls/certs/ca-bundle.crt
# With the bundle only at the openssl path, KDS works but every registry/messaging
# TLS handshake fails (rustls finds no trust anchors) -> surfaces as
# "client error (Connect)" -> settlement never lands. Pinned + checksum-verified
# like every other package; SECTION 4 normalizes mode/timestamps for reproducibility.
if [[ -n "${CA_CERTIFICATES_PKG_VERSION:-}" ]]; then
    : "${CA_CERTIFICATES_PKG_SHA256:?CA_CERTIFICATES_PKG_SHA256 required when CA_CERTIFICATES_PKG_VERSION is set}"
    log_info "Downloading ca-certificates=${CA_CERTIFICATES_PKG_VERSION}"
    CA_WORK="$WORK_DIR/ca-certificates"
    mkdir -p "$CA_WORK"
    ( cd "$CA_WORK" && apt-get download "ca-certificates=${CA_CERTIFICATES_PKG_VERSION}" )
    CA_DEB="$(find "$CA_WORK" -maxdepth 1 -name 'ca-certificates_*.deb' | head -1)"
    [[ -f "$CA_DEB" ]] || die "ca-certificates .deb not downloaded"
    CA_ACTUAL="$(sha256sum "$CA_DEB" | awk '{print $1}')"
    [[ "$CA_ACTUAL" == "$CA_CERTIFICATES_PKG_SHA256" ]] || \
        die "ca-certificates checksum mismatch (expected $CA_CERTIFICATES_PKG_SHA256, got $CA_ACTUAL)"
    log_ok "ca-certificates checksum verified"
    dpkg-deb -x "$CA_DEB" "$CA_WORK/extracted"
    # Assemble the trusted Mozilla roots in a deterministic (LC_ALL=C sorted) order.
    mkdir -p usr/local/ssl
    find "$CA_WORK/extracted/usr/share/ca-certificates" -name '*.crt' | LC_ALL=C sort | \
        xargs cat > usr/local/ssl/cert.pem
    CA_N="$(grep -c 'BEGIN CERTIFICATE' usr/local/ssl/cert.pem || true)"
    [[ "$CA_N" -ge 100 ]] || die "assembled CA bundle has too few roots ($CA_N)"
    # Same bundle at the rustls-native-certs probe paths (see comment above).
    mkdir -p etc/ssl/certs etc/pki/tls/certs
    cp usr/local/ssl/cert.pem etc/ssl/certs/ca-certificates.crt   # rustls-native-certs (primary)
    cp usr/local/ssl/cert.pem etc/ssl/cert.pem                    # rustls/alpine fallback
    cp usr/local/ssl/cert.pem etc/pki/tls/certs/ca-bundle.crt     # rustls/RHEL fallback
    # Ubuntu-built libssl3 consumers (the paymaster-service sidecar): the
    # system OpenSSL's compiled-in OPENSSLDIR is /usr/lib/ssl, so its default
    # cert file is /usr/lib/ssl/cert.pem. (The /usr/local/ssl path above
    # serves katana's VENDORED openssl only.)
    mkdir -p usr/lib/ssl
    cp usr/local/ssl/cert.pem usr/lib/ssl/cert.pem
    log_ok "CA bundle installed (vendored openssl + rustls + system openssl paths, $CA_N roots)"
else
    log_warn "CA_CERTIFICATES_PKG_VERSION not set — enclave will have NO CA bundle (outbound HTTPS will fail)"
fi

# ==============================================================================
# SECTION 4: Create CPIO Archive
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
[[ -n "$PAYMASTER_BINARY" ]] && chmod 0755 bin/paymaster-service
[[ -n "$VRF_BINARY" ]] && chmod 0755 bin/vrf-server
if [[ "$SEALED_STORAGE_BUILD" -eq 1 ]]; then
    chmod 0755 bin/cryptsetup bin/mkfs.ext2
    [[ -n "$SNP_DERIVEKEY_BINARY" ]] && chmod 0755 bin/snp-derivekey
fi
# The ELF interpreter is execve'd by the kernel when launching katana (and
# the sidecars), so it must keep its execute bit through the 0644 sweep above
# — a non-executable interpreter makes every dynamic exec fail with EACCES
# ("Permission denied", exit 126). Shared libraries stay 0644: the loader
# only opens and mmaps them, which needs read permission, not execute.
if [[ -n "$RUNTIME_INTERPRETER" ]]; then
    chmod 0755 "${RUNTIME_INTERPRETER#/}"
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
# SECTION 5: Final Validation
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
