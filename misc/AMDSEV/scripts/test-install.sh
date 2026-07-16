#!/bin/bash
# ==============================================================================
# TEST-INSTALL.SH - Unit tests for install.sh's helper functions.
# ==============================================================================
#
# install.sh is the operator-facing entry point for standing up a TEE VM host
# from a published release. Its download/verify/wizard flow composes small
# helpers (validators, tag encoding, checksum gates, config round-trip,
# launcher feature detection); this exercises them directly — fast, no QEMU,
# no network — the same pattern as test-parse-metrics-port.sh. install.sh
# exposes them via the KATANA_INSTALL_LIB=1 source guard.
#
# An optional END-TO-END DRY RUN against a real published release (network,
# ~hundreds of MB) is gated behind KATANA_INSTALL_TEST_NETWORK=1 — too heavy
# for every CI run, but the full download+verify path in one command.
#
# Run from anywhere; exits non-zero on any failed assertion.
# ==============================================================================

# SC2034: many globals here are consumed by functions sourced out of
# install.sh (write_config/write_run_sh), which shellcheck can't see through
# the KATANA_INSTALL_LIB source indirection.
# shellcheck disable=SC2034

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
INSTALL_SH="${SCRIPT_DIR}/../install.sh"
START_VM="${SCRIPT_DIR}/../start-vm.sh"
[[ -f "$INSTALL_SH" ]] || { echo "install.sh not found at $INSTALL_SH" >&2; exit 1; }
[[ -f "$START_VM" ]] || { echo "start-vm.sh not found at $START_VM" >&2; exit 1; }

WORK="$(mktemp -d /tmp/katana-test-install.XXXXXX)"
trap 'rm -rf "$WORK"' EXIT

# Source install.sh as a library (defines helpers, skips main). It sets
# set -e; undo that so failed assertions report instead of aborting.
# shellcheck disable=SC1090
KATANA_INSTALL_LIB=1 . "$INSTALL_SH"
set +e +o pipefail

FAILS=0
check() {
    # $1 description, $2 expected, $3 actual
    if [[ "$3" == "$2" ]]; then
        echo "ok   - $1"
    else
        echo "FAIL - $1"
        echo "         want: [$2]"
        echo "         got:  [$3]"
        FAILS=$((FAILS + 1))
    fi
}
check_true() {
    # $1 description, then the command to run
    local desc="$1"; shift
    if "$@"; then
        echo "ok   - $desc"
    else
        echo "FAIL - $desc"
        FAILS=$((FAILS + 1))
    fi
}
check_false() {
    local desc="$1"; shift
    if "$@"; then
        echo "FAIL - $desc"
        FAILS=$((FAILS + 1))
    else
        echo "ok   - $desc"
    fi
}

# ---- Validators --------------------------------------------------------------

check_true  "valid_port accepts 15051"        valid_port 15051
check_false "valid_port rejects 0"            valid_port 0
check_false "valid_port rejects 65536"        valid_port 65536
check_false "valid_port rejects non-numeric"  valid_port abc

check_true  "valid_vcpus accepts 4"           valid_vcpus 4
check_false "valid_vcpus rejects 0"           valid_vcpus 0
check_false "valid_vcpus rejects 2.5"         valid_vcpus 2.5

# Overcommit (vCPUs > host cores) is deliberately allowed — the host core
# count is informational (shown in the prompt, warned past) — so plain
# valid_vcpus is the only gate and it must not encode an upper bound.
check_true  "valid_vcpus allows overcommit (no upper bound)" valid_vcpus 1024

check_true  "valid_memory accepts 4G"         valid_memory 4G
check_true  "valid_memory accepts 2048M"      valid_memory 2048M
check_false "valid_memory rejects 4"          valid_memory 4
check_false "valid_memory rejects 4g"         valid_memory 4g
check_false "valid_memory rejects 4GB"        valid_memory 4GB

check_true  "valid_tag accepts tee-vm-v0.1.0+katana-v1.8.0" valid_tag "tee-vm-v0.1.0+katana-v1.8.0"
check_false "valid_tag rejects a katana release tag"        valid_tag "v1.8.0"

check_true  "valid_uuid accepts canonical lowercase" valid_uuid "00000000-0000-0000-0000-000000000001"
check_false "valid_uuid rejects uppercase"           valid_uuid "00000000-0000-0000-0000-00000000000A"

# Gates the pinned prebuilt snp-digest download (SNP_DIGEST_SHA256).
check_true  "valid_sha256 accepts 64 hex"      valid_sha256 "$(printf 'a%.0s' {1..64})"
check_false "valid_sha256 rejects 63 hex"      valid_sha256 "$(printf 'a%.0s' {1..63})"
check_false "valid_sha256 rejects uppercase"   valid_sha256 "$(printf 'A%.0s' {1..64})"
check_false "valid_sha256 rejects a 404 page"  valid_sha256 "Not Found"

