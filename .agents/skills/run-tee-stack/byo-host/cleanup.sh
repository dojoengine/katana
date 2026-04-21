#!/usr/bin/env bash
#
# Tears down the katana stack on a bring-your-own host. Stops the compose
# services, removes volumes, optionally removes the checkout. Does NOT touch
# docker itself or the host OS.
#
# Usage:
#   HOST_IP=1.2.3.4 SSH_USER=ubuntu SSH_KEY_PATH=~/.ssh/id_rsa ./cleanup.sh
#
# Optional:
#   REMOVE_CHECKOUT=1    — also `rm -rf ~/katana` on the host.

set -euo pipefail

: "${HOST_IP:?HOST_IP is required}"
: "${SSH_KEY_PATH:?SSH_KEY_PATH is required}"
SSH_USER="${SSH_USER:-root}"
SSH_PORT="${SSH_PORT:-22}"

SSH_OPTS=(
    -o StrictHostKeyChecking=accept-new
    -o ConnectTimeout=10
    -p "$SSH_PORT"
    -i "$SSH_KEY_PATH"
)

echo "[byo-host/cleanup] Tearing down compose on ${SSH_USER}@${HOST_IP}..."
ssh "${SSH_OPTS[@]}" "${SSH_USER}@${HOST_IP}" 'bash -s' <<'EOF'
set -euo pipefail
if [ ! -d ~/katana ]; then
    echo "[byo-host/cleanup] ~/katana not found; nothing to tear down."
    exit 0
fi
cd ~/katana
if [ "$(id -u)" -eq 0 ] || id -nG | tr ' ' '\n' | grep -qx docker; then
    DOCKER="docker"
else
    DOCKER="sudo docker"
fi
$DOCKER compose -f docker/tee-mock.compose.yml --env-file .env down -v || true
EOF

if [ "${REMOVE_CHECKOUT:-0}" = "1" ]; then
    echo "[byo-host/cleanup] Removing ~/katana on host (REMOVE_CHECKOUT=1)..."
    ssh "${SSH_OPTS[@]}" "${SSH_USER}@${HOST_IP}" "rm -rf ~/katana"
fi

echo "[byo-host/cleanup] done."
