#!/usr/bin/env bash
#
# Provisions an Azure Confidential VM (AMD SEV-SNP) and brings up the katana
# compose stack on it.
#
# Requirements:
#   - az cli logged in (`az login` or service principal), with permissions in
#     the target subscription to create resource groups, network resources,
#     and compute resources.
#   - An SSH public key on disk (defaults to ~/.ssh/id_rsa.pub).
#
# Usage:
#   AZ_REGION=eastus \
#   AZ_SSH_KEY_PATH=~/.ssh/id_rsa.pub \
#   AZ_SSH_PRIVATE_KEY_PATH=~/.ssh/id_rsa \
#   ./provision.sh
#
# Other env knobs:
#   STACK_NAME        — resource group name (default: katana-tee-stack)
#   VM_SIZE           — default: Standard_DC2as_v5 (SEV-SNP capable).
#                       Other options: DC4as_v5, EC2as_v5, DC2ads_v5, etc.
#   ALLOWED_CIDR      — source CIDR for SSH + RPC ingress (default: 0.0.0.0/0)
#   KATANA_REF        — git ref of dojoengine/katana to clone (default: main)
#   SETTLEMENT_* vars — passed through to the compose .env on the remote host

set -euo pipefail

STACK_NAME="${STACK_NAME:-katana-tee-stack}"
VM_SIZE="${VM_SIZE:-Standard_DC2as_v5}"
ALLOWED_CIDR="${ALLOWED_CIDR:-0.0.0.0/0}"
KATANA_REF="${KATANA_REF:-main}"

: "${AZ_REGION:?AZ_REGION is required (e.g. eastus)}"
: "${AZ_SSH_KEY_PATH:=$HOME/.ssh/id_rsa.pub}"
: "${AZ_SSH_PRIVATE_KEY_PATH:=${AZ_SSH_KEY_PATH%.pub}}"

if ! command -v az >/dev/null 2>&1; then
    echo "az cli not found. Install: https://learn.microsoft.com/cli/azure/install-azure-cli" >&2
    exit 1
fi
az account show >/dev/null 2>&1 || { echo "az cli not authenticated. Run 'az login'." >&2; exit 1; }
[ -r "$AZ_SSH_KEY_PATH" ] || { echo "SSH public key not readable: $AZ_SSH_KEY_PATH" >&2; exit 1; }
[ -r "$AZ_SSH_PRIVATE_KEY_PATH" ] || { echo "SSH private key not readable: $AZ_SSH_PRIVATE_KEY_PATH" >&2; exit 1; }

VM_NAME="${STACK_NAME}-vm"

echo "[azure/provision] Region: $AZ_REGION, VM: $VM_NAME ($VM_SIZE), RG: $STACK_NAME"

# -----------------------------------------------------------------------------
# 1. Resource group (idempotent).
# -----------------------------------------------------------------------------
az group create --name "$STACK_NAME" --location "$AZ_REGION" --output none

# -----------------------------------------------------------------------------
# 2. Confidential VM. --security-type ConfidentialVM + the CVM image variant
#    is what actually enables SEV-SNP. A standard Ubuntu image on a DC-series
#    VM will NOT give you an attestation device.
# -----------------------------------------------------------------------------
if ! az vm show --resource-group "$STACK_NAME" --name "$VM_NAME" --output none 2>/dev/null; then
    echo "[azure/provision] Creating Confidential VM..."
    az vm create \
        --resource-group "$STACK_NAME" \
        --name "$VM_NAME" \
        --size "$VM_SIZE" \
        --location "$AZ_REGION" \
        --image "Canonical:ubuntu-24_04-lts:cvm:latest" \
        --admin-username azureuser \
        --ssh-key-values "$AZ_SSH_KEY_PATH" \
        --security-type ConfidentialVM \
        --enable-vtpm true \
        --enable-secure-boot true \
        --os-disk-security-encryption-type VMGuestStateOnly \
        --public-ip-sku Standard \
        --tags "Stack=${STACK_NAME}" \
        --output none
else
    echo "[azure/provision] Reusing existing VM $VM_NAME"
fi

# -----------------------------------------------------------------------------
# 3. Open SSH + RPC ports. Replaces the default NSG rules safely (az vm open-port
#    is idempotent; duplicate calls update the existing rule).
# -----------------------------------------------------------------------------
az vm open-port --resource-group "$STACK_NAME" --name "$VM_NAME" --port 22 --priority 1000 \
    --output none || true
