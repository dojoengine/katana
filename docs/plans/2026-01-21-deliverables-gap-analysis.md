# Deliverables Gap Analysis

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Identify gaps between deliverables.md and current repo state, then complete missing items.

**Architecture:** The project uses Cairo contracts verified via Garaga SP1 Groth16 verifier, Rust clients for proof generation and RPC, and integration scripts for deployment.

**Tech Stack:** Cairo 2.x, Rust, SP1 zkVM, Garaga, Starknet Foundry

---

## Summary: Current State vs Deliverables

| Deliverable | Status | Gap |
|-------------|--------|-----|
| **2.1.1** Primary Contract (SP1 verification) | ✅ Complete | None |
| **2.1.2** Cairo Library (attestation utils) | ✅ Complete | None |
| **2.1.3** Secondary Contract (Katana TEE) | ✅ Complete | None |
| **2.2** Rust Crate | ✅ Complete | None |
| **2.3** E2E Demo on Testnet | ⚠️ Partial | Missing testnet deployment + documented example |
| **2.4** Upstream Improvements | ✅ Complete | All integrated as submodules |

---

## Detailed Gap Analysis

### ✅ 2.1 Starknet Smart Contract Components — COMPLETE

**Primary Contract (`amd_tee_registry`):**
- ✅ SP1 proof verification via Garaga `verify_sp1_groth16_proof_bn254`
- ✅ Certificate chain validation (root + intermediate)
- ✅ Processor type validation (Milan, Genoa, Bergamo, Siena)
- ✅ Timestamp validation with `max_time_diff`
- ✅ On-chain caching via `CertCacheComponent`
- ✅ SP1 program ID verification

**Cairo Library (`katana_report_utils`):**
- ✅ Poseidon hash verification of `report_data`
- ✅ `state_root` + `block_hash` commitment validation
- ✅ Endianness conversion utilities

**Secondary Contract (`katana_tee`):**
- ✅ Stores registry address
- ✅ `verify_sp1_proof()` delegation
- ✅ `verify_and_update_state()` with Katana report validation
- ✅ `get_latest_state()` exposes sequencer state

### ✅ 2.2 Rust Crate — COMPLETE

**katana_tee_client:**
- ✅ `tee_generateQuote` RPC client
- ✅ SP1 proof generation wrapper
- ✅ Starknet transaction building
- ✅ Full CLI with `fetch`, `prove`, `pipeline` commands

**amd_tee_registry_client:**
- ✅ AMD KDS certificate fetching
- ✅ SP1 Groth16 proof generation
- ✅ Starknet calldata serialization
- ✅ Cache-aware proving (queries trusted prefix)

### ⚠️ 2.3 End-to-End Demo on Testnet — GAPS EXIST

**What EXISTS:**
- ✅ Local devnet integration tests (`tests/deployment/run_integration_tests.sh`)
- ✅ Full pipeline CLI (`katana_tee_client pipeline`)
- ✅ snfoundry.toml configured for sepolia/mainnet
- ✅ Environment templates (`.env.example`)

**What's MISSING:**

1. **No Testnet Deployment Artifacts**
   - No `deployments/sepolia.json` with deployed contract addresses
   - No documented Garaga SP1 verifier class hash for Sepolia
   - No documented SP1 program ID for the AMD SEV-SNP program

2. **No Testnet Deployment Script**
   - `run_integration_tests.sh` only targets devnet
   - Need sepolia-specific deployment with:
     - Real trusted certificates
     - Real root certificates per processor model
     - Actual Garaga verifier class hash
     - Actual SP1 program vkey hash

3. **No Documented E2E Example**
   - Need step-by-step guide: "Verify Katana TEE on Sepolia"
   - Include actual transaction hashes as proof of completion

### ✅ 2.4 Upstream Improvements — COMPLETE

All upstream dependencies integrated as submodules:
- ✅ `crates/katana/` — Katana with TEE support
- ✅ `crates/garaga/` — SP1 Groth16 verifier
- ✅ `crates/amd-sev-snp-attestation-sdk/` — AMD attestation SDK
- ✅ `crates/starknet-rust/` — Starknet RPC client

