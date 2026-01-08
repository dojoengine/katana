# End-to-End TEE Attestation Testing

This guide explains how to perform end-to-end testing of the Katana TEE build with actual SEV-SNP hardware attestation.

## Prerequisites

### Hardware Requirements

- **AMD EPYC Processor** with SEV-SNP support (Milan or Genoa generation)
- Firmware with SEV-SNP enabled in BIOS
- Linux kernel 6.0+ with SEV-SNP guest support
- QEMU 7.1+ with SEV-SNP support

### Software Requirements

```bash
# Install dependencies
sudo apt-get install -y \
    qemu-system-x86 \
    ovmf \
    curl \
    jq \
    xxd

# Install sev-snp-measure tool
pipx install sev-snp-measure
```

## Test Overview

The end-to-end test validates that:

1. The reproducible build produces the expected measurement
2. The VM boots successfully in SEV-SNP mode
3. The attestation report contains the correct launch measurement
4. The Katana RPC responds correctly from within the TEE

## Step 1: Build Boot Components

```bash
# Build reproducible binary
export SOURCE_DATE_EPOCH=$(git log -1 --format=%ct)
docker build \
    -f reproducible.Dockerfile \
    -t katana-reproducible:test \
    --build-arg SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH} \
    .

# Extract binary
docker create --name katana-extract katana-reproducible:test
docker cp katana-extract:/katana ./katana-binary
docker rm katana-extract

# Build boot components
docker build \
    -f vm-image.Dockerfile \
    -t katana-vm:test \
    --build-arg SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH} \
    .

# Extract components
docker create --name vm-extract katana-vm:test
docker cp vm-extract:/output/vmlinuz ./vmlinuz
docker cp vm-extract:/output/initrd.img ./initrd.img
docker cp vm-extract:/output/ovmf.fd ./ovmf.fd
docker rm vm-extract
```

## Step 2: Calculate Expected Measurement

```bash
# Calculate the expected SEV-SNP measurement
./tee/scripts/calculate-measurement.sh \
    ovmf.fd \
    vmlinuz \
    initrd.img \
    "console=ttyS0 katana.args=--http.addr=0.0.0.0 katana.args=--tee.provider katana.args=sev-snp" \
    4 \
    EPYC-v4

# This creates:
#   - expected-measurement.txt (96 hex characters)
#   - expected-measurement.json (structured metadata)
```

**Note**: The `sev-snp-measure` tool may show a warning about `SNP_KERNEL_HASHES` not being supported. This is expected - the tool cannot pre-calculate the measurement, but real SEV-SNP hardware will measure all components correctly when using direct kernel boot.

## Step 3: Launch VM with SEV-SNP

On SEV-SNP capable hardware, launch the VM with full SEV-SNP protection:

```bash
# Create SEV-SNP guest configuration
sudo qemu-system-x86_64 \
    -enable-kvm \
    -cpu EPYC-v4 \
    -machine q35,confidential-guest-support=sev0,memory-backend=ram1 \
    -object memory-backend-memfd,id=ram1,size=4G,share=true \
    -object sev-snp-guest,id=sev0,cbitpos=51,reduced-phys-bits=1 \
    -smp 4 \
    -m 4G \
    -bios ovmf.fd \
    -kernel vmlinuz \
    -initrd initrd.img \
    -append "console=ttyS0 katana.args=--http.addr=0.0.0.0 katana.args=--tee.provider katana.args=sev-snp" \
    -nographic \
    -net nic,model=virtio \
    -net user,hostfwd=tcp::5050-:5050 \
    -serial file:/tmp/katana-sev.log \
    &

# Wait for VM to boot (check logs)
tail -f /tmp/katana-sev.log
```

**Important parameters**:
- `-object sev-snp-guest`: Enables SEV-SNP protection
- `-kernel`, `-initrd`: Direct kernel boot (ensures measurement)
- `cbitpos=51`: C-bit position for memory encryption
- `reduced-phys-bits=1`: Physical address space reduction

## Step 4: Verify Basic Functionality

Wait for Katana to start, then test the RPC endpoint:

```bash
# Check health endpoint
curl http://localhost:5050/

# Should return: {"health":true}

# Test chain ID
curl -X POST http://localhost:5050 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"starknet_chainId","params":[],"id":1}'

# Should return: {"jsonrpc":"2.0","id":1,"result":"0x4b4154414e41"}
```

## Step 5: Generate and Verify Attestation

Now perform the actual attestation verification:

```bash
# Generate attestation quote and verify measurement
./tee/scripts/verify-attestation.sh \
    http://localhost:5050 \
    expected-measurement.txt
```

### Expected Output (Success)

```
===========================================
SEV-SNP Attestation Verification
===========================================

[INFO] Expected measurement: 435a701116921e3774988cc7411918b91a0b78cf9387109b314d0bf201332d82e945474f7539e93897050eba81d3ee31

[INFO] Requesting attestation quote from Katana...
[SUCCESS] Received attestation quote
[INFO]   Block Number: 0
[INFO]   Block Hash: 0x...
[INFO]   State Root: 0x...
[INFO]   Quote Size: 1184 bytes

[INFO] Extracting launch measurement from attestation report...
[INFO] Actual measurement:   435a701116921e3774988cc7411918b91a0b78cf9387109b314d0bf201332d82e945474f7539e93897050eba81d3ee31

[INFO] Comparing measurements...
[SUCCESS] ✓ Measurements match!

===========================================
[SUCCESS] Attestation verification PASSED
===========================================

[INFO] The running Katana instance was launched with the expected
[INFO] boot components (kernel + initrd + OVMF + cmdline).
[INFO]
[INFO] This proves:
[INFO]   1. The Katana binary matches the reproducible build
[INFO]   2. The kernel and initrd have not been tampered with
[INFO]   3. The launch measurement is cryptographically bound to the build
```

### Expected Output (Failure - Modified Binary)

If someone modified the Katana binary or boot components:

```
[ERROR] ✗ Measurements do NOT match!

Expected: 435a701116921e3774988cc7411918b91a0b78cf9387109b314d0bf201332d82e945474f7539e93897050eba81d3ee31
Actual:   f8a3c94e67d1b22f9c8e7a5d4b3c2f1e0d9c8b7a6e5d4c3b2a1f0e9d8c7b6a5e4d3c2b1a0f9e8d7c6b5a4e3d2c1b0a9

[WARNING] This indicates the running instance was NOT launched with
[WARNING] the expected boot components.
```

## Step 6: Manual Verification (Optional)

For additional verification, manually parse the attestation report:

```bash
# Get the attestation quote
QUOTE=$(curl -s -X POST http://localhost:5050 \
    -H "Content-Type: application/json" \
    -d '{"jsonrpc":"2.0","method":"tee_generateQuote","params":[],"id":1}' \
    | jq -r '.result.quote')

# Extract measurement (offset 0x90, 48 bytes)
MEASUREMENT=$(echo "$QUOTE" | sed 's/^0x//' | xxd -r -p | dd bs=1 skip=144 count=48 2>/dev/null | xxd -p -c 48)

echo "Launch Measurement: $MEASUREMENT"

# Compare with expected
EXPECTED=$(cat expected-measurement.txt)
if [ "$MEASUREMENT" = "$EXPECTED" ]; then
    echo "✓ Measurements match"
else
    echo "✗ Measurements do NOT match"
fi
```

## Understanding the Attestation Report

### AMD SEV-SNP Report Structure

The attestation report is 1184 bytes with the following key fields:

| Offset | Size | Field | Description |
|--------|------|-------|-------------|
| 0x00 | 4 | VERSION | Report structure version |
| 0x04 | 4 | GUEST_SVN | Guest security version number |
| 0x08 | 8 | POLICY | Guest policy flags |
| 0x10 | 16 | FAMILY_ID | Family ID of the guest |
| 0x20 | 16 | IMAGE_ID | Image ID of the guest |
| 0x30 | 4 | VMPL | Virtual machine privilege level |
| 0x50 | 32 | REPORT_ID | Report ID |
| 0x70 | 32 | REPORT_ID_MA | Migration agent report ID |
| **0x90** | **48** | **MEASUREMENT** | **Launch measurement (what we verify!)** |
| 0xC0 | 32 | HOST_DATA | Host-provided data |
| 0xE0 | 32 | ID_KEY_DIGEST | ID key digest |
| 0x100 | 32 | AUTHOR_KEY_DIGEST | Author key digest |
| 0x1C0 | 64 | REPORT_DATA | Report data (Poseidon hash of blockchain state) |
| 0x200 | 4 | CURRENT_TCB | Current TCB version |
| 0x2A0 | 512 | SIGNATURE | RSA signature over the report |

### The Measurement Field (Offset 0x90)

The **MEASUREMENT** field contains a 48-byte (384-bit) SHA-384 hash that covers:

1. **OVMF Firmware** - Initial firmware code
2. **Kernel** - Linux kernel binary
3. **Initrd** - Initial RAM disk (containing Katana)
4. **Kernel Command Line** - Boot parameters

This measurement is calculated by the AMD secure processor before the VM starts executing. Any modification to these components will result in a different measurement.

### The Report Data Field (Offset 0x1C0)

The **REPORT_DATA** field (64 bytes) contains application-specific data. Katana uses this to bind the blockchain state to the attestation:

```
report_data[0:32] = Poseidon(state_root, block_hash)
report_data[32:64] = zeros (reserved)
```

This proves that the attestation was generated at a specific blockchain state.

## Troubleshooting

### Error: "SEV-SNP not supported"

```
[ERROR] RPC error: TEE not available: Failed to initialize SEV-SNP: ...
```

**Cause**: Not running on SEV-SNP capable hardware or SEV-SNP not enabled.

**Solution**:
- Verify hardware supports SEV-SNP: `cat /sys/module/kvm_amd/parameters/sev_snp`
- Enable SEV-SNP in BIOS
- Check kernel config: `CONFIG_AMD_MEM_ENCRYPT_ACTIVE_BY_DEFAULT=y`

### Error: "Device /dev/sev-guest not found"

**Cause**: SEV-SNP guest driver not loaded.

**Solution**:
```bash
# Load the module
sudo modprobe sev-guest

# Verify device exists
ls -l /dev/sev-guest
```

### Measurement Mismatch

**Cause**: The boot components used don't match the expected measurement.

**Possible reasons**:
1. Different kernel version
2. Modified initrd (different Katana binary)
3. Different OVMF firmware
4. Different kernel command line
5. Outdated expected-measurement.txt

**Solution**:
- Rebuild all components with the same `SOURCE_DATE_EPOCH`
- Recalculate the expected measurement
- Ensure kernel cmdline matches exactly (including Katana args order)

### QEMU Fails to Start

**Cause**: Incorrect SEV-SNP parameters or insufficient resources.

**Solution**:
- Check QEMU version supports SEV-SNP (7.1+)
- Verify sufficient memory (4GB minimum)
- Check kernel messages: `dmesg | grep -i sev`
- Ensure OVMF supports SEV-SNP

## Security Notes

### What This Test Proves

✅ **The Katana binary is authentic** - Matches the reproducible build
✅ **The boot stack is unmodified** - Kernel, initrd, OVMF unchanged
✅ **The measurement is cryptographically bound** - Can't be forged
✅ **The VM is running in a TEE** - AMD hardware-enforced isolation

### What This Test Does NOT Prove

❌ **The build infrastructure was secure** - Trust in GitHub Actions required
❌ **The source code is bug-free** - Code review still necessary
❌ **Runtime behavior** - Only proves initial state, not execution
❌ **Network security** - Doesn't validate TLS or network isolation

### Chain of Trust

```
Source Code (GitHub)
    ↓ (GitHub Actions)
Reproducible Build
    ↓ (Docker + SOURCE_DATE_EPOCH)
Katana Binary (SHA-384)
    ↓ (Embedded in Initrd)
Boot Components (kernel, initrd, OVMF)
    ↓ (Direct Kernel Boot)
AMD Secure Processor
    ↓ (Calculate Hash)
Launch Measurement (SHA-384)
    ↓ (Signed by CPU)
Attestation Report
    ↓ (Verify Signature + Measurement)
Trusted State ✓
```

## Continuous Integration

This test should be integrated into CI/CD for every release:

```yaml
- name: E2E Attestation Test (SEV-SNP Hardware)
  runs-on: [self-hosted, sev-snp]
  steps:
    - name: Launch VM with SEV-SNP
      run: |
        qemu-system-x86_64 -enable-kvm ... &
        sleep 30

    - name: Verify Attestation
      run: |
        ./tee/scripts/verify-attestation.sh http://localhost:5050 expected-measurement.txt
```

## References

- [AMD SEV-SNP Specification](https://www.amd.com/system/files/TechDocs/56860.pdf)
- [sev-snp-measure Tool](https://github.com/virtee/sev-snp-measure)
- [QEMU SEV-SNP Documentation](https://qemu.readthedocs.io/en/latest/system/i386/amd-memory-encryption.html)
- [Automata Network SEV-SNP SDK](https://github.com/automata-network/amd-sev-snp-attestation-sdk)
