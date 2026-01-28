#!/bin/bash
# ==============================================================================
# BUILD-OVMF.SH - Build AMD SEV-SNP OVMF firmware
# ==============================================================================
#
# Builds OVMF from AMD's fork with SEV-SNP support.
#
# Usage:
#   ./build-ovmf.sh OUTPUT_DIR
#
# Environment (required):
#   OVMF_GIT_URL   Git URL for OVMF repository
#   OVMF_BRANCH    Git branch to build
#
# ==============================================================================

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

function usage() {
    echo "Usage: $0 OUTPUT_DIR"
    echo ""
    echo "Build AMD SEV-SNP OVMF firmware."
    echo ""
    echo "ARGUMENTS:"
    echo "  OUTPUT_DIR    Directory to store built OVMF.fd"
    echo ""
    echo "ENVIRONMENT VARIABLES (or source build-config):"
    echo "  OVMF_GIT_URL  Git URL for OVMF repository"
    echo "  OVMF_BRANCH   Git branch to build"
    echo ""
    echo "EXAMPLES:"
    echo "  source build-config && $0 ./output"
    exit 1
}

run_cmd() {
    echo "$*"
    eval "$*" || {
        echo "ERROR: $*"
        exit 1
    }
}

if [[ $# -lt 1 ]] || [[ "${1:-}" == "-h" ]] || [[ "${1:-}" == "--help" ]]; then
    usage
fi

DEST="$1"

# Validate required environment variables
: "${OVMF_GIT_URL:?OVMF_GIT_URL not set - source build-config first}"
: "${OVMF_BRANCH:?OVMF_BRANCH not set - source build-config first}"

echo "=========================================="
echo "Building OVMF"
echo "=========================================="
echo "Configuration:"
echo "  Output dir:    $DEST"
echo "  Git URL:       $OVMF_GIT_URL"
echo "  Branch:        $OVMF_BRANCH"
echo "=========================================="
echo ""

# Determine GCC version for EDK2
GCC_VERSION=$(gcc -v 2>&1 | tail -1 | awk '{print $3}')
GCC_MAJOR=$(echo $GCC_VERSION | awk -F . '{print $1}')
GCC_MINOR=$(echo $GCC_VERSION | awk -F . '{print $2}')
if [ "$GCC_MAJOR" == "4" ]; then
    GCCVERS="GCC${GCC_MAJOR}${GCC_MINOR}"
else
    GCCVERS="GCC5"
fi

BUILD_CMD="nice build -q --cmd-len=64436 -DDEBUG_ON_SERIAL_PORT=TRUE -n $(getconf _NPROCESSORS_ONLN) ${GCCVERS:+-t $GCCVERS} -a X64 -p OvmfPkg/AmdSev/AmdSevX64.dsc"

# Clone or update OVMF repository
OVMF_DIR="${SCRIPT_DIR}/ovmf"
if [ -d "$OVMF_DIR" ]; then
    pushd "$OVMF_DIR" >/dev/null
    if git remote get-url current 2>/dev/null; then
        run_cmd git remote set-url current ${OVMF_GIT_URL}
    else
        run_cmd git remote add current ${OVMF_GIT_URL}
    fi
    popd >/dev/null
else
    run_cmd git clone --single-branch -b ${OVMF_BRANCH} ${OVMF_GIT_URL} "$OVMF_DIR"
    pushd "$OVMF_DIR" >/dev/null
    run_cmd git remote add current ${OVMF_GIT_URL}
    popd >/dev/null
fi

# Build OVMF
pushd "$OVMF_DIR" >/dev/null
    run_cmd git fetch current
    run_cmd git checkout current/${OVMF_BRANCH}
    run_cmd git submodule update --init --recursive
    run_cmd touch OvmfPkg/AmdSev/Grub/grub.efi # https://github.com/AMDESE/ovmf/issues/6#issuecomment-2843109558
    run_cmd make -C BaseTools clean
    run_cmd make -C BaseTools -j $(getconf _NPROCESSORS_ONLN)
    # Temporarily disable strict mode for edksetup.sh (has unbound variables)
    set +u
    . ./edksetup.sh --reconfig
    set -u
    run_cmd $BUILD_CMD

    mkdir -p "$DEST"
    run_cmd cp -f Build/AmdSev/DEBUG_$GCCVERS/FV/OVMF.fd $DEST

    COMMIT=$(git log --format="%h" -1 HEAD)
    echo "$COMMIT" > "${SCRIPT_DIR}/source-commit.ovmf"
popd >/dev/null

echo ""
echo "=========================================="
echo "[OK] OVMF build complete"
echo "=========================================="
echo "Output: $DEST/OVMF.fd"
echo "Commit: $COMMIT"
echo "SHA256: $(sha256sum "$DEST/OVMF.fd" | awk '{print $1}')"
echo "=========================================="
