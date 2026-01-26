# Testing on GCP Confidential VMs

This guide explains how to test Katana TEE attestation on Google Cloud Platform using Confidential VMs with AMD SEV.

## Overview

Google Cloud Confidential VMs provide AMD SEV (Secure Encrypted Virtualization) support, allowing you to run Katana with hardware-based memory encryption and attestation. Instead of running QEMU inside GCP, **Katana runs directly on the GCP Confidential VM**, which itself is an SEV guest.

## GCP Confidential VM Support

### Current Status (as of 2025)

- **Available**: AMD SEV and SEV-ES support
- **Instance Types**: N2D (AMD EPYC Milan/Rome)
- **SEV-SNP**: Support varies by region and may require specific instance types

### Important Distinctions

**Traditional Deployment (Bare Metal)**:
```
Bare Metal Hardware
  └─ QEMU (launches SEV-SNP guest)
      └─ Katana VM (with our boot components)
```

**GCP Confidential VM Deployment**:
```
GCP Hypervisor
  └─ Confidential VM (SEV guest) ← This is where Katana runs directly
      └─ Katana process (with --tee.provider sev-snp)
```

## Prerequisites

### GCP Setup

1. **Enable Confidential Computing API**:
   ```bash
   gcloud services enable compute.googleapis.com
   ```

2. **Check Available Regions**:
   Confidential VMs are available in most regions, but verify:
   ```bash
   gcloud compute machine-types list \
       --filter="name:n2d-standard" \
       --zones=us-central1-a
   ```

## Step 1: Create a Confidential VM

### Using gcloud CLI

```bash
# Create an N2D confidential VM
gcloud compute instances create katana-tee-test \
    --zone=us-central1-a \
    --machine-type=n2d-standard-4 \
    --confidential-compute \
    --maintenance-policy=TERMINATE \
    --image-family=ubuntu-2404-lts-amd64 \
    --image-project=ubuntu-os-cloud \
    --boot-disk-size=50GB \
    --boot-disk-type=pd-balanced \
    --scopes=cloud-platform \
    --metadata=startup-script='#!/bin/bash
    apt-get update
    apt-get install -y git build-essential curl
    '
```

### Using Console

1. Go to **Compute Engine** → **VM instances**
2. Click **Create Instance**
3. Configure:
   - **Name**: `katana-tee-test`
   - **Region**: `us-central1` (or your preferred region)
   - **Machine type**: **N2D series** → `n2d-standard-4` (4 vCPUs, 16 GB)
   - **Confidential VM service**: **Enable**
   - **Boot disk**: Ubuntu 24.04 LTS, 50 GB
4. Click **Create**

### Key Settings

- **Must use N2D machine type** (AMD EPYC processors)
- **Confidential VM must be enabled**
- **Maintenance policy must be TERMINATE** (required for confidential VMs)

## Step 2: Connect and Verify SEV Support

```bash
# SSH to the instance
gcloud compute ssh katana-tee-test --zone=us-central1-a

# Check if running in confidential VM
ls -l /dev/sev-guest
# Should show: crw------- 1 root root 10, 124 Jan 8 00:00 /dev/sev-guest

# Check SEV support
sudo dmesg | grep -i sev
# Should show SEV-related messages

# Verify it's AMD processor
cat /proc/cpuinfo | grep "model name" | head -1
# Should show: AMD EPYC
```

## Step 3: Install Dependencies

```bash
# Update system
sudo apt-get update && sudo apt-get upgrade -y

# Install build tools
sudo apt-get install -y \
    build-essential \
    git \
    curl \
    jq \
    xxd \
    pkg-config \
    libssl-dev

# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source "$HOME/.cargo/env"

# Install Docker (for building reproducible binary)
curl -fsSL https://get.docker.com -o get-docker.sh
sudo sh get-docker.sh
sudo usermod -aG docker $USER
newgrp docker
```

## Step 4: Clone and Build Katana

```bash
# Clone repository
git clone https://github.com/dojoengine/katana.git
cd katana
git checkout tee/reproducible-builds

# Build reproducible binary
export SOURCE_DATE_EPOCH=$(git log -1 --format=%ct)
docker build \
    -f reproducible.Dockerfile \
    -t katana-reproducible:gcp \
    --build-arg SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH} \
    .

# Extract binary
docker create --name katana-extract katana-reproducible:gcp
docker cp katana-extract:/katana ./katana-tee
docker rm katana-extract

# Make executable
chmod +x katana-tee
```

