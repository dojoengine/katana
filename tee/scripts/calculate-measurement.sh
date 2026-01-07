#!/bin/bash
set -euo pipefail

OVMF=$1
KERNEL=$2
INITRD=$3
CMDLINE=$4
VCPUS=${5:-4}
VCPU_TYPE=${6:-EPYC-v4}

# Calculate measurement using sev-snp-measure (Python)
MEASUREMENT=$(sev-snp-measure \
    --mode snp \
    --ovmf "$OVMF" \
    --kernel "$KERNEL" \
    --initrd "$INITRD" \
    --append "$CMDLINE" \
    --vcpus "$VCPUS" \
    --vcpu-type "$VCPU_TYPE" \
    --output-format hex)

# Save as hex
echo "$MEASUREMENT" > expected-measurement.txt

# Create JSON format for structured output
cat > expected-measurement.json <<EOF
{
  "measurement": "$MEASUREMENT",
  "components": {
    "ovmf": "$OVMF",
    "kernel": "$KERNEL",
    "initrd": "$INITRD",
    "cmdline": "$CMDLINE"
  },
  "vm_config": {
    "vcpus": $VCPUS,
    "vcpu_type": "$VCPU_TYPE"
  }
}
EOF

echo "Measurement: $MEASUREMENT"
