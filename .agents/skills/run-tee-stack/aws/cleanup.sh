#!/usr/bin/env bash
#
# Tears down everything provisioned by ./provision.sh. Matches by the
# Stack=$STACK_NAME tag.
#
# Usage:
#   AWS_REGION=us-east-2 STACK_NAME=katana-tee-stack ./cleanup.sh

set -euo pipefail

STACK_NAME="${STACK_NAME:-katana-tee-stack}"
: "${AWS_REGION:?AWS_REGION is required}"

echo "[aws/cleanup] Region: $AWS_REGION, Stack: $STACK_NAME"

INSTANCE_IDS=$(aws ec2 describe-instances \
    --region "$AWS_REGION" \
    --filters "Name=tag:Stack,Values=${STACK_NAME}" "Name=instance-state-name,Values=pending,running,stopped,stopping" \
    --query 'Reservations[].Instances[].InstanceId' --output text)

if [ -n "$INSTANCE_IDS" ]; then
    echo "[aws/cleanup] Terminating: $INSTANCE_IDS"
    # shellcheck disable=SC2086
    aws ec2 terminate-instances --region "$AWS_REGION" --instance-ids $INSTANCE_IDS >/dev/null
    # shellcheck disable=SC2086
    aws ec2 wait instance-terminated --region "$AWS_REGION" --instance-ids $INSTANCE_IDS
else
    echo "[aws/cleanup] No running instances."
fi

SG_ID=$(aws ec2 describe-security-groups \
    --region "$AWS_REGION" \
    --filters "Name=tag:Stack,Values=${STACK_NAME}" \
    --query 'SecurityGroups[0].GroupId' --output text 2>/dev/null || echo "None")
if [ "$SG_ID" != "None" ] && [ -n "$SG_ID" ]; then
    echo "[aws/cleanup] Deleting security group $SG_ID"
    aws ec2 delete-security-group --region "$AWS_REGION" --group-id "$SG_ID"
fi

echo "[aws/cleanup] done."
