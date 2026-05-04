#!/usr/bin/env bash
#
# Bring-your-own-host provisioner. Use this for any machine you already have
# shell access to — bare metal boxes from latitude.sh, Hetzner, OVH, a Nitro
# EC2 you booted by hand, an on-prem server, whatever. No cloud API, no
# provisioning, no network setup — the host already exists, we just get
# the stack running on it.
#
# Especially useful for real AMD SEV-SNP: bare-metal providers often have
# SEV-SNP enabled on AMD EPYC Milan+ hardware with less friction and lower
# cost than hyperscaler Confidential VMs.
#
# Requirements on YOUR machine:
#   - ssh + scp
#   - Network reachability to the host
#
# Requirements on the REMOTE host:
#   - Ubuntu 22.04 / 24.04 or Debian 12+ (other distros might work; only
#     these are tested). apt-based package manager.
#   - sudo-capable user (passwordless sudo strongly preferred)
#   - Port 5050 reachable from wherever you want to submit txs from
#   - For real SEV-SNP mode: `/dev/sev-guest` present and accessible
#
# Usage:
#   HOST_IP=1.2.3.4 \
#   SSH_USER=ubuntu \
#   SSH_KEY_PATH=~/.ssh/id_rsa \
#   SETTLEMENT_RPC_URL=https://... \
#   SETTLEMENT_ACCOUNT_ADDRESS=0x... \
#   SETTLEMENT_ACCOUNT_PRIVATE_KEY=0x... \
#   SETTLEMENT_CHAIN_ID=sepolia \
#   ./provision.sh
#
# Other env knobs:
#   KATANA_REF         — git ref of dojoengine/katana (default: main)
#   CHAIN_ID           — L3 chain id (default: katana_tee)
#   BLOCK_TIME         — L3 block time ms (default: 3000)
#   BATCH_SIZE         — saya-tee batch (default: 1)
#   SSH_PORT           — default 22

set -euo pipefail

: "${HOST_IP:?HOST_IP is required (e.g. 1.2.3.4)}"
: "${SSH_KEY_PATH:?SSH_KEY_PATH is required (path to private key)}"
SSH_USER="${SSH_USER:-root}"
SSH_PORT="${SSH_PORT:-22}"
KATANA_REF="${KATANA_REF:-main}"

[ -r "$SSH_KEY_PATH" ] || { echo "SSH_KEY_PATH not readable: $SSH_KEY_PATH" >&2; exit 1; }

SSH_OPTS=(
    -o StrictHostKeyChecking=accept-new
    -o UserKnownHostsFile=~/.ssh/known_hosts
    -o ConnectTimeout=10
    -p "$SSH_PORT"
    -i "$SSH_KEY_PATH"
)

run_remote() { ssh "${SSH_OPTS[@]}" "${SSH_USER}@${HOST_IP}" "$@"; }

# ---------------------------------------------------------------------------
# 1. Verify the host is reachable and usable.
# ---------------------------------------------------------------------------
echo "[byo-host/provision] Checking SSH reachability of ${SSH_USER}@${HOST_IP}:${SSH_PORT}..."
if ! run_remote "echo ok" >/dev/null 2>&1; then
    echo "SSH to ${SSH_USER}@${HOST_IP}:${SSH_PORT} failed. Check IP, key, firewall." >&2
    exit 1
fi

# ---------------------------------------------------------------------------
# 2. Detect CPU + SEV-SNP capability. Informative for byo-host — we don't
#    fail by default (REQUIRE_SEV=0) because users often run this against a
#    plain VM for mock-prove testing. Set REQUIRE_SEV=1 if you explicitly
#    want the script to bail when /dev/sev-guest is absent.
# ---------------------------------------------------------------------------
echo "[byo-host/provision] Detecting CPU + SEV-SNP support..."
HOST_INFO=$(run_remote 'bash -s' <<'EOF'
set -eu
echo "KERNEL=$(uname -r)"
echo "CPU=$(grep -m1 "model name" /proc/cpuinfo | sed "s/model name[[:space:]]*:[[:space:]]*//")"
[ -e /dev/sev-guest ] && echo "SEV_GUEST=present" || echo "SEV_GUEST=absent"
if dmesg 2>/dev/null | grep -q "SEV-SNP"; then
    echo "SEV_SNP_KERNEL=supported"
else
    echo "SEV_SNP_KERNEL=unknown"
fi
EOF
)
echo "$HOST_INFO" | sed 's/^/  /'

# shellcheck disable=SC1091
. "$(dirname "$0")/../lib/verify-sev.sh"
PROVIDER=byo-host REQUIRE_SEV="${REQUIRE_SEV:-0}" verify_host_ready "${SSH_USER}@${HOST_IP}"

# ---------------------------------------------------------------------------
# 3. Install prerequisites (docker + git). Skip if already present.
# ---------------------------------------------------------------------------
echo "[byo-host/provision] Installing docker + git if missing..."
run_remote 'bash -s' <<'EOF'
set -euo pipefail

need_sudo() {
    if [ "$(id -u)" -eq 0 ]; then
        "$@"
    else
        sudo -n "$@" 2>/dev/null || sudo "$@"
    fi
}

