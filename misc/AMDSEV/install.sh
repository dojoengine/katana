#!/bin/bash
# ==============================================================================
# INSTALL.SH - One-command installer for the Katana TEE VM (AMD SEV-SNP)
# ==============================================================================
#
# Sets up a host with AMD SEV-SNP enabled (bare metal, or a cloud bare-metal
# instance) to run the Katana TEE VM from a published release:
#
#   curl -fsSL https://raw.githubusercontent.com/dojoengine/katana/main/misc/AMDSEV/install.sh | bash
#
# What it does:
#   1. Preflights the host (AMD SEV-SNP, KVM, required tools, free ports)
#   2. Downloads a tee-vm-v* release (boot artifacts) AND the matching
#      launcher scripts from the repo source at the same tag — they are a
#      matched pair; the release tarball alone does not include start-vm.sh
#   3. Verifies artifact checksums and the SEV-SNP launch measurement
#   4. Walks through an interactive wizard for vCPUs, memory, data disk,
#      RPC port, storage sealing and Katana args (every answer also has a
#      flag / env-var override; --yes skips the wizard entirely)
#   5. Builds QEMU (the pinned version from the release's build-qemu.sh)
#      when the host doesn't already have it
#   6. Recomputes the EXPECTED LAUNCH MEASUREMENT for the chosen config —
#      vCPU count is part of the SEV-SNP measurement (memory is not), so a
#      non-default count needs its own expected value. Computed with
#      snp-digest: the prebuilt snp-tools release pinned (tag + sha256) in
#      the tag's build-config when available, else built from source (cargo)
#   7. Writes config.env + a foreground run.sh and stops there. Persistence
#      (systemd, tmux, a supervisor) is deliberately the operator's choice:
#      `install.sh print-systemd` prints a sample unit but never installs it.
#
# The installer itself does not need root; run.sh invokes start-vm.sh via
# sudo (KVM + disk setup need it).
#
# Layout created under --home (default ~/.katana/tee-vm):
#   config.env                  all wizard answers, sourceable
#   run.sh                      generated foreground launcher
#   expected-measurement.txt    launch measurement for THIS host's config
#   install.sh                  self-copy, for upgrades / verify / print-systemd
#   data.img                    persistent VM data disk (default path)
#   qemu/                       locally-built QEMU (only if built locally)
#   current -> releases/<tag>   convenience symlink
#   releases/<tag>/boot/        OVMF.fd, vmlinuz, initrd.img, build-info.txt, ...
#   releases/<tag>/src/         misc/AMDSEV at the tag (start-vm.sh, scripts/, snp-tools/)
#
# Upgrades: re-run this script (the self-copy at ~/.katana/tee-vm/install.sh
# works too). New releases land in their own releases/<tag>/ dir; the data
# disk is never touched. Sealed-mode operators: the disk key is bound to the
# launch measurement, so upgrading re-keys the disk (see docs/amdsev.md).
#
# ==============================================================================

set -euo pipefail

# ------------------------------------------------------------------------------
# Defaults and env overrides. Flags (parsed in main) override env, env
# overrides the built-in default, and the wizard prompts for whatever is
# still unset when running interactively.
# ------------------------------------------------------------------------------
VM_REPO="${KATANA_TEE_VM_REPO:-dojoengine/katana}"
TEE_HOME="${KATANA_TEE_HOME:-$HOME/.katana/tee-vm}"
TEE_HOME_EXPLICIT=0
[[ -n "${KATANA_TEE_HOME:-}" ]] && TEE_HOME_EXPLICIT=1
TAG="${KATANA_TEE_TAG:-}"
VCPUS="${KATANA_VCPUS:-}"
MEMORY="${KATANA_MEMORY:-}"
RPC_PORT="${HOST_RPC_PORT:-}"
DATA_DISK="${KATANA_DATA_DISK:-}"
DISK_SIZE_MB="${KATANA_DISK_SIZE_MB:-}"
LUKS_UUID="${KATANA_LUKS_UUID:-}"
SEALED_MODE=""     # "", "sealed", "unsealed"
KATANA_ARGS_CSV=""
ASSUME_YES=0
DRY_RUN=0
INTERACTIVE=0
# Set to 1 to skip the SEV-SNP hardware checks (CI / dry-runs on non-SNP
# machines). The install is then only useful for testing the install flow.
SKIP_SNP_CHECK="${KATANA_INSTALL_SKIP_SNP_CHECK:-0}"

# Must match start-vm.sh's default --katana-args (the guest RPC port 5050 is
# a convention shared with KATANA_RPC_PORT there).
DEFAULT_KATANA_ARGS_CSV="--http.addr,0.0.0.0,--http.port,5050,--tee,sev-snp,--metrics,--metrics.addr,0.0.0.0,--metrics.port,9100"
DEFAULT_VCPUS=1
DEFAULT_MEMORY="4G"
DEFAULT_RPC_PORT=15051
DEFAULT_DISK_SIZE_MB=4096

usage() {
    echo "Usage: install.sh [COMMAND] [OPTIONS]"
    echo ""
    echo "Install and configure the Katana TEE VM (AMD SEV-SNP) from a published"
    echo "release. Interactive wizard by default; every answer has a flag/env"
    echo "override and --yes makes the whole run non-interactive."
    echo ""
    echo "Commands:"
    echo "  install               Preflight, download, verify, configure (default)"
    echo "  verify                Re-verify the installed release + recompute the"
    echo "                        expected launch measurement for the saved config"
    echo "  print-systemd         Print a sample systemd unit (never installs it)"
    echo ""
    echo "Options (env var in parentheses):"
    echo "  --tag TAG             Pin a tee-vm-v* release (KATANA_TEE_TAG)"
    echo "                        Default: latest published tee-vm-v* release"
    echo "  --vcpus N             Guest vCPU count (KATANA_VCPUS). Default: 1."
    echo "                        PART OF the SEV-SNP launch measurement — the"
    echo "                        expected measurement is recomputed for your value."
    echo "  --memory SIZE         Guest RAM, e.g. 4G or 2048M (KATANA_MEMORY)."
    echo "                        Default: 4G. NOT part of the measurement."
    echo "  --rpc-port PORT       Host port forwarded to guest RPC 5050"
    echo "                        (HOST_RPC_PORT). Default: 15051."
    echo "  --data-disk PATH      Persistent data disk file (KATANA_DATA_DISK)."
    echo "                        Default: <home>/data.img, created if absent."
    echo "  --disk-size-mb N      Size when creating the data disk"
    echo "                        (KATANA_DISK_SIZE_MB). Default: 4096."
    echo "  --sealed | --unsealed Storage mode. Default: unsealed. Sealed binds the"
    echo "                        disk key to the launch measurement (re-keys on"
    echo "                        upgrade — see docs/amdsev.md, Sealed storage)."
    echo "  --katana-args CSV     Comma-separated Katana CLI args (unmeasured,"
    echo "                        delivered via fw_cfg). Default: start-vm.sh's."
    echo "  --home DIR            Install root (KATANA_TEE_HOME)."
    echo "                        Default: ~/.katana/tee-vm"
    echo "  --yes                 Non-interactive: accept flags/env/defaults"
    echo "  --dry-run             Everything except QEMU build, rustup install,"
    echo "                        and data-disk creation. Combine with"
    echo "                        KATANA_INSTALL_SKIP_SNP_CHECK=1 on non-SNP hosts."
    echo "  -h, --help            Show this help"
}

