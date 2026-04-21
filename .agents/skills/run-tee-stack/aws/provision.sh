#!/usr/bin/env bash
#
# Provisions an AWS EC2 SEV-SNP capable VM, installs Docker, clones katana,
# and brings up the compose stack. Idempotent when run repeatedly: reuses
# existing resources tagged with the stack name.
#
# Requirements:
#   - aws cli v2 installed and authenticated (`aws sts get-caller-identity`)
#   - An EC2 key pair already created in the target region (SSH keypair name,
#     with the matching .pem readable locally)
#   - IAM permission to: EC2:RunInstances, DescribeInstances, CreateTags,
#     CreateSecurityGroup, AuthorizeSecurityGroupIngress, DescribeImages,
#     GetParameters (SSM for AMI lookup)
#
# Usage:
#   AWS_REGION=us-east-2 \
#   AWS_KEY_NAME=my-ssh-key \
#   AWS_KEY_PATH=~/.ssh/my-ssh-key.pem \
#   ./provision.sh
#
# Other env knobs (optional):
#   STACK_NAME         — tag + security group prefix (default: katana-tee-stack)
#   INSTANCE_TYPE      — must be AMD EPYC Milan+ (default: m7a.large)
#   ALLOWED_CIDR       — source CIDR for SSH + RPC ingress (default: 0.0.0.0/0, tighten this)
#   KATANA_REF         — git ref of dojoengine/katana to clone (default: main)
#   SETTLEMENT_RPC_URL, SETTLEMENT_ACCOUNT_ADDRESS,
#   SETTLEMENT_ACCOUNT_PRIVATE_KEY, SETTLEMENT_CHAIN_ID
#                      — passed through to the compose .env on the remote host
#
# Teardown: ./cleanup.sh (uses the STACK_NAME tag to find resources).

set -euo pipefail

STACK_NAME="${STACK_NAME:-katana-tee-stack}"
INSTANCE_TYPE="${INSTANCE_TYPE:-m7a.large}"
ALLOWED_CIDR="${ALLOWED_CIDR:-0.0.0.0/0}"
KATANA_REF="${KATANA_REF:-main}"

: "${AWS_REGION:?AWS_REGION is required (e.g. us-east-2)}"
: "${AWS_KEY_NAME:?AWS_KEY_NAME is required (existing EC2 keypair name)}"
: "${AWS_KEY_PATH:?AWS_KEY_PATH is required (path to matching .pem file)}"

if ! command -v aws >/dev/null 2>&1; then
    echo "aws cli not found. Install: https://docs.aws.amazon.com/cli/latest/userguide/getting-started-install.html" >&2
    exit 1
fi

if ! aws sts get-caller-identity --region "$AWS_REGION" >/dev/null 2>&1; then
    echo "aws cli is not authenticated. Run 'aws configure' or set AWS_PROFILE." >&2
    exit 1
fi

if [ ! -r "$AWS_KEY_PATH" ]; then
    echo "AWS_KEY_PATH ($AWS_KEY_PATH) is not readable." >&2
    exit 1
fi

echo "[aws/provision] Region: $AWS_REGION, Instance: $INSTANCE_TYPE, Stack: $STACK_NAME"

# -----------------------------------------------------------------------------
# 1. Resolve the latest Ubuntu 24.04 LTS amd64 AMI via SSM Public Parameters.
# -----------------------------------------------------------------------------
echo "[aws/provision] Looking up Ubuntu 24.04 LTS AMI..."
AMI_ID=$(aws ssm get-parameters \
    --region "$AWS_REGION" \
    --names "/aws/service/canonical/ubuntu/server/24.04/stable/current/amd64/hvm/ebs-gp3/ami-id" \
    --query 'Parameters[0].Value' \
    --output text)
if [ -z "$AMI_ID" ] || [ "$AMI_ID" = "None" ]; then
    echo "Failed to resolve Ubuntu 24.04 AMI in $AWS_REGION" >&2
    exit 1