## Step 5: Calculate Expected Measurement (Important!)

**NOTE**: Since we're running Katana directly on the GCP VM (not in a nested VM with QEMU), we cannot pre-calculate the exact measurement. The GCP hypervisor measures the confidential VM's initial state, not our Katana binary specifically.

For GCP testing, we'll:
1. **Skip pre-calculated measurements** (not applicable to this deployment model)
2. **Obtain the actual measurement from the attestation report** after Katana starts
3. **Record this measurement** for future verification

This is different from the bare-metal approach where we control the entire boot stack.

## Step 6: Run Katana with TEE Support

```bash
# Run Katana with SEV-SNP provider
./katana-tee \
    --http.addr 0.0.0.0 \
    --http.port 5050 \
    --tee.provider sev-snp \
    --dev \
    > katana.log 2>&1 &

# Wait for startup
sleep 10

# Check logs
tail -50 katana.log

# Should see:
# [INFO] TEE API initialized provider_type=SEV-SNP
# [INFO] RPC server started addr=0.0.0.0:5050
```

## Step 7: Test Attestation

### Basic Health Check

```bash
# Test RPC endpoint
curl http://localhost:5050/

# Should return: {"health":true}
```

### Generate Attestation Quote

```bash
# Call tee_generateQuote
curl -X POST http://localhost:5050 \
    -H "Content-Type: application/json" \
    -d '{
        "jsonrpc": "2.0",
        "method": "tee_generateQuote",
        "params": [],
        "id": 1
    }' | jq '.'
```

**Expected Response**:
```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "quote": "0x010000000400000007000000...",
    "stateRoot": "0x...",
    "blockHash": "0x...",
    "blockNumber": 0
  }
}
```

### Extract and Record Measurement

```bash
# Get attestation quote
QUOTE=$(curl -s -X POST http://localhost:5050 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"tee_generateQuote","params":[],"id":1}' \
    | jq -r '.result.quote')

# Extract measurement (offset 0x90, 48 bytes)
MEASUREMENT=$(echo "$QUOTE" | sed 's/^0x//' | xxd -r -p | dd bs=1 skip=144 count=48 2>/dev/null | xxd -p -c 48)

echo "Launch Measurement: $MEASUREMENT"

# Save for future reference
echo "$MEASUREMENT" > gcp-measurement-baseline.txt
echo "Measurement saved to gcp-measurement-baseline.txt"
```

## Step 8: Verify Attestation Report

```bash
# Create verification script for GCP
cat > verify-gcp-attestation.sh << 'EOF'
#!/bin/bash
set -e

QUOTE_HEX=$(curl -s -X POST http://localhost:5050 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"tee_generateQuote","params":[],"id":1}' \
    | jq -r '.result.quote')

# Extract measurement
MEASUREMENT=$(echo "$QUOTE_HEX" | sed 's/^0x//' | xxd -r -p | dd bs=1 skip=144 count=48 2>/dev/null | xxd -p -c 48)

echo "==================================="
echo "GCP Confidential VM Attestation"
echo "==================================="
echo ""
echo "Measurement: $MEASUREMENT"
echo ""

# Extract report data (blockchain state commitment)
REPORT_DATA=$(echo "$QUOTE_HEX" | sed 's/^0x//' | xxd -r -p | dd bs=1 skip=448 count=64 2>/dev/null | xxd -p -c 64)
echo "Report Data: $REPORT_DATA"
echo ""

# If baseline exists, compare
if [ -f gcp-measurement-baseline.txt ]; then
    BASELINE=$(cat gcp-measurement-baseline.txt)
    if [ "$MEASUREMENT" = "$BASELINE" ]; then
        echo "✓ Measurement matches baseline"
    else
        echo "✗ Measurement DOES NOT match baseline!"
        echo "  Expected: $BASELINE"
        echo "  Actual:   $MEASUREMENT"
    fi
else
    echo "No baseline found. This measurement can be used as baseline."
fi
EOF

chmod +x verify-gcp-attestation.sh
./verify-gcp-attestation.sh
```

## Understanding GCP Attestation

### What the Measurement Includes

