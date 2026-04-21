#!/usr/bin/env bash
#
# Provisions a GCP Confidential VM with AMD SEV-SNP and brings up the katana
# compose stack on it.
#
# Requirements:
#   - gcloud CLI installed + authenticated (`gcloud auth login`), default
#     project set (`gcloud config set project <PROJECT_ID>`), Compute Engine
#     API and Confidential Computing API enabled on that project.
#   - An SSH public key on disk (defaults to ~/.ssh/id_rsa.pub). gcloud will
#     add it to the VM's project-level SSH keys.
#
# Usage:
#   GCP_ZONE=us-central1-a \
#   GCP_SSH_KEY_PATH=~/.ssh/id_rsa.pub \
#   GCP_SSH_PRIVATE_KEY_PATH=~/.ssh/id_rsa \
#   ./provision.sh
#
# Other env knobs:
#   STACK_NAME        — tag + VM name prefix (default: katana-tee-stack)
#   MACHINE_TYPE      — default: n2d-standard-2 (SEV-SNP capable).
#                       Note: SEV-SNP requires N2D and the minCpuPlatform must
#                       be "AMD Milan" or newer.
#   ALLOWED_CIDR      — source CIDR for SSH + RPC ingress (default: 0.0.0.0/0)
#   KATANA_REF        — git ref of dojoengine/katana (default: main)
#   SETTLEMENT_* vars — passed through to the compose .env on the remote host

set -euo pipefail

STACK_NAME="${STACK_NAME:-katana-tee-stack}"
MACHINE_TYPE="${MACHINE_TYPE:-n2d-standard-2}"
ALLOWED_CIDR="${ALLOWED_CIDR:-0.0.0.0/0}"
KATANA_REF="${KATANA_REF:-main}"

: "${GCP_ZONE:?GCP_ZONE is required (e.g. us-central1-a)}"
: "${GCP_SSH_KEY_PATH:=$HOME/.ssh/id_rsa.pub}"
: "${GCP_SSH_PRIVATE_KEY_PATH:=${GCP_SSH_KEY_PATH%.pub}}"

if ! command -v gcloud >/dev/null 2>&1; then
    echo "gcloud CLI not found. Install: https://cloud.google.com/sdk/docs/install" >&2
    exit 1
fi
PROJECT=$(gcloud config get-value project 2>/dev/null || true)
[ -n "$PROJECT" ] || { echo "No default gcloud project. Run 'gcloud config set project <PROJECT_ID>'." >&2; exit 1; }

[ -r "$GCP_SSH_KEY_PATH" ] || { echo "SSH public key not readable: $GCP_SSH_KEY_PATH" >&2; exit 1; }
[ -r "$GCP_SSH_PRIVATE_KEY_PATH" ] || { echo "SSH private key not readable: $GCP_SSH_PRIVATE_KEY_PATH" >&2; exit 1; }

VM_NAME="${STACK_NAME}"
FW_NAME="${STACK_NAME}-allow"

echo "[gcp/provision] Project: $PROJECT, Zone: $GCP_ZONE, VM: $VM_NAME ($MACHINE_TYPE)"

# -----------------------------------------------------------------------------
# 1. Firewall rule (idempotent): allow SSH + katana RPC to instances tagged
#    with $STACK_NAME.
# -----------------------------------------------------------------------------
if ! gcloud compute firewall-rules describe "$FW_NAME" --project "$PROJECT" --quiet 2>/dev/null >/dev/null; then
    gcloud compute firewall-rules create "$FW_NAME" \
        --project "$PROJECT" \
        --direction=INGRESS \
        --action=ALLOW \
        --rules=tcp:22,tcp:5050 \
        --source-ranges="$ALLOWED_CIDR" \
        --target-tags="$STACK_NAME" >/dev/null
    echo "[gcp/provision] Created firewall rule $FW_NAME"
fi

# -----------------------------------------------------------------------------
# 2. Create the Confidential VM with SEV-SNP. --confidential-compute-type=SEV_SNP
#    plus --min-cpu-platform=AMD\ Milan is what switches on the attestation.
# -----------------------------------------------------------------------------
if ! gcloud compute instances describe "$VM_NAME" --zone "$GCP_ZONE" --project "$PROJECT" --quiet 2>/dev/null >/dev/null; then
    echo "[gcp/provision] Creating Confidential VM (SEV-SNP)..."
    gcloud compute instances create "$VM_NAME" \
        --project "$PROJECT" \
        --zone "$GCP_ZONE" \
        --machine-type "$MACHINE_TYPE" \
        --confidential-compute-type=SEV_SNP \
        --min-cpu-platform="AMD Milan" \
        --maintenance-policy=TERMINATE \
        --image-family=ubuntu-2404-lts-amd64 \
        --image-project=ubuntu-os-cloud \
        --boot-disk-size=40GB \
        --boot-disk-type=pd-balanced \
        --tags="$STACK_NAME" \
        --metadata="enable-oslogin=FALSE,ssh-keys=ubuntu:$(cat "$GCP_SSH_KEY_PATH")" \
        --labels="stack=$STACK_NAME" >/dev/null
