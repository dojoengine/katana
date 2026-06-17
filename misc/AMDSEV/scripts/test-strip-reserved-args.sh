#!/bin/sh
# ==============================================================================
# TEST-STRIP-RESERVED-ARGS.SH - Unit tests for the guest init's
# strip_reserved_args sanitizer.
# ==============================================================================
#
# strip_reserved_args lives in the init heredoc inside build-initrd.sh and
# runs in the guest under busybox sh. It word-splits host-supplied Katana CLI
# args (intentional) while filtering the flags init owns (--db-* / --data-dir
# / --chain). The word-split loop must NOT also pathname-expand glob
# characters in operator values — most importantly the `*` in
# `--http.cors-origins *`, which would otherwise expand against the initrd
# rootfs CWD into `bin dev etc init lib …` and reach Katana as a list of
# fake origins.
#
# This extracts the function from build-initrd.sh and exercises it directly —
# fast, no QEMU. It is the precise regression test for that glob bug; the
# boot smoke test (test-initrd.sh) covers the same path end to end but slowly
# and only indirectly (via an RPC-liveness timeout).
#
# Run from anywhere; exits non-zero on any failed assertion.
# ==============================================================================

set -eu

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
BUILD_INITRD="${SCRIPT_DIR}/build-initrd.sh"
[ -f "$BUILD_INITRD" ] || { echo "build-initrd.sh not found at $BUILD_INITRD" >&2; exit 1; }

FN="$(mktemp)"
GLOBDIR="$(mktemp -d)"
trap 'rm -f "$FN"; rm -rf "$GLOBDIR"' EXIT

# Extract just the strip_reserved_args function definition (from its header
# to the first line that is a lone closing brace).
awk '/^strip_reserved_args\(\)/{f=1} f{print} f && /^}$/{exit}' "$BUILD_INITRD" > "$FN"
grep -q '^strip_reserved_args()' "$FN" || { echo "failed to extract strip_reserved_args from $BUILD_INITRD" >&2; exit 1; }

# The function logs to stderr via log(); the real one targets the console.
log() { :; }
# shellcheck disable=SC1090  # sourcing the extracted function by path
. "$FN"

# Populate a directory so a leaked unquoted glob would expand to filenames,
# reproducing the rootfs-CWD expansion the bug caused.
touch "$GLOBDIR/bin" "$GLOBDIR/dev" "$GLOBDIR/etc"

FAILS=0
check() {
    # $1 description, $2 expected, $3 actual
    if [ "$3" = "$2" ]; then
        echo "ok   - $1"
    else
        echo "FAIL - $1"
        echo "         want: [$2]"
        echo "         got:  [$3]"
        FAILS=$((FAILS + 1))
    fi
}

# Invoke the sanitizer from inside the populated directory.
run() { ( cd "$GLOBDIR" && strip_reserved_args "$@" ); }

# Regression: a glob in an operator value must survive literally.
check "glob '*' stays literal" \
    "--http.cors-origins *" "$(run "--http.cors-origins *")"

# Reserved flags are still stripped (two-token form).
check "--db-dir and its value are stripped" \
    "--http.port 5050" "$(run "--db-dir /mnt/evil --http.port 5050")"

check "--chain and its value are stripped" \
    "--tee sev-snp" "$(run "--chain /mnt/evil --tee sev-snp")"

# Single-token --flag=value form is stripped.
check "--data-dir=VALUE is stripped" \
    "--http.port 5050" "$(run "--data-dir=/mnt/evil --http.port 5050")"

# Combined: a reserved flag is stripped while a glob value is preserved.
check "reserved stripped and glob preserved together" \
    "--http.cors-origins *" "$(run "--db-dir /mnt/evil --http.cors-origins *")"

if [ "$FAILS" -ne 0 ]; then
    echo "strip_reserved_args: ${FAILS} assertion(s) failed" >&2
    exit 1
fi
echo "strip_reserved_args: all assertions passed"