fi
echo "[aws/provision] AMI: $AMI_ID"

# -----------------------------------------------------------------------------
# 2. Security group: reuse if tagged, else create.
# -----------------------------------------------------------------------------
SG_NAME="${STACK_NAME}-sg"
SG_ID=$(aws ec2 describe-security-groups \
    --region "$AWS_REGION" \
    --filters "Name=tag:Name,Values=${SG_NAME}" "Name=tag:Stack,Values=${STACK_NAME}" \
    --query 'SecurityGroups[0].GroupId' \
    --output text 2>/dev/null || echo "None")

if [ "$SG_ID" = "None" ] || [ -z "$SG_ID" ]; then
    echo "[aws/provision] Creating security group $SG_NAME..."
    SG_ID=$(aws ec2 create-security-group \
        --region "$AWS_REGION" \
        --group-name "$SG_NAME" \
        --description "Katana TEE stack: SSH + L3 RPC" \
        --tag-specifications "ResourceType=security-group,Tags=[{Key=Name,Value=${SG_NAME}},{Key=Stack,Value=${STACK_NAME}}]" \
        --query 'GroupId' --output text)
    aws ec2 authorize-security-group-ingress --region "$AWS_REGION" --group-id "$SG_ID" \
        --protocol tcp --port 22 --cidr "$ALLOWED_CIDR" >/dev/null
    aws ec2 authorize-security-group-ingress --region "$AWS_REGION" --group-id "$SG_ID" \
        --protocol tcp --port 5050 --cidr "$ALLOWED_CIDR" >/dev/null
    echo "[aws/provision] Created SG $SG_ID with ingress 22, 5050 from $ALLOWED_CIDR"
else
    echo "[aws/provision] Reusing security group $SG_ID"
fi

# -----------------------------------------------------------------------------
# 3. Look up an existing instance tagged with this stack, or create one.
# -----------------------------------------------------------------------------
INSTANCE_ID=$(aws ec2 describe-instances \
    --region "$AWS_REGION" \
    --filters "Name=tag:Stack,Values=${STACK_NAME}" "Name=instance-state-name,Values=pending,running,stopped,stopping" \
    --query 'Reservations[0].Instances[0].InstanceId' \
    --output text 2>/dev/null || echo "None")

if [ "$INSTANCE_ID" = "None" ] || [ -z "$INSTANCE_ID" ]; then
    echo "[aws/provision] Launching $INSTANCE_TYPE with SEV-SNP enabled..."
    INSTANCE_ID=$(aws ec2 run-instances \
        --region "$AWS_REGION" \
        --image-id "$AMI_ID" \
        --instance-type "$INSTANCE_TYPE" \
        --key-name "$AWS_KEY_NAME" \
        --security-group-ids "$SG_ID" \
        --cpu-options "AmdSevSnp=enabled" \
        --block-device-mappings 'DeviceName=/dev/sda1,Ebs={VolumeSize=40,VolumeType=gp3}' \
        --tag-specifications "ResourceType=instance,Tags=[{Key=Name,Value=${STACK_NAME}},{Key=Stack,Value=${STACK_NAME}}]" \
        --query 'Instances[0].InstanceId' --output text)
    echo "[aws/provision] Launched: $INSTANCE_ID"
else
    echo "[aws/provision] Reusing instance $INSTANCE_ID"
    # Ensure it's running.
    STATE=$(aws ec2 describe-instances --region "$AWS_REGION" --instance-ids "$INSTANCE_ID" \
        --query 'Reservations[0].Instances[0].State.Name' --output text)
    if [ "$STATE" != "running" ]; then
        aws ec2 start-instances --region "$AWS_REGION" --instance-ids "$INSTANCE_ID" >/dev/null
    fi
fi

echo "[aws/provision] Waiting for instance to be running + status OK (may take a minute)..."
aws ec2 wait instance-running --region "$AWS_REGION" --instance-ids "$INSTANCE_ID"
aws ec2 wait instance-status-ok --region "$AWS_REGION" --instance-ids "$INSTANCE_ID"