else
    echo "[gcp/provision] Reusing existing VM $VM_NAME"
fi

echo "[gcp/provision] Waiting for VM to report RUNNING..."
until [ "$(gcloud compute instances describe "$VM_NAME" --zone "$GCP_ZONE" --project "$PROJECT" --format='value(status)' 2>/dev/null)" = "RUNNING" ]; do
    sleep 3
done

PUBLIC_IP=$(gcloud compute instances describe "$VM_NAME" --zone "$GCP_ZONE" --project "$PROJECT" \
    --format='get(networkInterfaces[0].accessConfigs[0].natIP)')
echo "[gcp/provision] VM public IP: $PUBLIC_IP"

# -----------------------------------------------------------------------------
# 3. Wait for SSH, then bootstrap the host.
# -----------------------------------------------------------------------------
SSH_OPTS=(-o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o ConnectTimeout=10 -i "$GCP_SSH_PRIVATE_KEY_PATH")

echo "[gcp/provision] Waiting for SSH..."
for i in $(seq 1 30); do
    if ssh "${SSH_OPTS[@]}" "ubuntu@${PUBLIC_IP}" "echo ready" >/dev/null 2>&1; then
        break
    fi
    sleep 4
done

# Verify SEV-SNP is live. GCP ties SEV-SNP to N2D + --min-cpu-platform; if
# either falls back to a generic image or an older CPU, the VM boots without
# the attestation device. `--min-cpu-platform="AMD Milan"` helps but is not
# itself proof SEV came up.
# shellcheck disable=SC1091
. "$(dirname "$0")/../lib/verify-sev.sh"
PROVIDER=gcp verify_host_ready "ubuntu@${PUBLIC_IP}"

ssh "${SSH_OPTS[@]}" "ubuntu@${PUBLIC_IP}" 'bash -s' <<EOF
set -euo pipefail
if ! command -v docker >/dev/null 2>&1; then
    curl -fsSL https://get.docker.com | sudo sh
    sudo usermod -aG docker ubuntu
fi
if ! command -v git >/dev/null 2>&1; then
    sudo apt-get update -qq && sudo apt-get install -y git
fi
if [ ! -d ~/katana ]; then
    git clone https://github.com/dojoengine/katana.git ~/katana
fi
cd ~/katana && git fetch --all --quiet && git checkout "$KATANA_REF" && git pull --quiet origin "$KATANA_REF" || true
EOF

ssh "${SSH_OPTS[@]}" "ubuntu@${PUBLIC_IP}" "cat > ~/katana/.env" <<EOF
SETTLEMENT_RPC_URL=${SETTLEMENT_RPC_URL:-CHANGEME-sepolia-rpc}
SETTLEMENT_ACCOUNT_ADDRESS=${SETTLEMENT_ACCOUNT_ADDRESS:-0xCHANGEME}
SETTLEMENT_ACCOUNT_PRIVATE_KEY=${SETTLEMENT_ACCOUNT_PRIVATE_KEY:-0xCHANGEME}
SETTLEMENT_CHAIN_ID=${SETTLEMENT_CHAIN_ID:-sepolia}
CHAIN_ID=katana_tee
BLOCK_TIME=3000
BATCH_SIZE=1
EOF

# NOTE: mock-prove compose for now. Swap for docker/tee.compose.yml when v2 lands.
ssh "${SSH_OPTS[@]}" "ubuntu@${PUBLIC_IP}" 'bash -s' <<'EOF'
set -euo pipefail
cd ~/katana
sudo docker compose -f docker/tee-mock.compose.yml --env-file .env up --build -d
EOF

echo
echo "========================================================================"
echo "VM:           $VM_NAME ($PUBLIC_IP)"
echo "RPC:          http://${PUBLIC_IP}:5050"
echo "SSH:          ssh -i $GCP_SSH_PRIVATE_KEY_PATH ubuntu@${PUBLIC_IP}"
echo "Follow logs:  ssh -i $GCP_SSH_PRIVATE_KEY_PATH ubuntu@${PUBLIC_IP} 'cd ~/katana && sudo docker compose -f docker/tee-mock.compose.yml --env-file .env logs -f'"
echo "Teardown:     STACK_NAME=$STACK_NAME GCP_ZONE=$GCP_ZONE ./cleanup.sh"
echo "========================================================================"