# ------------------------------------------------------------------------------
# Helpers
# ------------------------------------------------------------------------------
log()  { echo "$*"; }
warn() { echo "Warning: $*" >&2; }
fail() { echo "Error: $*" >&2; exit 1; }
ok()   { printf '  [ok] %s\n' "$1"; }
bad()  { printf '  [!!] %s\n' "$1"; }
have() { command -v "$1" >/dev/null 2>&1; }

TMP_DIR=""
cleanup() {
    # Guarded `if` (not `[[ ]] &&`) so a no-op cleanup can't turn the EXIT
    # trap — and with it the script's exit status — into a failure.
    if [[ -n "$TMP_DIR" && -d "$TMP_DIR" ]]; then
        rm -rf "$TMP_DIR"
    fi
}
trap cleanup EXIT INT TERM

# Prompt on /dev/tty (curl | bash consumes stdin with the script body).
# $1 prompt, $2 default; echoes the answer. Non-interactive => the default.
ask() {
    local ans=""
    if [[ "$INTERACTIVE" -eq 1 ]]; then
        read -r -p "  $1 [$2]: " ans < /dev/tty || ans=""
    fi
    printf '%s' "${ans:-$2}"
}

# $1 prompt, $2 default, $3 validator function. Re-prompts until valid when
# interactive; hard-fails on an invalid non-interactive value.
ask_validated() {
    local ans
    while true; do
        ans="$(ask "$1" "$2")"
        if "$3" "$ans"; then
            printf '%s' "$ans"
            return 0
        fi
        [[ "$INTERACTIVE" -eq 1 ]] || fail "invalid value for '$1': '$ans'"
        echo "  invalid value: '$ans' — try again" > /dev/tty
    done
}

# $1 prompt, $2 default (y/n). Returns 0 for yes.
ask_yn() {
    local ans
    ans="$(ask "$1 (y/n)" "$2")"
    [[ "$ans" =~ ^[Yy] ]]
}

valid_port()   { [[ "$1" =~ ^[0-9]+$ ]] && (( $1 >= 1 && $1 <= 65535 )); }
valid_vcpus()  { [[ "$1" =~ ^[0-9]+$ ]] && (( $1 >= 1 )); }
# valid_vcpus plus the host's core budget (WIZ_MAX_VCPUS; 0 = unknown, no
# upper bound). A separate validator so the wizard re-prompts on an
# over-count instead of aborting the whole install.
WIZ_MAX_VCPUS=0
valid_vcpus_host() {
    valid_vcpus "$1" || return 1
    [[ "$WIZ_MAX_VCPUS" == "0" ]] || (( $1 <= WIZ_MAX_VCPUS ))
}
valid_memory() { [[ "$1" =~ ^[0-9]+[MG]$ ]]; }
valid_mode()   { [[ "$1" == "sealed" || "$1" == "unsealed" ]]; }
valid_int()    { [[ "$1" =~ ^[0-9]+$ ]] && (( $1 >= 1 )); }
valid_tag()    { [[ "$1" == tee-vm-v* ]]; }
valid_uuid()   { [[ "$1" =~ ^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$ ]]; }
valid_sha256() { [[ "$1" =~ ^[0-9a-f]{64}$ ]]; }

# True when nothing is listening on 127.0.0.1:$1.
port_free() {
    ! (exec 3<>"/dev/tcp/127.0.0.1/$1") 2>/dev/null
}

# The tag/asset name carries a '+' (SemVer build metadata); percent-encode it
# so curl sends a literal '+' in the URL path rather than a space.
encode_tag() { printf '%s' "${1//+/%2B}"; }

# Copied from start-vm.sh (parse_metrics_port) — keep in sync. The installer
# needs it before a release (and its start-vm.sh) has been chosen, so it
# can't extract the original at runtime the way the unit tests do.
parse_metrics_port() {
    local prev="" arg port=""
    local -a args
    IFS=',' read -ra args <<< "$1"
    for arg in "${args[@]}"; do
        case "$arg" in --metrics.port=*) port="${arg#*=}" ;; esac
        [[ "$prev" == "--metrics.port" ]] && port="$arg"
        prev="$arg"
    done
    printf '%s' "$port"
}

# Shared curl flags: fail loudly on a stalled network instead of hanging the
# install (unauthenticated api.github.com in particular can stall when
# rate-limited). No overall --max-time — the release tarball is hundreds of
# MB and download time is connection-dependent; the speed floor below kills
# genuinely dead transfers instead.
CURL=(curl -fsSL --connect-timeout 15 --retry 2 --speed-limit 1024 --speed-time 60)

# Latest published tee-vm-v* tag (the repo also publishes katana's own
# vX.Y.Z releases, which carry no VM image).
resolve_latest_tag() {
    "${CURL[@]}" --max-time 30 "https://api.github.com/repos/${VM_REPO}/releases?per_page=30" \
        | python3 -c 'import json,sys; rs=[r["tag_name"] for r in json.load(sys.stdin) if r["tag_name"].startswith("tee-vm-v")]; print(rs[0] if rs else "")'
}

# Read a single value from a build-info.txt by key.
info_get() {
    awk -F= -v k="$2" '$1 == k { sub(/^[^=]*=/, ""); print; exit }' "$1"
}

# True when the three boot artifacts in $1 match the SHA-256s recorded in
# $1/build-info.txt (same loop as test-snp-e2e.sh).
boot_checksums_ok() {
    local boot="$1" pair f k actual expected
    [[ -f "$boot/build-info.txt" ]] || return 1
    for pair in "OVMF.fd:OVMF_SHA256" "vmlinuz:KERNEL_SHA256" "initrd.img:INITRD_SHA256"; do
        f="${pair%%:*}"; k="${pair##*:}"
        [[ -f "$boot/$f" ]] || return 1
        actual="$(sha256sum "$boot/$f" | awk '{print $1}')"
        expected="$(info_get "$boot/build-info.txt" "$k")"
        [[ -n "$expected" && "$actual" == "$expected" ]] || return 1
    done
    return 0
}

# Extract misc/AMDSEV/ out of a repo source tarball into $2. The archive's
# top-level directory name mangles the '+' in the tag, so strip it by depth
# instead of naming it.
extract_amdsev_subtree() {
    local tarball="$1" dest="$2"
    mkdir -p "$dest"
    if tar --version 2>/dev/null | grep -q 'GNU tar'; then
        tar xzf "$tarball" -C "$dest" --strip-components=3 --wildcards '*/misc/AMDSEV/*'
    else
        # bsdtar treats patterns as globs without a flag.
        tar xzf "$tarball" -C "$dest" --strip-components=3 '*/misc/AMDSEV/*'
    fi
}