if ! command -v docker >/dev/null 2>&1; then
    echo "Installing docker..."
    if ! command -v curl >/dev/null 2>&1; then
        need_sudo apt-get update -qq
        need_sudo apt-get install -y curl ca-certificates
    fi
    curl -fsSL https://get.docker.com | need_sudo sh
fi

# Ensure compose v2 plugin exists. get.docker.com pulls it by default on
# new installs; on older boxes the plugin may be missing.
if ! docker compose version >/dev/null 2>&1; then
    need_sudo apt-get update -qq
    need_sudo apt-get install -y docker-compose-plugin || {
        echo "docker-compose-plugin not in apt. Pulling from docker's repo..."
        need_sudo mkdir -p /etc/apt/keyrings
        curl -fsSL https://download.docker.com/linux/ubuntu/gpg | need_sudo gpg --dearmor -o /etc/apt/keyrings/docker.gpg
        echo "deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/docker.gpg] https://download.docker.com/linux/ubuntu $(. /etc/os-release && echo "$VERSION_CODENAME") stable" | \
            need_sudo tee /etc/apt/sources.list.d/docker.list >/dev/null
        need_sudo apt-get update -qq
        need_sudo apt-get install -y docker-compose-plugin
    }
fi

if ! command -v git >/dev/null 2>&1; then
    need_sudo apt-get install -y git
fi

# Add non-root user to docker group so scripts don't need sudo. (If we're
# already root this is a no-op.)
if [ "$(id -u)" -ne 0 ]; then
    if ! id -nG | tr ' ' '\n' | grep -qx docker; then
        need_sudo usermod -aG docker "$(whoami)"
        echo "Added $(whoami) to docker group. A new login (or 'newgrp docker') is required for the group to take effect on interactive shells; this script uses sudo where needed."
    fi
fi
EOF

# ---------------------------------------------------------------------------
# 4. Fetch / update the katana repo on the host.
# ---------------------------------------------------------------------------
echo "[byo-host/provision] Fetching katana (ref=${KATANA_REF})..."
run_remote "bash -s" <<EOF
set -euo pipefail
if [ ! -d ~/katana ]; then
    git clone https://github.com/dojoengine/katana.git ~/katana
fi
cd ~/katana
git fetch --all --quiet
git checkout "${KATANA_REF}"
git pull --quiet origin "${KATANA_REF}" || true
EOF

# ---------------------------------------------------------------------------
# 5. Write the .env.
# ---------------------------------------------------------------------------
echo "[byo-host/provision] Writing .env on host..."
run_remote "cat > ~/katana/.env" <<EOF
SETTLEMENT_RPC_URL=${SETTLEMENT_RPC_URL:?SETTLEMENT_RPC_URL required}
SETTLEMENT_ACCOUNT_ADDRESS=${SETTLEMENT_ACCOUNT_ADDRESS:?SETTLEMENT_ACCOUNT_ADDRESS required}
SETTLEMENT_ACCOUNT_PRIVATE_KEY=${SETTLEMENT_ACCOUNT_PRIVATE_KEY:?SETTLEMENT_ACCOUNT_PRIVATE_KEY required}
SETTLEMENT_CHAIN_ID=${SETTLEMENT_CHAIN_ID:?SETTLEMENT_CHAIN_ID required}
CHAIN_ID=${CHAIN_ID:-katana_tee}
BLOCK_TIME=${BLOCK_TIME:-3000}
BATCH_SIZE=${BATCH_SIZE:-1}
EOF

# ---------------------------------------------------------------------------
# 6. Bring up the stack. Currently always the mock-prove compose — the
#    real-SEV compose (docker/tee.compose.yml) is a v2 follow-up; when it
#    lands, this script should branch on SEV_GUEST detection and use it.
# ---------------------------------------------------------------------------
echo "[byo-host/provision] Bringing up compose (detached)..."
run_remote 'bash -s' <<'EOF'
set -euo pipefail
cd ~/katana
if [ "$(id -u)" -eq 0 ] || id -nG | tr ' ' '\n' | grep -qx docker; then
    DOCKER="docker"
else
    DOCKER="sudo docker"
fi
$DOCKER compose -f docker/tee-mock.compose.yml --env-file .env up --build -d
EOF

# ---------------------------------------------------------------------------
# 7. Report.
# ---------------------------------------------------------------------------
echo
echo "========================================================================"
echo "Host:         ${SSH_USER}@${HOST_IP}"
echo "RPC:          http://${HOST_IP}:5050   (make sure 5050 is reachable from where you submit txs)"
echo "SSH:          ssh -i ${SSH_KEY_PATH} -p ${SSH_PORT} ${SSH_USER}@${HOST_IP}"
echo "Follow logs:  ssh -i ${SSH_KEY_PATH} -p ${SSH_PORT} ${SSH_USER}@${HOST_IP} 'cd ~/katana && docker compose -f docker/tee-mock.compose.yml --env-file .env logs -f'"
echo "Teardown:     HOST_IP=${HOST_IP} SSH_KEY_PATH=${SSH_KEY_PATH} SSH_USER=${SSH_USER} ./cleanup.sh"
echo "========================================================================"