In a GCP Confidential VM, the measurement covers:
- **GCP's VM firmware** (GCP-provided)
- **Kernel** (from Ubuntu image)
- **Initial RAM disk** (from Ubuntu image)
- **Boot configuration** (GCP-managed)

**Important**: The measurement does **NOT** include your Katana binary directly because Katana runs as a process inside the VM, not as part of the boot image.

### What You Can Verify

✅ **The VM is running in a confidential environment** - Hardware-enforced isolation
✅ **Memory is encrypted by AMD SEV** - Protected from hypervisor access
✅ **Attestation report is authentic** - Signed by AMD secure processor
✅ **Blockchain state is bound to attestation** - Report data contains Poseidon(state_root, block_hash)

❌ **The Katana binary itself is NOT measured** - It's a runtime process, not part of boot
❌ **Cannot pre-calculate expected measurement** - GCP controls the boot stack

### Deployment Model Comparison

| Aspect | Bare Metal (QEMU) | GCP Confidential VM |
|--------|-------------------|---------------------|
| **Boot Control** | Full (custom kernel, initrd) | Limited (GCP-managed) |
| **Binary Measurement** | Yes (in initrd) | No (runtime process) |
| **Pre-calculated Measurement** | Yes | No |
| **Memory Encryption** | Yes (SEV-SNP) | Yes (SEV/SEV-ES) |
| **Attestation Available** | Yes | Yes |
| **Use Case** | Maximum trust, custom builds | Quick testing, cloud-native |

## Step 9: Continuous Verification

For ongoing verification in GCP:

```bash
# Create monitoring script
cat > monitor-attestation.sh << 'EOF'
#!/bin/bash
while true; do
    echo "[$(date)] Checking attestation..."
    ./verify-gcp-attestation.sh
    echo ""
    sleep 300  # Every 5 minutes
done
EOF

chmod +x monitor-attestation.sh

# Run in background
nohup ./monitor-attestation.sh > attestation-monitor.log 2>&1 &
```

## Limitations on GCP

### What This DOESN'T Prove

Since Katana runs as a userspace process (not embedded in boot):

❌ **Binary authenticity** - Cannot verify Katana binary matches reproducible build
❌ **Boot-time measurement** - Katana loaded after boot, not during
❌ **Complete chain of trust** - GCP controls boot stack

### What This DOES Prove

✅ **Confidential execution** - Process runs in encrypted memory
✅ **Hardware isolation** - AMD SEV protects from hypervisor
✅ **Attestation capability** - Can generate hardware-backed quotes
✅ **State binding** - Blockchain state is cryptographically bound

## Alternative: Full Trust Model on GCP

To achieve a similar trust level as bare metal, you would need:

1. **Custom GCP Image** with embedded Katana in initrd (not currently supported)
2. **Bring Your Own Key (BYOK)** for image signing
3. **Confidential GKE** with custom node images
4. **Bare Metal Solution** from GCP partners

## Cost Estimation

**N2D Standard-4 Confidential VM**:
- 4 vCPUs, 16 GB RAM
- ~$0.20/hour (~$150/month)
- Plus network egress and storage

## Cleanup

```bash
# Stop Katana
pkill -f katana-tee

# From your local machine, delete the instance
gcloud compute instances delete katana-tee-test \
    --zone=us-central1-a \
    --quiet
```

## Recommendations

### For Quick Testing
✅ **Use GCP Confidential VMs** - Fast, easy, proves confidential execution works

### For Production/Maximum Trust
✅ **Use Bare Metal with QEMU** - Full control, measured boot, complete chain of trust

### Hybrid Approach
1. Develop and test on GCP (confidential execution)
2. Deploy to bare metal (measured boot + reproducible builds)
3. Use both measurements for different trust models

## Next Steps

If you need the full measured boot trust model on cloud infrastructure:

1. **AWS Nitro Enclaves** - Provides measured boot for enclave images
2. **Azure Confidential VMs** - Similar to GCP, SEV-SNP support
3. **Bare Metal Cloud** - OVHcloud, Equinix Metal, etc. with SEV-SNP

## References

- [GCP Confidential VMs](https://cloud.google.com/confidential-computing)
- [AMD SEV on GCP](https://cloud.google.com/compute/confidential-vm/docs/about-cvm)
- [N2D Machine Types](https://cloud.google.com/compute/docs/general-purpose-machines#n2d_machines)