---

## Tasks to Complete Deliverables

### Task 1: Gather Testnet Prerequisites

**Files:**
- Create: `deployments/sepolia-prerequisites.json`

**Step 1: Get SP1 Program ID**

The SP1 program ID (vkey hash) is computed from the AMD SEV-SNP attestation program.

```bash
cd crates/amd-sev-snp-attestation-sdk
cargo run --bin snp-attest-cli -- program-id --sp1
```

Expected output:
```
ProgramID (Onchain Representation): 0x...
ProgramID (Offchain Representation): 0x...
```

Save the "Onchain Representation" as `sp1_program_id`.

**Step 2: Get Garaga SP1 Verifier Class Hash**

Check Garaga documentation or deployed contracts for the SP1 Groth16 verifier class hash on Sepolia.

```bash
# If not already deployed, generate and declare:
cd crates/garaga
python -m garaga gen --system sp1 --curve bn254 --out ./sp1_verifier
cd sp1_verifier
scarb build
sncast --account sepolia declare --contract-name Sp1VerifierBN254
```

**Step 3: Document AMD Root Certificates**

Obtain root certificate hashes for each processor type from AMD KDS:
- Milan: `https://kdsintf.amd.com/vcek/v1/Milan/cert_chain`
- Genoa: `https://kdsintf.amd.com/vcek/v1/Genoa/cert_chain`
- Bergamo: `https://kdsintf.amd.com/vcek/v1/Bergamo/cert_chain`
- Siena: `https://kdsintf.amd.com/vcek/v1/Siena/cert_chain`

Hash each root certificate (SHA256 → u256) for contract initialization.

**Step 4: Save prerequisites**

```json
{
  "network": "sepolia",
  "garaga_sp1_verifier_class_hash": "0x...",
  "sp1_program_id": "0x...",
  "max_time_diff": 86400,
  "root_certs": {
    "milan": "0x...",
    "genoa": "0x...",
    "bergamo": "0x...",
    "siena": "0x..."
  },
  "initial_trusted_certs": []
}
```

---

### Task 2: Create Testnet Deployment Script

**Files:**
- Create: `tests/deployment/deploy_sepolia.sh`

**Step 1: Write deployment script**

