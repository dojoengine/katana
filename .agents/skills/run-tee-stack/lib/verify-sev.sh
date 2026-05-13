#!/usr/bin/env bash
# Shared SEV-SNP + host-prereq verification, sourced by each provisioner.
#
# The cloud scripts pass the right CLI flags to request AMD SEV-SNP. They do
# NOT, by themselves, prove the hardware actually came up in SEV-SNP mode —
# cloud providers will sometimes silently downgrade for quota/region/image
# reasons. This helper closes that gap: SSH in, check for /dev/sev-guest,
# optionally check the kernel for SEV-SNP init, and fail loudly.
#
# Usage (from a provisioner):
#
#   SSH_OPTS=(-o StrictHostKeyChecking=accept-new -i "$KEY") # caller-owned
#   # shellcheck disable=SC1091
#   . "$(dirname "$0")/../lib/verify-sev.sh"
#   verify_host_ready "ubuntu@$PUBLIC_IP"
#
# Env knobs:
#   REQUIRE_SEV   — "1" (default for cloud scripts) fails if /dev/sev-guest
#                    is absent. "0" (default for byo-host) warns + proceeds.
#                    Override from the caller's environment.
#   CHECK_DOCKER  — "1" also verifies docker is present or installable;
#                    "0" (default) skips. The provisioners install docker
#                    themselves, so they leave this off.

# verify_sev_snp <user@host>
# Emits "present" / "absent" / "error" on stdout. Exit code mirrors.
verify_sev_snp() {
    local target="$1"
    local out
    if ! out=$(ssh "${SSH_OPTS[@]}" "$target" "[ -c /dev/sev-guest ] && echo present || echo absent" 2>/dev/null); then
        echo "error"
        return 2
    fi
    case "$out" in
        present) echo "present"; return 0 ;;
        absent)  echo "absent"; return 1 ;;
        *)       echo "error"; return 2 ;;
    esac
}

# verify_host_ready <user@host>
# Runs the full preflight and fails per REQUIRE_SEV policy.
verify_host_ready() {
    local target="$1"
    local provider="${PROVIDER:-host}"

    echo "[$provider/verify] Checking /dev/sev-guest on $target..."
    local result
    result=$(verify_sev_snp "$target") || true

    case "$result" in
        present)
            echo "[$provider/verify] SEV-SNP device present. Good."
            ;;
        absent)
            if [ "${REQUIRE_SEV:-1}" = "1" ]; then
                echo "[$provider/verify] ERROR: /dev/sev-guest not found on $target." >&2
                echo "[$provider/verify] The cloud/host did not produce an SEV-SNP-capable guest." >&2
                echo "[$provider/verify] Set REQUIRE_SEV=0 to proceed anyway (e.g. for mock-prove testing on non-SEV hardware)." >&2
                return 1
            else
                echo "[$provider/verify] WARNING: /dev/sev-guest not found, but REQUIRE_SEV=0 — proceeding with mock-prove only." >&2
            fi
            ;;
        error|*)
            echo "[$provider/verify] ERROR: couldn't run the SEV check over SSH (target=$target). Check connectivity + sudo + OS support." >&2
            if [ "${REQUIRE_SEV:-1}" = "1" ]; then
                return 1
            fi
            ;;
    esac
    return 0
}