az vm open-port --resource-group "$STACK_NAME" --name "$VM_NAME" --port 5050 --priority 1010 \
    --output none || true

# Tighten source CIDR if the user passed a non-default.
if [ "$ALLOWED_CIDR" != "0.0.0.0/0" ]; then
    NSG_NAME=$(az vm show --resource-group "$STACK_NAME" --name "$VM_NAME" \
        --query 'networkProfile.networkInterfaces[0].id' -o tsv \
        | xargs -I{} az network nic show --ids {} --query 'networkSecurityGroup.id' -o tsv \
        | awk -F/ '{print $NF}')
    for rule in open-port-22 open-port-5050; do
        az network nsg rule update \
            --resource-group "$STACK_NAME" --nsg-name "$NSG_NAME" --name "$rule" \
            --source-address-prefixes "$ALLOWED_CIDR" --output none 2>/dev/null || true
    done
fi

PUBLIC_IP=$(az vm show --resource-group "$STACK_NAME" --name "$VM_NAME" -d \
    --query 'publicIps' -o tsv)
echo "[azure/provision] VM public IP: $PUBLIC_IP"

# -----------------------------------------------------------------------------
# 4. Bootstrap + run compose on the remote host.
# -----------------------------------------------------------------------------
SSH_OPTS=(-o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o ConnectTimeout=10 -i "$AZ_SSH_PRIVATE_KEY_PATH")

echo "[azure/provision] Waiting for SSH..."
for i in $(seq 1 30); do
    if ssh "${SSH_OPTS[@]}" "azureuser@${PUBLIC_IP}" "echo ready" >/dev/null 2>&1; then
        break
    fi
    sleep 4
done

ssh "${SSH_OPTS[@]}" "azureuser@${PUBLIC_IP}" 'bash -s' <<EOF
set -euo pipefail
if ! command -v docker >/dev/null 2>&1; then
    curl -fsSL https://get.docker.com | sudo sh
    sudo usermod -aG docker azureuser
fi
if ! command -v git >/dev/null 2>&1; then
    sudo apt-get update -qq && sudo apt-get install -y git
fi
if [ ! -d ~/katana ]; then
    git clone https://github.com/dojoengine/katana.git ~/katana
fi
cd ~/katana && git fetch --all --quiet && git checkout "$KATANA_REF" && git pull --quiet origin "$KATANA_REF" || true
EOF

ssh "${SSH_OPTS[@]}" "azureuser@${PUBLIC_IP}" "cat > ~/katana/.env" <<EOF
SETTLEMENT_RPC_URL=${SETTLEMENT_RPC_URL:-CHANGEME-sepolia-rpc}
SETTLEMENT_ACCOUNT_ADDRESS=${SETTLEMENT_ACCOUNT_ADDRESS:-0xCHANGEME}
SETTLEMENT_ACCOUNT_PRIVATE_KEY=${SETTLEMENT_ACCOUNT_PRIVATE_KEY:-0xCHANGEME}
SETTLEMENT_CHAIN_ID=${SETTLEMENT_CHAIN_ID:-sepolia}
CHAIN_ID=katana_tee
BLOCK_TIME=3000
BATCH_SIZE=1
EOF

# NOTE: mock-prove compose runs today. Real-SEV compose (docker/tee.compose.yml)
# is a v2 follow-up — swap file and add /dev/sev-guest passthrough then.
ssh "${SSH_OPTS[@]}" "azureuser@${PUBLIC_IP}" 'bash -s' <<'EOF'
set -euo pipefail
cd ~/katana
sudo docker compose -f docker/tee-mock.compose.yml --env-file .env up --build -d
EOF

echo
echo "========================================================================"
echo "VM:           $VM_NAME ($PUBLIC_IP)"
echo "RPC:          http://${PUBLIC_IP}:5050"
echo "SSH:          ssh -i $AZ_SSH_PRIVATE_KEY_PATH azureuser@${PUBLIC_IP}"
echo "Follow logs:  ssh -i $AZ_SSH_PRIVATE_KEY_PATH azureuser@${PUBLIC_IP} 'cd ~/katana && sudo docker compose -f docker/tee-mock.compose.yml --env-file .env logs -f'"
echo "Teardown:     STACK_NAME=$STACK_NAME ./cleanup.sh"
echo "========================================================================"