```bash
#!/usr/bin/env bash
# Deploy katana-tee contracts to Sepolia testnet
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
DEPLOYMENTS_DIR="$PROJECT_ROOT/deployments"
PREREQ_FILE="$DEPLOYMENTS_DIR/sepolia-prerequisites.json"

# Verify prerequisites exist
if [[ ! -f "$PREREQ_FILE" ]]; then
    echo "ERROR: $PREREQ_FILE not found. Run Task 1 first."
    exit 1
fi

# Load prerequisites
VERIFIER_CLASS_HASH=$(jq -r '.garaga_sp1_verifier_class_hash' "$PREREQ_FILE")
SP1_PROGRAM_ID=$(jq -r '.sp1_program_id' "$PREREQ_FILE")
MAX_TIME_DIFF=$(jq -r '.max_time_diff' "$PREREQ_FILE")

# Extract root certs as arrays
PROCESSOR_MODELS="0 1 2 3"  # Milan, Genoa, Bergamo, Siena
ROOT_CERTS=$(jq -r '[.root_certs.milan, .root_certs.genoa, .root_certs.bergamo, .root_certs.siena] | join(" ")' "$PREREQ_FILE")

echo "=== Deploying to Sepolia ==="
echo "Verifier: $VERIFIER_CLASS_HASH"
echo "Program ID: $SP1_PROGRAM_ID"

# Build contracts
cd "$PROJECT_ROOT/contracts/amd_tee_registry"
scarb build

cd "$PROJECT_ROOT/contracts/katana_tee"
scarb build

# Declare AMDTEERegistry
cd "$PROJECT_ROOT/contracts/amd_tee_registry"
AMD_CLASS_HASH=$(sncast --profile sepolia declare \
    --contract-name AMDTEERegistry \
    --package amd_tee_registry 2>&1 | grep -oP 'class_hash:\s*\K0x[a-fA-F0-9]+' || \
    sncast utils class-hash --contract-name AMDTEERegistry --package amd_tee_registry | grep -oP '0x[a-fA-F0-9]+')

echo "AMDTEERegistry class hash: $AMD_CLASS_HASH"

# Build constructor calldata for AMDTEERegistry
# constructor(verifier_class_hash, sp1_program_id, max_time_diff, trusted_certs, processor_models, root_certs)
# sp1_program_id is u256 (2 felts: low, high)
SP1_LOW=$(python3 -c "print(hex(int('$SP1_PROGRAM_ID', 16) & ((1 << 128) - 1)))")
SP1_HIGH=$(python3 -c "print(hex(int('$SP1_PROGRAM_ID', 16) >> 128))")

# Deploy AMDTEERegistry
AMD_ADDRESS=$(sncast --profile sepolia deploy \
    --class-hash "$AMD_CLASS_HASH" \
    --constructor-calldata "$VERIFIER_CLASS_HASH $SP1_LOW $SP1_HIGH $MAX_TIME_DIFF 0 4 0 1 2 3 4 $ROOT_CERTS" \
    2>&1 | grep -oP 'Contract Address:\s*\K0x[a-fA-F0-9]+')

echo "AMDTEERegistry deployed: $AMD_ADDRESS"

# Declare KatanaTee
cd "$PROJECT_ROOT/contracts/katana_tee"
KATANA_CLASS_HASH=$(sncast --profile sepolia declare \
    --contract-name KatanaTee \
    --package katana_tee 2>&1 | grep -oP 'class_hash:\s*\K0x[a-fA-F0-9]+' || \
    sncast utils class-hash --contract-name KatanaTee --package katana_tee | grep -oP '0x[a-fA-F0-9]+')

echo "KatanaTee class hash: $KATANA_CLASS_HASH"

# Deploy KatanaTee
KATANA_ADDRESS=$(sncast --profile sepolia deploy \
    --class-hash "$KATANA_CLASS_HASH" \
    --constructor-calldata "$AMD_ADDRESS" \
    2>&1 | grep -oP 'Contract Address:\s*\K0x[a-fA-F0-9]+')

echo "KatanaTee deployed: $KATANA_ADDRESS"

# Save deployment
cat > "$DEPLOYMENTS_DIR/sepolia.json" << EOF
{
  "network": "sepolia",
  "timestamp": "$(date -Iseconds)",
  "contracts": {
    "garaga_sp1_verifier": {
      "class_hash": "$VERIFIER_CLASS_HASH"
    },
    "amd_tee_registry": {
      "class_hash": "$AMD_CLASS_HASH",
      "address": "$AMD_ADDRESS"
    },
    "katana_tee": {
      "class_hash": "$KATANA_CLASS_HASH",
      "address": "$KATANA_ADDRESS"
    }
  },
  "config": {
    "sp1_program_id": "$SP1_PROGRAM_ID",
    "max_time_diff": $MAX_TIME_DIFF
  }
}
EOF

echo ""
echo "=== Deployment Complete ==="
echo "Saved to: $DEPLOYMENTS_DIR/sepolia.json"
```

**Step 2: Make executable**

```bash
chmod +x tests/deployment/deploy_sepolia.sh
```

---

### Task 3: Create E2E Demo Documentation

**Files:**
- Create: `docs/e2e-demo-sepolia.md`

**Step 1: Write E2E guide**

```markdown
# End-to-End Demo: Verify Katana TEE on Sepolia

This guide demonstrates the complete flow of:
1. Fetching TEE attestation from a Katana node
2. Generating a ZK proof of attestation validity
3. Verifying the proof on-chain and updating sequencer state

## Prerequisites

- Deployed contracts (see `deployments/sepolia.json`)
- Access to Katana TEE RPC endpoint
- SP1 Prover Network API key (for `SP1_PROVER=network`)
- Funded Starknet account on Sepolia

## Step 1: Configure Environment

```bash
cp .env.example .env

