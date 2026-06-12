#!/bin/bash
# ==============================================================================
# VERIFY-BUILD.SH - Verify a Katana TEE VM build / release
# ==============================================================================
#
# Checks that the build artifacts in OUTPUT_DIR match the values recorded in
# build-info.txt, then recomputes the SEV-SNP sealed launch measurement and
# compares it to LAUNCH_MEASUREMENT in build-info.txt. Exits non-zero on any
# mismatch.
#
# Usage:
#   ./verify-build.sh [OUTPUT_DIR]
#
# Default OUTPUT_DIR is ./output/qemu (build.sh's default). To verify a
# downloaded release, extract the katana-tee-vm-<tag>.tar.gz tarball and
# point this script at the extracted directory.
#
# Requirements:
#   - sha256sum
#   - snp-digest (on PATH or at snp-tools/target/release/snp-digest)
#     Build with: cargo build -p snp-tools --release
#
# ==============================================================================

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUTPUT_DIR="${1:-${SCRIPT_DIR}/output/qemu}"

if [[ ! -d "$OUTPUT_DIR" ]]; then
    echo "ERROR: Output directory not found: $OUTPUT_DIR"
    echo ""
    echo "Usage: $0 [OUTPUT_DIR]"
    echo "  Default: ${SCRIPT_DIR}/output/qemu"
    exit 1
fi

BUILD_INFO="$OUTPUT_DIR/build-info.txt"
if [[ ! -f "$BUILD_INFO" ]]; then
    echo "ERROR: build-info.txt not found at $BUILD_INFO" >&2
    exit 1
fi

echo "=========================================="
echo "Katana TEE VM Build Verification"
echo "=========================================="
echo "Output directory: $OUTPUT_DIR"
echo "Date:             $(date -u +%Y-%m-%dT%H:%M:%SZ)"
echo ""

# Read a single value from build-info.txt by key.
info_get() {
    local key="$1"
    awk -F= -v k="$key" '$1 == k { sub(/^[^=]*=/, ""); print; exit }' "$BUILD_INFO"
}

FAILURES=0

# ---- Artifact checksum verification ----------------------------------------

echo "Artifact checksums (SHA256):"
echo "------------------------------------------------------------"

verify_checksum() {
    local file="$1"
    local info_key="$2"
    local path="$OUTPUT_DIR/$file"

    if [[ ! -f "$path" ]]; then
        printf "  %-14s [MISSING]\n" "$file"
        FAILURES=$((FAILURES + 1))
        return
    fi

    local computed expected
    computed=$(sha256sum "$path" | awk '{print $1}')
    expected=$(info_get "$info_key")

    if [[ -z "$expected" ]]; then
        printf "  %-14s %s [no recorded sha256]\n" "$file" "$computed"
        return
    fi

    if [[ "$computed" == "$expected" ]]; then
        printf "  %-14s %s [OK]\n" "$file" "$computed"
    else
        printf "  %-14s [MISMATCH]\n" "$file"
        printf "  %-14s   computed: %s\n" "" "$computed"
        printf "  %-14s   recorded: %s\n" "" "$expected"
        FAILURES=$((FAILURES + 1))
    fi
}

verify_checksum "OVMF.fd"    "OVMF_SHA256"
verify_checksum "vmlinuz"    "KERNEL_SHA256"
verify_checksum "initrd.img" "INITRD_SHA256"
verify_checksum "katana"     "KATANA_BINARY_SHA256"

echo ""

# ---- Launch measurement verification ---------------------------------------

LUKS_UUID="$(info_get LUKS_UUID)"
EXPECTED_MEASUREMENT="$(info_get LAUNCH_MEASUREMENT)"

if [[ -z "$LUKS_UUID" || -z "$EXPECTED_MEASUREMENT" ]]; then
    echo "Launch measurement: [SKIPPED — LUKS_UUID or LAUNCH_MEASUREMENT not recorded in build-info.txt]"
    echo ""
else
    echo "Launch measurement:"
    echo "------------------------------------------------------------"
    echo "  LUKS_UUID:        $LUKS_UUID"

    # Locate snp-digest. Check PATH first, then any of cargo's possible output
    # paths under snp-tools/target/ (snp-tools/.cargo/config.toml may pin a
    # specific target triple, so the binary may live under a target-specific
    # subdirectory rather than directly under target/release/).
    SNP_DIGEST=""
    if command -v snp-digest >/dev/null 2>&1; then
        SNP_DIGEST="$(command -v snp-digest)"
    else
        SNP_DIGEST="$(find "${SCRIPT_DIR}/snp-tools/target" \
            -type f -name snp-digest -perm -u+x 2>/dev/null | head -n1)"
    fi
    if [[ -z "$SNP_DIGEST" ]]; then
        echo "ERROR: snp-digest not found." >&2
        echo "  Build with: (cd snp-tools && cargo build --release)" >&2
        echo "  Then re-run this script." >&2
        exit 1
    fi

    # shellcheck source=scripts/sealed-cmdline.sh
    . "${SCRIPT_DIR}/scripts/sealed-cmdline.sh"
    CMDLINE="$(build_sealed_cmdline "$LUKS_UUID")"
    COMPUTED_MEASUREMENT=$("$SNP_DIGEST" \
        --ovmf="$OUTPUT_DIR/OVMF.fd" \
        --kernel="$OUTPUT_DIR/vmlinuz" \
        --initrd="$OUTPUT_DIR/initrd.img" \
        --append="$CMDLINE" \
        --vcpus=1 --cpu=epyc-v4 --vmm=qemu --guest-features=0x1)

    echo "  computed:         $COMPUTED_MEASUREMENT"
    echo "  recorded:         $EXPECTED_MEASUREMENT"

    if [[ "$COMPUTED_MEASUREMENT" == "$EXPECTED_MEASUREMENT" ]]; then
        echo "  result:           [OK]"
    else
        echo "  result:           [MISMATCH]"
        FAILURES=$((FAILURES + 1))
    fi
    echo ""
fi

# ---- Summary ---------------------------------------------------------------

echo "=========================================="
if (( FAILURES == 0 )); then
    echo "Verification: PASS"
    echo "=========================================="
    exit 0
else
    echo "Verification: FAIL ($FAILURES issue(s))"
    echo "=========================================="
    exit 1
fi