PUBLIC_IP=$(aws ec2 describe-instances --region "$AWS_REGION" --instance-ids "$INSTANCE_ID" \
    --query 'Reservations[0].Instances[0].PublicIpAddress' --output text)
echo "[aws/provision] Instance public IP: $PUBLIC_IP"

# -----------------------------------------------------------------------------
# 4. Bootstrap the host: docker, katana, compose.
# -----------------------------------------------------------------------------
SSH_OPTS=(-o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o ConnectTimeout=10 -i "$AWS_KEY_PATH")

echo "[aws/provision] Waiting for SSH..."
for i in $(seq 1 30); do
    if ssh "${SSH_OPTS[@]}" "ubuntu@${PUBLIC_IP}" "echo ready" >/dev/null 2>&1; then
        break
    fi
    sleep 4
done

# Verify SEV-SNP actually came up. AWS occasionally ignores CpuOptions.AmdSevSnp
# when the AMI or instance family disagrees and gives you a regular VM at the
# same price. This check closes that loop. Override with REQUIRE_SEV=0.
# shellcheck disable=SC1091
. "$(dirname "$0")/../lib/verify-sev.sh"
PROVIDER=aws verify_host_ready "ubuntu@${PUBLIC_IP}"

echo "[aws/provision] Installing docker + compose + katana repo..."
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

# -----------------------------------------------------------------------------
# 5. Write the compose .env on the remote host.
# -----------------------------------------------------------------------------
echo "[aws/provision] Writing remote .env..."
ssh "${SSH_OPTS[@]}" "ubuntu@${PUBLIC_IP}" "cat > ~/katana/.env" <<EOF
SETTLEMENT_RPC_URL=${SETTLEMENT_RPC_URL:-CHANGEME-sepolia-rpc}
SETTLEMENT_ACCOUNT_ADDRESS=${SETTLEMENT_ACCOUNT_ADDRESS:-0xCHANGEME}
SETTLEMENT_ACCOUNT_PRIVATE_KEY=${SETTLEMENT_ACCOUNT_PRIVATE_KEY:-0xCHANGEME}
SETTLEMENT_CHAIN_ID=${SETTLEMENT_CHAIN_ID:-sepolia}
CHAIN_ID=katana_tee
BLOCK_TIME=3000
BATCH_SIZE=1
EOF

# -----------------------------------------------------------------------------
# 6. Bring up the compose stack in the background.
# -----------------------------------------------------------------------------
# NOTE: this currently runs the mock-prove compose. A real-SEV compose
# (docker/tee.compose.yml) is tracked as a v2 follow-up; when it lands, swap
# tee-mock.compose.yml for tee.compose.yml and add /dev/sev-guest to the
# katana service's devices.
echo "[aws/provision] Running docker compose up (detached)..."
ssh "${SSH_OPTS[@]}" "ubuntu@${PUBLIC_IP}" 'bash -s' <<'EOF'
set -euo pipefail
cd ~/katana
sudo -n true 2>/dev/null || { echo "sudo needed"; exit 1; }
sudo docker compose -f docker/tee-mock.compose.yml --env-file .env up --build -d
EOF

echo
echo "========================================================================"
echo "Instance:     $INSTANCE_ID ($PUBLIC_IP)"
echo "RPC:          http://${PUBLIC_IP}:5050"
echo "SSH:          ssh -i $AWS_KEY_PATH ubuntu@${PUBLIC_IP}"
echo "Follow logs:  ssh -i $AWS_KEY_PATH ubuntu@${PUBLIC_IP} 'cd ~/katana && sudo docker compose -f docker/tee-mock.compose.yml --env-file .env logs -f'"
echo "Teardown:     STACK_NAME=$STACK_NAME AWS_REGION=$AWS_REGION ./cleanup.sh"
echo "========================================================================"