# ------------------------------------------------------------------------------
# Preflight
# ------------------------------------------------------------------------------
preflight() {
    local failures=0 t missing_tools=()

    log ""
    log "Preflight checks"

    if [[ "$SKIP_SNP_CHECK" == "1" ]]; then
        bad "SEV-SNP hardware checks SKIPPED (KATANA_INSTALL_SKIP_SNP_CHECK=1)"
    else
        if [[ "$(uname -s)" == "Linux" && "$(uname -m)" == "x86_64" ]]; then
            ok "Linux x86_64"
        else
            bad "not Linux x86_64 ($(uname -s) $(uname -m)) — SEV-SNP hosts are x86_64 Linux"
            failures=$((failures + 1))
        fi

        if [[ -e /dev/sev ]]; then
            ok "/dev/sev present (AMD SEV firmware)"
        else
            bad "/dev/sev not present — not an SNP-capable host?"
            echo "       SEV-SNP needs an AMD EPYC (Milan or newer) HOST with SNP enabled"
            echo "       in BIOS (SME + SNP settings) and an SNP-capable host kernel."
            echo "       On clouds, that means a bare-metal AMD instance (e.g. AWS"
            echo "       m6a.metal / c6a.metal); regular confidential VMs cannot host"
            echo "       nested SEV-SNP guests."
            failures=$((failures + 1))
        fi

        if [[ "$(cat /sys/module/kvm_amd/parameters/sev_snp 2>/dev/null)" == "Y" ]]; then
            ok "kvm_amd sev_snp = Y"
        else
            bad "kvm_amd sev_snp not enabled"
            echo "       Enable with:  echo 'options kvm_amd sev_snp=1 sev=1 sev_es=1' \\"
            echo "                       | sudo tee /etc/modprobe.d/kvm-amd-snp.conf"
            echo "       then reload kvm_amd (or reboot). Needs an SNP host kernel."
            failures=$((failures + 1))
        fi

        if [[ -e /dev/kvm ]]; then
            ok "/dev/kvm present"
        else
            bad "/dev/kvm not present — KVM unavailable"
            failures=$((failures + 1))
        fi
    fi

    # The installer needs the first group itself; start-vm.sh needs the rest
    # at boot time (root check, disk mkfs, control channel).
    for t in curl tar sha256sum python3 grep awk dd mkfs.ext4 socat; do
        if have "$t"; then
            continue
        fi
        missing_tools+=("$t")
    done
    if (( ${#missing_tools[@]} == 0 )); then
        ok "tools: curl tar sha256sum python3 dd mkfs.ext4 socat"
    else
        bad "missing tools: ${missing_tools[*]}"
        echo "       On Debian/Ubuntu: sudo apt-get install -y curl tar coreutils python3 e2fsprogs socat"
        if [[ "$DRY_RUN" -eq 1 ]]; then
            warn "continuing anyway (--dry-run)"
        else
            failures=$((failures + 1))
        fi
    fi

    if have cargo; then
        ok "cargo $(cargo --version 2>/dev/null | awk '{print $2}')"
    else
        bad "cargo not found — only needed to build snp-digest when the release"
        echo "       pins no prebuilt snp-digest (not fatal: the wizard offers"
        echo "       rustup, or the install falls back to checksum-only verification)"
    fi

    (( failures == 0 )) || fail "preflight failed ($failures issue(s)) — fix the items above and re-run"
}

# ------------------------------------------------------------------------------
# Download release + matching launcher scripts
# ------------------------------------------------------------------------------
fetch_release() {
    local tag="$1" tag_url boot_dir src_dir
    tag_url="$(encode_tag "$tag")"
    boot_dir="$TEE_HOME/releases/$tag/boot"
    src_dir="$TEE_HOME/releases/$tag/src"

    if boot_checksums_ok "$boot_dir"; then
        log "Release $tag already downloaded and checksums match — skipping download"
    else
        log "Downloading katana-tee-vm-${tag}.tar.gz ..."
        mkdir -p "$boot_dir"
        "${CURL[@]}" -o "$TMP_DIR/release.tar.gz" \
            "https://github.com/${VM_REPO}/releases/download/${tag_url}/katana-tee-vm-${tag_url}.tar.gz" \
            || fail "could not download release tarball for $tag"
        tar xzf "$TMP_DIR/release.tar.gz" -C "$boot_dir"
        boot_checksums_ok "$boot_dir" \
            || fail "downloaded artifacts do not match the SHA-256s recorded in build-info.txt"
        log "  boot artifacts verified against build-info.txt"
    fi

    # The release tarball carries only boot artifacts. The launcher scripts
    # (start-vm.sh, scripts/, snp-tools/) come from the repo source AT THE
    # SAME TAG — they are a matched pair; a launcher from another version may
    # assemble a different (unverifiable) boot configuration.
    if [[ -f "$src_dir/start-vm.sh" ]]; then
        log "Launcher scripts for $tag already present — skipping download"
    else
        log "Fetching launcher scripts at tag $tag ..."
        "${CURL[@]}" -o "$TMP_DIR/src.tar.gz" \
            "https://github.com/${VM_REPO}/archive/refs/tags/${tag_url}.tar.gz" \
            || fail "could not download source tarball for tag $tag"
        extract_amdsev_subtree "$TMP_DIR/src.tar.gz" "$src_dir"
        [[ -f "$src_dir/start-vm.sh" ]] || fail "source tarball for $tag has no misc/AMDSEV/start-vm.sh"
        chmod +x "$src_dir"/*.sh "$src_dir"/scripts/*.sh 2>/dev/null || true
    fi
}

# ------------------------------------------------------------------------------
# QEMU
# ------------------------------------------------------------------------------
# Echo the QEMU version pinned by the release's build-qemu.sh (single source
# of truth — don't hardcode it here too).
pinned_qemu_version() {
    sed -n 's/^QEMU_VERSION="\(.*\)"$/\1/p' "$1/scripts/build-qemu.sh" | head -n1
}

# Echo the qemu-system-x86_64 binary matching version $1: a previously
# locally-built one under $TEE_HOME/qemu first, then PATH. Empty if neither.
find_qemu() {
    local want="$1" candidate
    for candidate in "$TEE_HOME/qemu/bin/qemu-system-x86_64" "$(command -v qemu-system-x86_64 || true)"; do
        [[ -n "$candidate" && -x "$candidate" ]] || continue
        if "$candidate" --version 2>/dev/null | head -n1 | grep -q "version ${want}"; then
            printf '%s' "$candidate"
            return 0
        fi
    done
    return 0
}

ensure_qemu() {
    local src_dir="$1" want found choice
    want="$(pinned_qemu_version "$src_dir")"
    [[ -n "$want" ]] || fail "could not read QEMU_VERSION from $src_dir/scripts/build-qemu.sh"

    found="$(find_qemu "$want")"
    if [[ -n "$found" ]]; then
        log "QEMU $want found: $found"
        if [[ "$found" == "$TEE_HOME/qemu/bin/qemu-system-x86_64" ]]; then
            QEMU_PREFIX="$TEE_HOME/qemu"
        else
            QEMU_PREFIX=""
        fi
        return 0
    fi

    log "QEMU $want not found on this host (other versions may lack required"
    log "SEV-SNP features — the TEE VM is only tested against $want)."
    if [[ "$DRY_RUN" -eq 1 ]]; then
        warn "skipping QEMU build (--dry-run)"
        QEMU_PREFIX=""
        return 0
    fi

    choice="$(ask_validated "Build QEMU $want from source? (local/global/skip)" "local" valid_qemu_choice)"
    case "$choice" in
        skip)
            warn "QEMU $want not installed — run.sh will fail until it is."
            warn "Build later with: $src_dir/scripts/build-qemu.sh --prefix $TEE_HOME/qemu"
            QEMU_PREFIX=""
            ;;
        local|global)
            echo "  Build dependencies (Debian/Ubuntu):"
            echo "    sudo apt-get install -y build-essential ninja-build pkg-config \\"
            echo "        libglib2.0-dev libpixman-1-dev python3-venv flex bison wget"
            if [[ "$choice" == "global" ]]; then
                "$src_dir/scripts/build-qemu.sh" --global
                QEMU_PREFIX=""
            else
                "$src_dir/scripts/build-qemu.sh" --prefix "$TEE_HOME/qemu"
                QEMU_PREFIX="$TEE_HOME/qemu"
            fi
            [[ -n "$(find_qemu "$want")" ]] || fail "QEMU build finished but qemu-system-x86_64 $want still not found"
            ;;
    esac
}
valid_qemu_choice() { [[ "$1" == "local" || "$1" == "global" || "$1" == "skip" ]]; }

# ------------------------------------------------------------------------------
# Measurement verification (snp-digest)
# ------------------------------------------------------------------------------
ensure_cargo() {
    have cargo && return 0
    if [[ "$DRY_RUN" -eq 1 ]]; then
        return 1
    fi
    if [[ "$INTERACTIVE" -eq 1 ]]; then
        ask_yn "cargo not found — install Rust via rustup (needed to build snp-digest)?" "y" || return 1
    elif [[ "${KATANA_INSTALL_RUSTUP:-0}" != "1" ]]; then
        return 1
    fi
    log "Installing rustup ..."
    # --default-toolchain none: snp-tools/rust-toolchain.toml pins the exact
    # toolchain and rustup fetches it on first build.
    "${CURL[@]}" https://sh.rustup.rs | sh -s -- -y --default-toolchain none
    # shellcheck disable=SC1091
    [[ -f "$HOME/.cargo/env" ]] && . "$HOME/.cargo/env"
    have cargo
}

# Build snp-digest from the source at the tag and echo its path. The crate
# pins its own toolchain + target triple, so the binary lands under a
# target-specific subdirectory — find it like verify-build.sh does.
build_snp_digest() {
    local src_dir="$1"
    (cd "$src_dir/snp-tools" && cargo build --release >&2)
    find "$src_dir/snp-tools/target" -type f -name snp-digest -perm -u+x 2>/dev/null | head -n1
}

# Read a KEY="value" assignment out of a build-config file without sourcing
# it (extraction keeps this a data read, not code execution).
build_config_get() {
    sed -n "s/^${2}=\"\(.*\)\"\$/\1/p" "$1" | head -n1
}

# Fetch the prebuilt snp-digest pinned by the tag's build-config
# (SNP_DIGEST_RELEASE names a snp-tools-v* release, SNP_DIGEST_SHA256 gates
# the download — trusted because build-config is versioned in git at the
# tee-vm tag being installed) and echo its path; non-zero when the tag has no
# pin (predates it, or pins left empty), the checksum mismatches, or the
# binary doesn't run — callers fall back to the source build. It lands where
# verify-build.sh's discovery looks (snp-tools/target/release/), so the
# release verification step finds it without changes. A convenience copy, not
# the trust root: auditors build snp-digest from the pinned release's source.
fetch_prebuilt_snp_digest() {
    local src_dir="$1" rel sha_expected sha_actual rel_url dest
    # The prebuilt targets x86_64 Linux — the only SNP host platform. Dry
    # runs elsewhere (e.g. macOS) use the source-build or no-cargo paths.
    [[ "$(uname -s)/$(uname -m)" == "Linux/x86_64" ]] || return 1
    [[ -f "$src_dir/build-config" ]] || return 1
    rel="$(build_config_get "$src_dir/build-config" SNP_DIGEST_RELEASE)"
    sha_expected="$(build_config_get "$src_dir/build-config" SNP_DIGEST_SHA256)"
    [[ -n "$rel" ]] || return 1
    if ! valid_sha256 "$sha_expected"; then
        warn "build-config pins SNP_DIGEST_RELEASE=$rel but SNP_DIGEST_SHA256 is not a sha256 — falling back to source build"
        return 1
    fi
    rel_url="$(encode_tag "$rel")"
    dest="$src_dir/snp-tools/target/release/snp-digest"

    # Reuse an already-downloaded copy when it still matches the pin.
    if [[ ! -x "$dest" ]] || [[ "$(sha256sum "$dest" | awk '{print $1}')" != "$sha_expected" ]]; then
        "${CURL[@]}" -o "$TMP_DIR/snp-digest" \
            "https://github.com/${VM_REPO}/releases/download/${rel_url}/snp-digest-${rel_url}" \
            || { warn "could not download pinned snp-digest release $rel — falling back to source build"; return 1; }
        sha_actual="$(sha256sum "$TMP_DIR/snp-digest" | awk '{print $1}')"
        if [[ "$sha_actual" != "$sha_expected" ]]; then
            warn "pinned snp-digest checksum mismatch (got $sha_actual, want $sha_expected) — falling back to source build"
            return 1
        fi
        mkdir -p "$(dirname "$dest")"
        cp "$TMP_DIR/snp-digest" "$dest"
        chmod +x "$dest"
    fi

    # Smoke-run: catches a binary the host can't execute (e.g. libc mismatch).
    if ! "$dest" --help >/dev/null 2>&1; then
        warn "pinned snp-digest does not run on this host — falling back to source build"
        rm -f "$dest"
        return 1
    fi
    printf '%s' "$dest"
}

# Compute the expected launch measurement for the operator's configuration
# and write it (with provenance) to $TEE_HOME/expected-measurement.txt.
# vCPU count feeds the digest; memory does not. The sealed cmdline variant
# measures differently from unsealed, and depends on the LUKS UUID.
compute_expected_measurement() {
    local snp_digest="$1" src_dir="$2" boot_dir="$3" cmdline measurement

    if [[ "$SEALED" == "1" ]]; then
        # shellcheck disable=SC1091
        . "$src_dir/scripts/sealed-cmdline.sh"
        cmdline="$(build_sealed_cmdline "$LUKS_UUID")"
    else
        cmdline="console=ttyS0"
    fi

    measurement="$("$snp_digest" \
        --ovmf="$boot_dir/OVMF.fd" \
        --kernel="$boot_dir/vmlinuz" \
        --initrd="$boot_dir/initrd.img" \
        --append="$cmdline" \
        --vcpus="$VCPUS" --cpu=epyc-v4 --vmm=qemu --guest-features=0x1)"
    [[ "$measurement" =~ ^[0-9a-f]{96}$ ]] \
        || fail "snp-digest produced an unexpected measurement: '$measurement'"

    {
        echo "# Expected SEV-SNP launch measurement for THIS host's configuration."
        echo "# Compare against offset 0x90 of a tee_generateQuote report (decode"
        echo "# with snp-report). Generated by install.sh."
        echo "# tag=$TAG"
        echo "# vcpus=$VCPUS (measured)"
        echo "# memory=$MEMORY (not measured)"
        echo "# mode=$([[ "$SEALED" == "1" ]] && echo sealed || echo unsealed)"
        [[ "$SEALED" == "1" ]] && echo "# luks_uuid=$LUKS_UUID"
        echo "# cmdline=$cmdline"
        echo "$measurement"
    } > "$TEE_HOME/expected-measurement.txt"

    EXPECTED_MEASUREMENT="$measurement"
}

verify_and_measure() {
    local src_dir="$1" boot_dir="$2" snp_digest

    # Prefer the prebuilt verifier release pinned by the tag's build-config;
    # build from source only when the tag has no pin (or the download fails).
    snp_digest="$(fetch_prebuilt_snp_digest "$src_dir")" || snp_digest=""
    if [[ -n "$snp_digest" ]]; then
        log "Using prebuilt snp-digest $(build_config_get "$src_dir/build-config" SNP_DIGEST_RELEASE) (pinned in build-config, checksum verified)"
    elif ensure_cargo; then
        log "No usable prebuilt snp-digest for $TAG — building from source ..."
        snp_digest="$(build_snp_digest "$src_dir")"
        [[ -n "$snp_digest" ]] || fail "snp-tools built but no snp-digest binary was found"
    else
        # Boot artifacts were already checksum-verified against build-info.txt
        # in fetch_release; only the measurement recompute is unavailable.
        rm -f "$TEE_HOME/expected-measurement.txt"
        warn "=================================================================="
        warn "no prebuilt snp-digest for $TAG and cargo unavailable —"
        warn "SKIPPING launch-measurement verification."
        warn "Artifact checksums were verified, but the expected measurement for"
        warn "your configuration was NOT computed, so remote attestation results"
        warn "cannot be checked against a local expected value."
        warn "Install Rust (https://rustup.rs) and run:"
        warn "    $TEE_HOME/install.sh verify"
        warn "=================================================================="
        EXPECTED_MEASUREMENT=""
        return 0
    fi

    # Full release verification: every artifact SHA-256 plus the RECORDED
    # measurement (vcpus=1, canonical sealed UUID — the values the release
    # was published with). verify-build.sh finds snp-digest in the src tree.
    log "Verifying release checksums + published launch measurement ..."
    "$src_dir/verify-build.sh" "$boot_dir" \
        || fail "release verification failed — do not boot these artifacts"

    log "Computing expected launch measurement for your config (vcpus=$VCPUS, $([[ "$SEALED" == "1" ]] && echo sealed || echo unsealed)) ..."
    compute_expected_measurement "$snp_digest" "$src_dir" "$boot_dir"
}

# ------------------------------------------------------------------------------
# Data disk
# ------------------------------------------------------------------------------
# start-vm.sh only auto-creates its own default disk path and refuses a
# user-specified --data-disk that doesn't exist, so the installer provisions
# it the same way start-vm.sh provisions the default (dd + mkfs.ext4 for
# unsealed; raw for sealed — the guest luksFormats on first boot).
provision_data_disk() {
    if [[ -f "$DATA_DISK" ]]; then
        log "Data disk exists: $DATA_DISK (left untouched)"
        return 0
    fi
    if [[ "$DRY_RUN" -eq 1 ]]; then
        warn "skipping data disk creation (--dry-run): $DATA_DISK"
        return 0
    fi
    log "Creating data disk: $DATA_DISK (${DISK_SIZE_MB}MB)"
    mkdir -p "$(dirname "$DATA_DISK")"
    dd if=/dev/zero of="$DATA_DISK" bs=1M count="$DISK_SIZE_MB" status=none
    if [[ "$SEALED" == "1" ]]; then
        log "  left raw — the guest will luksFormat it on first boot"
    else
        mkfs.ext4 -F -q "$DATA_DISK"
        log "  formatted as plain ext4"
    fi
}

# ------------------------------------------------------------------------------
# Config + run.sh
# ------------------------------------------------------------------------------
load_config() {
    if [[ -f "$TEE_HOME/config.env" ]]; then
        # shellcheck disable=SC1091
        . "$TEE_HOME/config.env"
        return 0
    fi
    return 1
}

write_config() {
    mkdir -p "$TEE_HOME"
    {
        echo "# Katana TEE VM configuration — written by install.sh, sourced by run.sh."
        echo "# Re-run install.sh to change values (it pre-fills the wizard from this file)."
        # %q so values with spaces/globs round-trip through `source` intact.
        printf 'KATANA_TEE_TAG=%q\n'      "$TAG"
        printf 'VCPUS=%q\n'               "$VCPUS"
        printf 'MEMORY=%q\n'              "$MEMORY"
        printf 'DATA_DISK=%q\n'           "$DATA_DISK"
        printf 'DISK_SIZE_MB=%q\n'        "$DISK_SIZE_MB"
        printf 'HOST_RPC_PORT=%q\n'       "$RPC_PORT"
        printf 'SEALED=%q\n'              "$SEALED"
        printf 'LUKS_UUID=%q\n'           "$LUKS_UUID"
        printf 'KATANA_ARGS_CSV=%q\n'     "$KATANA_ARGS_CSV"
        printf 'QEMU_PREFIX=%q\n'         "$QEMU_PREFIX"
        printf 'LAUNCHER_HAS_VCPUS=%q\n'  "$LAUNCHER_HAS_VCPUS"
        printf 'LAUNCHER_HAS_MEMORY=%q\n' "$LAUNCHER_HAS_MEMORY"
    } > "$TEE_HOME/config.env"
}

write_run_sh() {
    # Static launcher: all values come from config.env at run time, so
    # editing config.env is enough — no need to regenerate run.sh.
    cat > "$TEE_HOME/run.sh" <<'EOF'
#!/bin/bash
# Generated by install.sh — launches the Katana TEE VM in the FOREGROUND
# (serial log follows; Ctrl+C triggers a graceful guest shutdown).
# Persistence (systemd, tmux, a supervisor) is deliberately the operator's
# choice — `install.sh print-systemd` prints a sample unit.
set -euo pipefail
TEE_HOME="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck disable=SC1091
. "$TEE_HOME/config.env"

BOOT="$TEE_HOME/releases/$KATANA_TEE_TAG/boot"
SRC="$TEE_HOME/releases/$KATANA_TEE_TAG/src"

# Locally-built QEMU (if any) first; `sudo env PATH=...` below carries it
# past sudo's secure_path.
if [[ -n "${QEMU_PREFIX:-}" ]]; then
    PATH="$QEMU_PREFIX/bin:$PATH"
fi

ENV_VARS=("PATH=$PATH" "HOST_RPC_PORT=$HOST_RPC_PORT")
# Resources ride env vars (not flags) so the same run.sh works with older
# launchers too: a launcher without KATANA_VCPUS/KATANA_MEMORY support keeps
# its hardcoded values, which is exactly what its published measurement pins.
[[ "${LAUNCHER_HAS_VCPUS:-0}" == "1" ]] && ENV_VARS+=("KATANA_VCPUS=$VCPUS")
[[ "${LAUNCHER_HAS_MEMORY:-0}" == "1" ]] && ENV_VARS+=("KATANA_MEMORY=$MEMORY")

ARGS=(
    --ovmf "$BOOT/OVMF.fd"
    --kernel "$BOOT/vmlinuz"
    --initrd "$BOOT/initrd.img"
    --data-disk "$DATA_DISK"
    --katana-args "$KATANA_ARGS_CSV"
)
if [[ "${SEALED:-0}" == "1" ]]; then
    ARGS+=(--sealed --luks-uuid "$LUKS_UUID")
fi

# start-vm.sh needs root for KVM and disk setup.
exec sudo env "${ENV_VARS[@]}" "$SRC/start-vm.sh" "${ARGS[@]}"
EOF
    chmod +x "$TEE_HOME/run.sh"
}

# Keep a durable copy of the installer for upgrades / verify / print-systemd
# (a curl | bash invocation has no on-disk script to re-run). Prefer the
# copy shipped in the source at the installed tag; fall back to this script
# file when the tag predates install.sh.
persist_installer() {
    local src_dir="$1" self="${BASH_SOURCE[0]:-}"
    if [[ -f "$src_dir/install.sh" ]]; then
        cp "$src_dir/install.sh" "$TEE_HOME/install.sh"
    elif [[ -n "$self" && -f "$self" ]]; then
        cp "$self" "$TEE_HOME/install.sh"
    else
        warn "could not persist a copy of install.sh (running from a pipe and"
        warn "release $TAG predates the installer) — re-curl it for upgrades"
        return 0
    fi
    chmod +x "$TEE_HOME/install.sh"
}

print_systemd_unit() {
    # Printed, never installed: the installer stays unopinionated about how
    # (or whether) the VM is supervised.
    cat <<EOF
# Sample systemd unit for the Katana TEE VM. Review, then install with:
#   sudo cp katana-tee-vm.service /etc/systemd/system/
#   sudo systemctl daemon-reload
#   sudo systemctl enable --now katana-tee-vm
[Unit]
Description=Katana TEE VM (AMD SEV-SNP)
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
ExecStart=$TEE_HOME/run.sh
Restart=on-failure
RestartSec=5
# start-vm.sh's EXIT trap performs a graceful guest shutdown (katana TERM,
# sync, unmount, poweroff) on SIGTERM — give it time before SIGKILL. The
# data disk is persistent, so restarts resume the existing chain state.
KillMode=mixed
TimeoutStopSec=120

[Install]
WantedBy=multi-user.target
EOF
}

# ------------------------------------------------------------------------------
# Wizard
# ------------------------------------------------------------------------------
run_wizard() {
    local src_dir="$1"

    log ""
    log "Configuration (Enter accepts the default; flags/env vars override)"

    # --- vCPUs / memory: gated on what the tag's launcher supports ----------
    LAUNCHER_HAS_VCPUS=0
    LAUNCHER_HAS_MEMORY=0
    grep -q 'KATANA_VCPUS' "$src_dir/start-vm.sh" && LAUNCHER_HAS_VCPUS=1
    grep -q 'KATANA_MEMORY' "$src_dir/start-vm.sh" && LAUNCHER_HAS_MEMORY=1

    if [[ "$LAUNCHER_HAS_VCPUS" == "1" ]]; then
        # nproc on Linux (the real SNP hosts); getconf covers macOS dry runs.
        WIZ_MAX_VCPUS="$(nproc 2>/dev/null || getconf _NPROCESSORS_ONLN 2>/dev/null || echo 0)"
        valid_vcpus "$WIZ_MAX_VCPUS" || WIZ_MAX_VCPUS=0
        if [[ -z "$VCPUS" ]]; then
            echo "  vCPU count is PART OF the SEV-SNP launch measurement; the expected"
            echo "  measurement will be recomputed for your value."
            if [[ "$WIZ_MAX_VCPUS" != "0" ]]; then
                VCPUS="$(ask_validated "vCPUs (host has $WIZ_MAX_VCPUS cores available)" "$WIZ_DEF_VCPUS" valid_vcpus_host)"
            else
                VCPUS="$(ask_validated "vCPUs" "$WIZ_DEF_VCPUS" valid_vcpus_host)"
            fi
        else
            valid_vcpus "$VCPUS" || fail "invalid --vcpus: '$VCPUS'"
            valid_vcpus_host "$VCPUS" || fail "vCPUs ($VCPUS) exceeds host CPU count ($WIZ_MAX_VCPUS)"
        fi
    else
        if [[ -n "$VCPUS" && "$VCPUS" != "1" ]]; then
            fail "release $TAG's launcher predates configurable vCPUs — pick a newer release or keep --vcpus 1"
        fi
        VCPUS=1
        log "  vCPUs: locked to 1 (release $TAG's launcher predates configurable vCPUs)"
    fi

    if [[ "$LAUNCHER_HAS_MEMORY" == "1" ]]; then
        if [[ -z "$MEMORY" ]]; then
            echo "  Memory is NOT part of the measurement — size freely. The initramfs"
            echo "  (incl. the katana binary) unpacks into guest RAM: 4G minimum advised."
            MEMORY="$(ask_validated "Memory (e.g. 4G, 2048M)" "$WIZ_DEF_MEMORY" valid_memory)"
        else
            valid_memory "$MEMORY" || fail "invalid --memory: '$MEMORY'"
        fi
    else
        if [[ -n "$MEMORY" ]]; then
            fail "release $TAG's launcher predates configurable memory — pick a newer release or drop --memory"
        fi
        MEMORY=""
        log "  Memory: launcher default (release $TAG's launcher predates configurable memory)"
    fi

    # --- Data disk -----------------------------------------------------------
    if [[ -z "$DATA_DISK" ]]; then
        DATA_DISK="$(ask "Data disk path" "$WIZ_DEF_DISK")"
    fi
    if [[ -f "$DATA_DISK" ]]; then
        DISK_SIZE_MB="${DISK_SIZE_MB:-$WIZ_DEF_DISK_MB}"
    elif [[ -z "$DISK_SIZE_MB" ]]; then
        DISK_SIZE_MB="$(ask_validated "Data disk size in MB (created now)" "$WIZ_DEF_DISK_MB" valid_int)"
    else
        valid_int "$DISK_SIZE_MB" || fail "invalid --disk-size-mb: '$DISK_SIZE_MB'"
    fi

    # --- RPC port ------------------------------------------------------------
    if [[ -z "$RPC_PORT" ]]; then
        RPC_PORT="$(ask_validated "Host RPC port" "$WIZ_DEF_RPC_PORT" valid_port)"
    else
        valid_port "$RPC_PORT" || fail "invalid --rpc-port: '$RPC_PORT'"
    fi
    # Warn rather than fail: on an upgrade re-run the currently-running TEE VM
    # legitimately holds this port — it only needs to be free at boot time.
    port_free "$RPC_PORT" || warn "port $RPC_PORT is currently in use (a running TEE VM? it must be free when run.sh boots)"

    # --- Storage mode ----------------------------------------------------------
    if [[ -z "$SEALED_MODE" ]]; then
        echo "  Sealed storage binds the disk key to the launch measurement: strong"
        echo "  at-rest protection, but a release upgrade re-keys the disk and the"
        echo "  old data no longer unseals (docs/amdsev.md, Sealed storage)."
        SEALED_MODE="$(ask_validated "Storage mode (unsealed/sealed)" "$WIZ_DEF_SEALED_MODE" valid_mode)"
    fi
    if [[ "$SEALED_MODE" == "sealed" ]]; then
        SEALED=1
        if [[ -z "$LUKS_UUID" ]]; then
            if have uuidgen; then
                LUKS_UUID="$(uuidgen | tr '[:upper:]' '[:lower:]')"
            else
                LUKS_UUID="$(python3 -c 'import uuid; print(uuid.uuid4())')"
            fi
            log "  Generated LUKS UUID: $LUKS_UUID (persisted in config.env; part of"
            log "  the sealed launch measurement, stable across boots on this host)"
        fi
        valid_uuid "$LUKS_UUID" || fail "invalid LUKS UUID: '$LUKS_UUID'"
    else
        SEALED=0
        LUKS_UUID=""
    fi

    # --- Katana args -----------------------------------------------------------
    if [[ -z "$KATANA_ARGS_CSV" ]]; then
        if ask_yn "Keep these Katana args? ($WIZ_DEF_ARGS)" "y"; then
            KATANA_ARGS_CSV="$WIZ_DEF_ARGS"
        else
            KATANA_ARGS_CSV="$(ask "Katana args (comma-separated; keep --http.port 5050)" "$WIZ_DEF_ARGS")"
        fi
    fi
    local metrics_port
    metrics_port="$(parse_metrics_port "$KATANA_ARGS_CSV")"
    if [[ -n "$metrics_port" ]]; then
        valid_port "$metrics_port" || fail "invalid --metrics.port in katana args: '$metrics_port'"
        port_free "$metrics_port" || warn "metrics port $metrics_port is currently in use (it must be free when run.sh boots)"
    fi
}

# ------------------------------------------------------------------------------
# Subcommands
# ------------------------------------------------------------------------------
cmd_install() {
    local prev_tag="" prev_sealed=""

    # Pre-fill the wizard from an existing install (upgrade / re-run path).
    # Precedence: flag/env > saved config > built-in default. Sourcing
    # config.env would clobber the flag values (same variable names), so
    # stash them first and restore after; the config values become the
    # wizard's prompt defaults instead.
    local f_vcpus="$VCPUS" f_memory="$MEMORY" f_rpc="$RPC_PORT" f_disk="$DATA_DISK"
    local f_disk_mb="$DISK_SIZE_MB" f_args="$KATANA_ARGS_CSV" f_sealed_mode="$SEALED_MODE"
    local f_luks="$LUKS_UUID"
    WIZ_DEF_VCPUS="$DEFAULT_VCPUS"
    WIZ_DEF_MEMORY="$DEFAULT_MEMORY"
    WIZ_DEF_RPC_PORT="$DEFAULT_RPC_PORT"
    WIZ_DEF_DISK="$TEE_HOME/data.img"
    WIZ_DEF_DISK_MB="$DEFAULT_DISK_SIZE_MB"
    WIZ_DEF_SEALED_MODE="unsealed"
    WIZ_DEF_ARGS="$DEFAULT_KATANA_ARGS_CSV"
    SEALED="" LUKS_UUID="" QEMU_PREFIX=""
    if load_config; then
        prev_tag="${KATANA_TEE_TAG:-}"
        prev_sealed="${SEALED:-0}"
        [[ -n "${VCPUS:-}" ]]           && WIZ_DEF_VCPUS="$VCPUS"
        [[ -n "${MEMORY:-}" ]]          && WIZ_DEF_MEMORY="$MEMORY"
        [[ -n "${HOST_RPC_PORT:-}" ]]   && WIZ_DEF_RPC_PORT="$HOST_RPC_PORT"
        [[ -n "${DATA_DISK:-}" ]]       && WIZ_DEF_DISK="$DATA_DISK"
        [[ -n "${DISK_SIZE_MB:-}" ]]    && WIZ_DEF_DISK_MB="$DISK_SIZE_MB"
        [[ -n "${KATANA_ARGS_CSV:-}" ]] && WIZ_DEF_ARGS="$KATANA_ARGS_CSV"
        [[ "$prev_sealed" == "1" ]]     && WIZ_DEF_SEALED_MODE="sealed"
        log "Existing install found at $TEE_HOME (tag ${prev_tag:-<none>}) — values pre-filled"
    fi
    VCPUS="$f_vcpus"
    MEMORY="$f_memory"
    RPC_PORT="$f_rpc"
    DATA_DISK="$f_disk"
    DISK_SIZE_MB="$f_disk_mb"
    KATANA_ARGS_CSV="$f_args"
    SEALED_MODE="$f_sealed_mode"
    # No flag for the LUKS UUID (env KATANA_LUKS_UUID only) — reuse the saved
    # one so a sealed re-run keeps its measurement-stable UUID.
    [[ -z "$f_luks" ]] || LUKS_UUID="$f_luks"
    QEMU_PREFIX=""
    EXPECTED_MEASUREMENT=""

    log "Katana TEE VM installer"
    log "======================="

    preflight

    # --- Resolve the release tag ---------------------------------------------
    if [[ -z "$TAG" ]]; then
        log ""
        log "Resolving latest TEE-VM release (tee-vm-v*) ..."
        local latest
        latest="$(resolve_latest_tag)"
        [[ -n "$latest" ]] || fail "no published tee-vm-v* releases found in $VM_REPO"
        TAG="$(ask_validated "Release tag" "$latest" valid_tag)"
    else
        valid_tag "$TAG" || fail "not a TEE-VM release tag (want tee-vm-v*): '$TAG'"
    fi

    # Sealed upgrade guard: the sealed disk key is bound to the launch
    # measurement, so a release change re-keys the disk — the old data will
    # NOT unseal under the new release.
    if [[ "$prev_sealed" == "1" && -n "$prev_tag" && "$TAG" != "$prev_tag" ]]; then
        warn "sealed-mode release change: $prev_tag -> $TAG re-keys the data disk;"
        warn "the existing sealed data will no longer unseal. Use a fresh --data-disk"
        warn "to keep the old disk recoverable under the old release."
        if [[ "$INTERACTIVE" -eq 1 ]]; then
            ask_yn "Continue anyway?" "n" || fail "aborted by user"
        elif [[ "${KATANA_CONFIRM_SEALED_UPGRADE:-0}" != "1" ]]; then
            fail "refusing a sealed-mode release change non-interactively — set KATANA_CONFIRM_SEALED_UPGRADE=1 to proceed"
        fi
    fi

    mkdir -p "$TEE_HOME"
    fetch_release "$TAG"
    local boot_dir="$TEE_HOME/releases/$TAG/boot"
    local src_dir="$TEE_HOME/releases/$TAG/src"

    run_wizard "$src_dir"
    ensure_qemu "$src_dir"
    verify_and_measure "$src_dir" "$boot_dir"
    provision_data_disk

    write_config
    write_run_sh
    persist_installer "$src_dir"
    ln -sfn "releases/$TAG" "$TEE_HOME/current"

    # --- Summary ---------------------------------------------------------------
    local metrics_port
    metrics_port="$(parse_metrics_port "$KATANA_ARGS_CSV")"
    log ""
    log "============================================================"
    log "Installed: $TAG"
    log "Install root:  $TEE_HOME"
    if [[ -n "$EXPECTED_MEASUREMENT" ]]; then
        log "Expected launch measurement (vcpus=$VCPUS, $([[ "$SEALED" == "1" ]] && echo sealed || echo unsealed)):"
        log "  $EXPECTED_MEASUREMENT"
        log "  (saved to $TEE_HOME/expected-measurement.txt)"
    else
        log "Expected launch measurement: NOT computed (no cargo) — run"
        log "  $TEE_HOME/install.sh verify   after installing Rust"
    fi
    log ""
    log "Run the VM (foreground):        $TEE_HOME/run.sh"
    log "  RPC:      http://localhost:$RPC_PORT   (starknet JSON-RPC + tee_generateQuote)"
    if [[ -n "$metrics_port" ]]; then
        log "  Metrics:  http://localhost:$metrics_port"
    fi
    log "  Data:     $DATA_DISK (persists across restarts)"
    log ""
    log "Persistence is up to you — print a sample systemd unit with:"
    log "                                $TEE_HOME/install.sh print-systemd"
    log "Re-verify the release:          $TEE_HOME/install.sh verify"
    log "Upgrade to the latest release:  $TEE_HOME/install.sh"
    log "============================================================"
    if [[ "$DRY_RUN" -eq 1 ]]; then
        log "(dry run: QEMU build, rustup install and data-disk creation were skipped)"
    fi
}

cmd_verify() {
    load_config || fail "no install found at $TEE_HOME (run install first)"
    TAG="$KATANA_TEE_TAG"
    RPC_PORT="$HOST_RPC_PORT"
    EXPECTED_MEASUREMENT=""
    local boot_dir="$TEE_HOME/releases/$TAG/boot"
    local src_dir="$TEE_HOME/releases/$TAG/src"
    [[ -d "$boot_dir" && -d "$src_dir" ]] || fail "release $TAG is missing under $TEE_HOME/releases"

    boot_checksums_ok "$boot_dir" || fail "boot artifact checksums do NOT match build-info.txt"
    log "Boot artifact checksums OK"
    verify_and_measure "$src_dir" "$boot_dir"
    if [[ -n "$EXPECTED_MEASUREMENT" ]]; then
        log ""
        log "Expected launch measurement (vcpus=$VCPUS, $([[ "$SEALED" == "1" ]] && echo sealed || echo unsealed)):"
        log "  $EXPECTED_MEASUREMENT"
    fi
}

cmd_print_systemd() {
    # config.env is optional here — the unit only needs the install root.
    print_systemd_unit
}

# ------------------------------------------------------------------------------
# Main
# ------------------------------------------------------------------------------
main() {
    local command="install"
    if [[ $# -gt 0 ]]; then
        case "$1" in
            install|verify|print-systemd)
                command="$1"
                shift
                ;;
        esac
    fi

    while [[ $# -gt 0 ]]; do
        case "$1" in
            --tag)          [[ $# -ge 2 ]] || fail "--tag requires a value";          TAG="$2"; shift 2 ;;
            --vcpus)        [[ $# -ge 2 ]] || fail "--vcpus requires a value";        VCPUS="$2"; shift 2 ;;
            --memory)       [[ $# -ge 2 ]] || fail "--memory requires a value";       MEMORY="$2"; shift 2 ;;
            --rpc-port)     [[ $# -ge 2 ]] || fail "--rpc-port requires a value";     RPC_PORT="$2"; shift 2 ;;
            --data-disk)    [[ $# -ge 2 ]] || fail "--data-disk requires a value";    DATA_DISK="$2"; shift 2 ;;
            --disk-size-mb) [[ $# -ge 2 ]] || fail "--disk-size-mb requires a value"; DISK_SIZE_MB="$2"; shift 2 ;;
            --katana-args)  [[ $# -ge 2 ]] || fail "--katana-args requires a value";  KATANA_ARGS_CSV="$2"; shift 2 ;;
            --home)         [[ $# -ge 2 ]] || fail "--home requires a value";         TEE_HOME="$2"; TEE_HOME_EXPLICIT=1; shift 2 ;;
            --sealed)       SEALED_MODE="sealed"; shift ;;
            --unsealed)     SEALED_MODE="unsealed"; shift ;;
            --yes)          ASSUME_YES=1; shift ;;
            --dry-run)      DRY_RUN=1; shift ;;
            -h|--help)      usage; exit 0 ;;
            *)              echo "Error: unknown option: $1" >&2; echo "" >&2; usage >&2; exit 1 ;;
        esac
    done

    # When invoked from a persisted copy (~/.katana/tee-vm/install.sh) without
    # an explicit --home / KATANA_TEE_HOME, operate on the install the copy
    # lives in — not the default path — so upgrades/verify/print-systemd of a
    # relocated install target the right one.
    if [[ "$TEE_HOME_EXPLICIT" -eq 0 ]]; then
        local self_dir=""
        if [[ -n "${BASH_SOURCE[0]:-}" && -f "${BASH_SOURCE[0]}" ]]; then
            self_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
        fi
        if [[ -n "$self_dir" && -f "$self_dir/config.env" ]]; then
            TEE_HOME="$self_dir"
        fi
    fi

    # Interactive only when the operator can actually answer: not --yes, and
    # /dev/tty is usable (curl | bash keeps stdin busy with the script body,
    # so the terminal — not stdin — is the deciding factor).
    if [[ "$ASSUME_YES" -eq 0 ]] && { : < /dev/tty; } 2>/dev/null; then
        INTERACTIVE=1
    elif [[ "$ASSUME_YES" -eq 0 && "$command" == "install" ]]; then
        fail "no terminal available for the wizard — re-run non-interactively, e.g.:
    curl -fsSL .../install.sh | bash -s -- --yes [--vcpus N] [--memory SIZE] ..."
    fi

    TMP_DIR="$(mktemp -d /tmp/katana-tee-install.XXXXXX)"

    case "$command" in
        install)       cmd_install ;;
        verify)        cmd_verify ;;
        print-systemd) cmd_print_systemd ;;
    esac
}

# Test hook: KATANA_INSTALL_LIB=1 sources the helpers without running main
# (see scripts/test-install.sh).
if [[ "${KATANA_INSTALL_LIB:-0}" == "1" ]]; then
    return 0 2>/dev/null || exit 0
fi
main "$@"