# ---- build-config pin extraction -----------------------------------------------
# install.sh reads the SNP_DIGEST_RELEASE/SNP_DIGEST_SHA256 pins out of the
# fetched tag's build-config without sourcing it (data read, not code exec).

BC_FIXTURE="$WORK/build-config"
cat > "$BC_FIXTURE" <<'EOF'
# comment
OVMF_COMMIT="fbe0805b"
SNP_DIGEST_RELEASE="snp-tools-v0.1.0"
SNP_DIGEST_SHA256=""
EOF
check "build_config_get reads a pinned value" \
    "snp-tools-v0.1.0" "$(build_config_get "$BC_FIXTURE" SNP_DIGEST_RELEASE)"
check "build_config_get reads an empty pin as empty" \
    "" "$(build_config_get "$BC_FIXTURE" SNP_DIGEST_SHA256)"
check "build_config_get returns nothing for a missing key" \
    "" "$(build_config_get "$BC_FIXTURE" NO_SUCH_KEY)"
# The real build-config must stay extractable by this exact parser.
check_true "real build-config carries the SNP_DIGEST_RELEASE key" \
    grep -q '^SNP_DIGEST_RELEASE="' "${SCRIPT_DIR}/../build-config"

# ---- Tag URL encoding ----------------------------------------------------------

check "encode_tag percent-encodes '+'" \
    "tee-vm-v0.1.0%2Bkatana-v1.8.0-rc.5" \
    "$(encode_tag "tee-vm-v0.1.0+katana-v1.8.0-rc.5")"
check "encode_tag is a no-op without '+'" \
    "tee-vm-v0.1.0" "$(encode_tag "tee-vm-v0.1.0")"

# ---- parse_metrics_port stays in sync with start-vm.sh ------------------------
# install.sh carries a copy (it needs the function before a release's
# start-vm.sh has been fetched). Extract the original and compare behavior.

ORIG_FN="$WORK/orig-parse.sh"
awk '/^parse_metrics_port\(\)/{f=1} f{print} f && /^}$/{exit}' "$START_VM" \
    | sed 's/^parse_metrics_port()/orig_parse_metrics_port()/' > "$ORIG_FN"
# shellcheck disable=SC1090
. "$ORIG_FN"
if ! declare -f orig_parse_metrics_port >/dev/null; then
    echo "FAIL - could not extract parse_metrics_port from start-vm.sh" >&2
    FAILS=$((FAILS + 1))
else
    for csv in \
        "--http.addr,0.0.0.0,--http.port,5050,--tee,sev-snp,--metrics,--metrics.addr,0.0.0.0,--metrics.port,9100" \
        "--http.port,5050,--tee,sev-snp" \
        "--tee,sev-snp,--metrics,--metrics.port=9200" \
        "--metrics.port" \
        "--http.cors-origins,*,--metrics.port,9100"; do
        check "parse_metrics_port copy matches start-vm.sh for: $csv" \
            "$(orig_parse_metrics_port "$csv")" \
            "$(parse_metrics_port "$csv")"
    done
fi

# ---- Launcher feature detection -------------------------------------------------
# The same greps run_wizard uses to decide whether the fetched launcher
# supports configurable resources. The in-repo start-vm.sh must support both.

check_true "start-vm.sh advertises KATANA_VCPUS"  grep -q "KATANA_VCPUS" "$START_VM"
check_true "start-vm.sh advertises KATANA_MEMORY" grep -q "KATANA_MEMORY" "$START_VM"

# ---- Source-tarball subtree extraction ------------------------------------------
# GitHub source tarballs nest everything under <repo>-<mangled tag>/, and the
# tag's '+' mangles unpredictably — extraction must strip by depth, not name.

FAKE_REPO="$WORK/katana-tee-vm-v0.1.0-katana-v1.8.0"
mkdir -p "$FAKE_REPO/misc/AMDSEV/scripts"
echo "#!/bin/bash" > "$FAKE_REPO/misc/AMDSEV/start-vm.sh"
echo "#!/bin/bash" > "$FAKE_REPO/misc/AMDSEV/scripts/sealed-cmdline.sh"
echo "other" > "$FAKE_REPO/README.md"
tar czf "$WORK/src.tar.gz" -C "$WORK" "$(basename "$FAKE_REPO")"
extract_amdsev_subtree "$WORK/src.tar.gz" "$WORK/extracted" 2>/dev/null
check_true  "extract_amdsev_subtree extracts start-vm.sh" test -f "$WORK/extracted/start-vm.sh"
check_true  "extract_amdsev_subtree extracts scripts/"    test -f "$WORK/extracted/scripts/sealed-cmdline.sh"
check_false "extract_amdsev_subtree skips non-AMDSEV files" test -f "$WORK/extracted/README.md"