# Edit .env:
KATANA_RPC_URL=http://<katana-tee-host>:5050
STARKNET_NETWORK=sepolia
STARKNET_RPC_URL_SEPOLIA=https://starknet-sepolia.public.blastapi.io/rpc/v0_7
STARKNET_ACCOUNT=<your-sepolia-account>
STARKNET_PRIVATE_KEY=<your-private-key>
SP1_PROVER=network
SP1_PRIVATE_KEY=<your-sp1-prover-key>
```

## Step 2: Fetch Attestation

```bash
cargo run --bin katana-tee-cli -- fetch --output attestation.json
```

Output:
```json
{
  "quote": "0x...",
  "state_root": "0x...",
  "block_hash": "0x...",
  "block_number": 42
}
```

## Step 3: Generate ZK Proof

```bash
cargo run --bin katana-tee-cli -- prove \
  --input attestation.json \
  --output proof.json
```

This:
1. Queries Starknet for cached certificates
2. Fetches any missing certificates from AMD KDS
3. Generates SP1 Groth16 proof via network prover
4. Outputs Starknet-ready calldata

## Step 4: Verify On-Chain

```bash
cargo run --bin katana-tee-cli -- pipeline \
  --katana-rpc-url "$KATANA_RPC_URL" \
  --katana-tee-address "$(jq -r '.contracts.katana_tee.address' deployments/sepolia.json)"
```

Or invoke manually:
```bash
sncast --profile sepolia invoke \
  --contract-address "$(jq -r '.contracts.katana_tee.address' deployments/sepolia.json)" \
  --function verify_and_update_state \
  --calldata "$(cat proof-calldata.txt)"
```

## Step 5: Verify State Updated

```bash
sncast call \
  --url "$STARKNET_RPC_URL_SEPOLIA" \
  --contract-address "$(jq -r '.contracts.katana_tee.address' deployments/sepolia.json)" \
  --function get_latest_state
```

Expected output matches `block_number`, `state_root`, `block_hash` from attestation.

## Transaction Examples

| Step | Transaction Hash |
|------|------------------|
| Deploy AMDTEERegistry | `0x...` |
| Deploy KatanaTee | `0x...` |
| verify_and_update_state | `0x...` |

*(Fill in after actual deployment)*
```

**Step 2: Commit documentation**

```bash
git add docs/e2e-demo-sepolia.md
git commit -m "docs: add E2E demo guide for Sepolia testnet"
```

---

### Task 4: Execute Testnet Deployment (Manual Step)

This task requires actual execution with funded accounts.

**Prerequisites:**
1. Starknet Sepolia account with ETH for fees
2. SP1 Prover Network API key
3. Access to Katana TEE endpoint

**Steps:**
1. Complete Task 1 (gather prerequisites)
2. Run `tests/deployment/deploy_sepolia.sh`
3. Update `docs/e2e-demo-sepolia.md` with actual transaction hashes
4. Run full pipeline to verify

---

## Completion Criteria

The deliverables will be **100% complete** when:

1. ✅ `deployments/sepolia.json` exists with deployed contract addresses
2. ✅ `docs/e2e-demo-sepolia.md` contains actual transaction hashes
3. ✅ At least one successful `verify_and_update_state` transaction on Sepolia
4. ✅ `get_latest_state()` returns verified Katana state on Sepolia

---

## Notes

- **Garaga Verifier:** May already be deployed on Sepolia. Check Garaga docs for existing class hash.
- **SP1 Program ID:** Must match the exact vkey hash of the AMD SEV-SNP attestation program.
- **Root Certificates:** These are long-lived but should be verified against AMD KDS.
- **Mock Mode:** For testing without real attestations, use `SP1_PROVER=mock` but this won't produce valid on-chain proofs.
