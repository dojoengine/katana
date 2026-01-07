#!/bin/bash
set -euo pipefail

# SEV-SNP Measurement Calculator for Katana VM
# Supports both UEFI boot (from disk) and direct kernel boot modes

OVMF=$1
KERNEL=${2:-}
INITRD=${3:-}
CMDLINE=${4:-}
VCPUS=${5:-4}
VCPU_TYPE=${6:-EPYC-v4}
OUTPUT_DIR=${7:-.}

echo "=========================================="
echo "SEV-SNP Measurement Calculator"
echo "=========================================="
echo "OVMF:      $OVMF"
echo "Kernel:    ${KERNEL:-<none - UEFI boot>}"
echo "Initrd:    ${INITRD:-<none - UEFI boot>}"
echo "Cmdline:   ${CMDLINE:-<none - UEFI boot>}"
echo "VCPUs:     $VCPUS"
echo "VCPU Type: $VCPU_TYPE"
echo "=========================================="
echo ""

# Verify OVMF exists
if [[ ! -f "$OVMF" ]]; then
    echo "ERROR: OVMF file not found: $OVMF"
    exit 1
fi

# Determine boot mode
if [[ -n "$KERNEL" && -n "$INITRD" ]]; then
    BOOT_MODE="direct"
    echo "Boot mode: Direct kernel boot (kernel + initrd)"

    # Verify kernel and initrd exist
    if [[ ! -f "$KERNEL" ]]; then
        echo "ERROR: Kernel file not found: $KERNEL"
        exit 1
    fi
    if [[ ! -f "$INITRD" ]]; then
        echo "ERROR: Initrd file not found: $INITRD"
        exit 1
    fi

    # Calculate measurement with kernel/initrd
    echo "Calculating measurement with direct kernel boot..."
    MEASUREMENT=$(sev-snp-measure \
        --mode snp \
        --ovmf "$OVMF" \
        --kernel "$KERNEL" \
        --initrd "$INITRD" \
        --append "$CMDLINE" \
        --vcpus "$VCPUS" \
        --vcpu-type "$VCPU_TYPE" \
        --output-format hex 2>&1) || {
        echo ""
        echo "WARNING: Direct kernel boot measurement failed."
        echo "This OVMF firmware may not support SNP_KERNEL_HASHES."
        echo "Falling back to UEFI boot measurement..."
        echo ""
        BOOT_MODE="uefi"
        MEASUREMENT=$(sev-snp-measure \
            --mode snp \
            --ovmf "$OVMF" \
            --vcpus "$VCPUS" \
            --vcpu-type "$VCPU_TYPE" \
            --output-format hex)
    }
else
    BOOT_MODE="uefi"
    echo "Boot mode: UEFI boot from disk"

    # Calculate measurement with OVMF only
    echo "Calculating measurement with UEFI boot..."
    MEASUREMENT=$(sev-snp-measure \
        --mode snp \
        --ovmf "$OVMF" \
        --vcpus "$VCPUS" \
        --vcpu-type "$VCPU_TYPE" \
        --output-format hex)
fi

echo "✓ Measurement calculated successfully"
echo ""

# Save as hex
echo "$MEASUREMENT" > "$OUTPUT_DIR/expected-measurement.txt"

# Create JSON format for structured output
if [[ "$BOOT_MODE" == "direct" ]]; then
    cat > "$OUTPUT_DIR/expected-measurement.json" <<EOF
{
  "measurement": "$MEASUREMENT",
  "boot_mode": "direct_kernel",
  "components": {
    "ovmf": "$OVMF",
    "kernel": "$KERNEL",
    "initrd": "$INITRD",
    "cmdline": "$CMDLINE"
  },
  "vm_config": {
    "vcpus": $VCPUS,
    "vcpu_type": "$VCPU_TYPE"
  },
  "notes": "Measurement includes kernel, initrd, and cmdline in OVMF launch digest"
}
EOF
else
    cat > "$OUTPUT_DIR/expected-measurement.json" <<EOF
{
  "measurement": "$MEASUREMENT",
  "boot_mode": "uefi_disk",
  "components": {
    "ovmf": "$OVMF"
  },
  "vm_config": {
    "vcpus": $VCPUS,
    "vcpu_type": "$VCPU_TYPE"
  },
  "notes": "UEFI boot measurement - disk contents not included in initial measurement"
}
EOF
fi

echo "=========================================="
echo "✓ Measurement Complete"
echo "=========================================="
echo "Mode:        $BOOT_MODE"
echo "Measurement: $MEASUREMENT"
echo ""
echo "Saved to:"
echo "  - $OUTPUT_DIR/expected-measurement.txt (hex)"
echo "  - $OUTPUT_DIR/expected-measurement.json (structured)"
echo "=========================================="