# ---- boot_checksums_ok -----------------------------------------------------------

if command -v sha256sum >/dev/null 2>&1; then
    BOOT="$WORK/boot"
    mkdir -p "$BOOT"
    printf 'ovmf' > "$BOOT/OVMF.fd"
    printf 'kern' > "$BOOT/vmlinuz"
    printf 'init' > "$BOOT/initrd.img"
    {
        echo "OVMF_SHA256=$(sha256sum "$BOOT/OVMF.fd" | awk '{print $1}')"
        echo "KERNEL_SHA256=$(sha256sum "$BOOT/vmlinuz" | awk '{print $1}')"
        echo "INITRD_SHA256=$(sha256sum "$BOOT/initrd.img" | awk '{print $1}')"
    } > "$BOOT/build-info.txt"
    check_true "boot_checksums_ok passes on matching artifacts" boot_checksums_ok "$BOOT"
    printf 'tampered' >> "$BOOT/initrd.img"
    check_false "boot_checksums_ok fails on a tampered artifact" boot_checksums_ok "$BOOT"
    rm -rf "$BOOT"
else
    echo "skip - boot_checksums_ok tests (no sha256sum on this host)"
fi

# ---- config.env round-trip --------------------------------------------------------

TEE_HOME="$WORK/tee-home"
TAG="tee-vm-v0.9.9+katana-v9.9.9"
VCPUS=4
MEMORY="4G"
DATA_DISK="$TEE_HOME/data disk.img"   # space: exercises the %q quoting
DISK_SIZE_MB=2048
RPC_PORT=15051
SEALED=1
LUKS_UUID="00000000-0000-0000-0000-000000000001"
KATANA_ARGS_CSV="--http.cors-origins,*,--metrics.port,9100"
QEMU_PREFIX="$TEE_HOME/qemu"
LAUNCHER_HAS_VCPUS=1
LAUNCHER_HAS_MEMORY=1
write_config
write_run_sh

KATANA_TEE_TAG="" HOST_RPC_PORT="" DATA_DISK="" KATANA_ARGS_CSV=""
check_true "load_config finds the written config" load_config
check "config round-trips the tag"           "$TAG"  "$KATANA_TEE_TAG"
check "config round-trips the RPC port"      "15051" "$HOST_RPC_PORT"
check "config round-trips a path with space" "$TEE_HOME/data disk.img" "$DATA_DISK"
check "config round-trips a glob arg intact" "--http.cors-origins,*,--metrics.port,9100" "$KATANA_ARGS_CSV"

check_true "run.sh is generated executable" test -x "$TEE_HOME/run.sh"
check_true "run.sh parses (bash -n)" bash -n "$TEE_HOME/run.sh"

# ---- print-systemd -----------------------------------------------------------------

UNIT="$(print_systemd_unit)"
check_true "systemd unit points ExecStart at run.sh" \
    grep -q "ExecStart=$TEE_HOME/run.sh" <<<"$UNIT"
check_true "systemd unit restarts on failure" \
    grep -q "Restart=on-failure" <<<"$UNIT"

# ---- Optional: end-to-end dry run against a real release (network) -----------------

if [[ "${KATANA_INSTALL_TEST_NETWORK:-0}" == "1" ]]; then
    echo ""
    echo "--- network dry run (KATANA_INSTALL_TEST_NETWORK=1) ---"
    E2E_HOME="$WORK/e2e-home"
    if KATANA_INSTALL_SKIP_SNP_CHECK=1 bash "$INSTALL_SH" --yes --dry-run --home "$E2E_HOME"; then
        check_true "dry run writes config.env" test -f "$E2E_HOME/config.env"
        check_true "dry run writes run.sh"     test -x "$E2E_HOME/run.sh"
        check_true "dry run downloads boot artifacts" \
            test -f "$E2E_HOME/current/boot/build-info.txt"
        check_true "dry run fetches the matching launcher" \
            test -f "$E2E_HOME/current/src/start-vm.sh"
    else
        echo "FAIL - network dry run exited non-zero"
        FAILS=$((FAILS + 1))
    fi
fi

echo ""
if (( FAILS == 0 )); then
    echo "test-install: PASS"
    exit 0
else
    echo "test-install: FAIL ($FAILS assertion(s))"
    exit 1
fi
