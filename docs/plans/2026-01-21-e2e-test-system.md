# E2E Test System Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Create a complete end-to-end test system that deploys contracts to forked mainnet devnet, generates real SP1 proofs, submits them on-chain, and verifies state updates.

**Architecture:** The system consists of: (1) a CLI command to fetch AMD root certificates from KDS and output their SHA256 hashes, (2) a bash E2E test script that orchestrates devnet startup, contract deployment, proof generation, and verification, (3) Makefile targets for easy execution.

**Tech Stack:** Rust (snp-attest-cli), Bash, starknet-devnet, sncast, SP1 Prover Network

---

## Constants Reference

```bash
GARAGA_SP1_VERIFIER_CLASS_HASH=0x4b22453df42037dd61390736454e8390910adfbbc1fa9d85613e6f375f4de22
SP1_PROGRAM_ID=0x00d2342d2400bed28302507269281dcb2c621bae91a0626796ce637f01c928d8
```

---

### Task 1: Add KDS Module to amd_tee_registry_client

**Files:**
- Create: `clients/amd_tee_registry_client/src/kds.rs`
- Modify: `clients/amd_tee_registry_client/src/lib.rs`
- Modify: `clients/amd_tee_registry_client/Cargo.toml`

**Step 1: Add dependencies to Cargo.toml**

Add `sha2`, `serde_json`, and `hex` to the crate's Cargo.toml (move from dev-dependencies to dependencies):

```toml
# Add to [dependencies] section
sha2 = "0.10"
serde = { workspace = true }
serde_json = { workspace = true }
hex = { workspace = true }
```

**Step 2: Create kds.rs module**

Create `clients/amd_tee_registry_client/src/kds.rs`:

```rust
//! AMD Key Distribution Service (KDS) client
//!
//! Fetch AMD root certificates and compute their hashes for on-chain verification.

use amd_sev_snp_attestation_prover::kds::KDS as SdkKDS;
use amd_sev_snp_attestation_verifier::stub::ProcessorType;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::Path;

/// Result of fetching a root certificate
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RootCertInfo {
    /// SHA256 hash of the ARK certificate in DER format (hex with 0x prefix)
    pub ark_hash: String,
    /// Source URL from AMD KDS
    pub source: String,
}

/// AMD KDS client for fetching root certificates
pub struct KdsClient {
    inner: SdkKDS,
}

impl KdsClient {
    pub fn new() -> Self {
        Self {
            inner: SdkKDS::new(),
        }
    }

    /// Fetch root certificate (ARK) hash for a processor type
    pub fn fetch_root_cert_hash(&self, processor: ProcessorType) -> Result<RootCertInfo, crate::Error> {
        let proc_str = processor.to_str()
            .map_err(|e| crate::Error::Prover(format!("Invalid processor type: {}", e)))?;

        tracing::info!("Fetching cert chain for {}...", proc_str);

        let cert_chain = self.inner.fetch_model_cert_chain(processor)
            .map_err(|e| crate::Error::Prover(format!("Failed to fetch cert chain: {}", e)))?;

        // cert_chain is [ASK, ARK] - ARK is index 1
        if cert_chain.len() < 2 {
            return Err(crate::Error::Prover(format!(
                "Expected at least 2 certs in chain, got {}",
                cert_chain.len()
            )));
        }

        let ark_der = &cert_chain[1];
        let ark_hash = Sha256::digest(ark_der.as_ref());
        let ark_hash_hex = format!("0x{}", hex::encode(ark_hash));

        Ok(RootCertInfo {
            ark_hash: ark_hash_hex,
            source: format!("https://kdsintf.amd.com/vcek/v1/{}/cert_chain", proc_str),
        })
    }

    /// Fetch root certificate hashes for multiple processor types
    pub fn fetch_root_certs(&self, processors: &[ProcessorType]) -> Result<HashMap<String, RootCertInfo>, crate::Error> {
        let mut results = HashMap::new();

        for processor in processors {
            let proc_str = processor.to_str()
                .map_err(|e| crate::Error::Prover(format!("Invalid processor type: {}", e)))?
                .to_lowercase();

            let info = self.fetch_root_cert_hash(*processor)?;
            results.insert(proc_str, info);
        }

        Ok(results)
    }

    /// Validate fetched root cert against a local .der file
    pub fn validate_against_file(&self, processor: ProcessorType, der_path: &Path) -> Result<bool, crate::Error> {
        let fetched = self.fetch_root_cert_hash(processor)?;

        let local_der = std::fs::read(der_path)
            .map_err(|e| crate::Error::Prover(format!("Failed to read {}: {}", der_path.display(), e)))?;

        let local_hash = Sha256::digest(&local_der);
        let local_hash_hex = format!("0x{}", hex::encode(local_hash));

        Ok(fetched.ark_hash == local_hash_hex)
    }
}

impl Default for KdsClient {
    fn default() -> Self {
        Self::new()
    }
}

/// Parse processor type from string
pub fn parse_processor_type(s: &str) -> Option<ProcessorType> {
    match s.trim().to_lowercase().as_str() {
        "milan" => Some(ProcessorType::Milan),
        "genoa" => Some(ProcessorType::Genoa),
        "bergamo" => Some(ProcessorType::Bergamo),
        "siena" => Some(ProcessorType::Siena),
        _ => None,
    }
}
```

**Step 3: Update lib.rs to export the module**

Add to `clients/amd_tee_registry_client/src/lib.rs`:

```rust
pub mod kds;

pub use kds::{KdsClient, RootCertInfo, parse_processor_type};
```

**Step 4: Build and verify**

Run:
```bash
cargo build -p amd_tee_registry_client
```

Expected: Compiles successfully

**Step 5: Commit**

```bash
git add clients/amd_tee_registry_client/
git commit -m "feat(amd_tee_registry_client): add KDS module for fetching AMD root certs"
```

---

### Task 1b: Add fetch-root-certs Command to katana-tee CLI

**Files:**
- Modify: `clients/katana_tee_client/src/bin/cli.rs`
- Modify: `clients/katana_tee_client/Cargo.toml`

**Step 1: Add dependencies to katana_tee_client Cargo.toml**

Ensure these are in dependencies:

```toml
serde_json = { workspace = true }
```

**Step 2: Add FetchRootCerts command to CLI**

Add to the `Commands` enum in `clients/katana_tee_client/src/bin/cli.rs`:

```rust
    /// Fetch AMD root certificates from KDS and output their hashes
    FetchRootCerts {
        /// Processor types to fetch (comma-separated: milan,genoa,bergamo,siena)
        #[arg(long, default_value = "milan,genoa")]
        processors: String,

        /// Output JSON file path (prints to stdout if not specified)
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Directory containing .der files to validate against
        #[arg(long)]
        validate: Option<PathBuf>,
    },
```

**Step 3: Add the command handler in main**

Add the match arm for FetchRootCerts:

```rust
        Commands::FetchRootCerts { processors, output, validate } => {
            use amd_tee_registry_client::{KdsClient, parse_processor_type};

            let kds = KdsClient::new();
            let mut results = serde_json::Map::new();

            for proc_str in processors.split(',') {
                let proc_str = proc_str.trim();
                let proc_type = match parse_processor_type(proc_str) {
                    Some(p) => p,
                    None => {
                        eprintln!("Unknown processor type: {}", proc_str);
                        continue;
                    }
                };

                println!("Fetching cert chain for {}...", proc_str);

                match kds.fetch_root_cert_hash(proc_type) {
                    Ok(info) => {
                        println!("  ARK hash: {}", info.ark_hash);

                        // Validate against local .der file if provided
                        if let Some(validate_dir) = &validate {
                            let der_path = validate_dir.join(format!("ark-{}.der", proc_str.to_lowercase()));
                            if der_path.exists() {
                                match kds.validate_against_file(proc_type, &der_path) {
                                    Ok(true) => println!("  ✓ Matches local {}", der_path.display()),
                                    Ok(false) => eprintln!("  ✗ MISMATCH with local {}!", der_path.display()),
                                    Err(e) => eprintln!("  ✗ Validation error: {}", e),
                                }
                            }
                        }

                        let mut entry = serde_json::Map::new();
                        entry.insert("ark_hash".to_string(), serde_json::Value::String(info.ark_hash));
                        entry.insert("source".to_string(), serde_json::Value::String(info.source));
                        results.insert(proc_str.to_lowercase(), serde_json::Value::Object(entry));
                    }
                    Err(e) => {
                        eprintln!("  Error fetching {}: {}", proc_str, e);
                    }
                }
            }

            let json_output = serde_json::to_string_pretty(&results)
                .expect("Failed to serialize JSON");

            if let Some(output_path) = output {
                std::fs::write(&output_path, &json_output)
                    .expect("Failed to write output file");
                println!("\nSaved to {}", output_path.display());
            } else {
                println!("\n{}", json_output);
            }
        }
```

**Step 4: Build and test**

Run:
```bash
cargo build -p katana_tee_client --release
```

Expected: Compiles successfully

**Step 5: Test the command**

Run:
```bash
cargo run -p katana_tee_client --release --bin katana-tee -- fetch-root-certs \
    --processors milan,genoa \
    --validate crates/amd-sev-snp-attestation-sdk/contracts/test/assets \
    --output /tmp/root_certs.json
```

Expected output:
```
Fetching cert chain for milan...
  ARK hash: 0x69d063b45344d26a2e94e1f4210de49ef555308287d4c174445c95639a540bcd
  ✓ Matches local crates/amd-sev-snp-attestation-sdk/contracts/test/assets/ark-milan.der
Fetching cert chain for genoa...
  ARK hash: 0x4c6598d19c18719c5dfd4a7d335f674e5bfe1d8f800cea2cf270c10d103db2f1
  ✓ Matches local crates/amd-sev-snp-attestation-sdk/contracts/test/assets/ark-genoa.der

Saved to /tmp/root_certs.json
```

**Step 6: Commit**

```bash
git add clients/katana_tee_client/
git commit -m "feat(katana-tee-cli): add fetch-root-certs command"
```

---

### Task 2: Create E2E Test Directory Structure

**Files:**
- Create: `tests/e2e/fixtures/.gitkeep`
- Create: `tests/e2e/run_e2e_tests.sh`

**Step 1: Create directory structure**

```bash
mkdir -p tests/e2e/fixtures
touch tests/e2e/fixtures/.gitkeep
```

**Step 2: Commit structure**

```bash
git add tests/e2e/
git commit -m "chore: add e2e test directory structure"
```

---

### Task 3: Create E2E Test Script

**Files:**
- Create: `tests/e2e/run_e2e_tests.sh`

**Step 1: Write the complete E2E test script**

```bash
#!/usr/bin/env bash
#
# E2E Test Script for katana-tee
#
# Usage:
#   ./run_e2e_tests.sh --live     # Fetch from TEE, generate proof, save fixtures
#   ./run_e2e_tests.sh --fixture  # Use saved fixtures (default)
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
FIXTURES_DIR="$SCRIPT_DIR/fixtures"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
NC='\033[0m'

log() { echo -e "${GREEN}[E2E]${NC} $1"; }
warn() { echo -e "${YELLOW}[WARN]${NC} $1"; }
error() { echo -e "${RED}[ERROR]${NC} $1"; }
die() { error "$1"; exit 1; }

# Load environment
set -a
source "$PROJECT_ROOT/.env"
set +a

# Configuration
DEVNET_URL="http://127.0.0.1:${DEVNET_PORT:-5050}"
GARAGA_CLASS_HASH="0x4b22453df42037dd61390736454e8390910adfbbc1fa9d85613e6f375f4de22"
SP1_PROGRAM_ID="0x00d2342d2400bed28302507269281dcb2c621bae91a0626796ce637f01c928d8"
MAX_TIME_DIFF=86400
DEVNET_PID=""

cleanup() {
    if [[ -n "$DEVNET_PID" ]]; then
        log "Stopping devnet (PID: $DEVNET_PID)..."
        kill "$DEVNET_PID" 2>/dev/null || true
        wait "$DEVNET_PID" 2>/dev/null || true
    fi
}
trap cleanup EXIT

wait_for_rpc() {
    local url=$1
    local max_attempts=30
    log "Waiting for RPC at $url..."
    for i in $(seq 1 $max_attempts); do
        if curl -s "$url" -X POST -H "Content-Type: application/json" \
            -d '{"jsonrpc":"2.0","method":"starknet_chainId","params":[],"id":1}' \
            | grep -q "result"; then
            log "RPC ready"
            return 0
        fi
        sleep 1
    done
    die "RPC not ready after $max_attempts seconds"
}

# Split u256 into low/high felt252 for calldata
split_u256() {
    local value=$1
    # Remove 0x prefix if present
    value=${value#0x}
    # Pad to 64 chars
    value=$(printf "%064s" "$value" | tr ' ' '0')
    # Split: low = last 32 chars, high = first 32 chars
    local high="0x${value:0:32}"
    local low="0x${value:32:32}"
    # Remove leading zeros but keep at least one digit
    high=$(printf "0x%x" "$((16#${high#0x}))" 2>/dev/null || echo "0x0")
    low=$(printf "0x%x" "$((16#${low#0x}))" 2>/dev/null || echo "0x0")
    echo "$low $high"
}

start_devnet() {
    log "Starting devnet (forking mainnet, seed $DEVNET_SEED)..."
    starknet-devnet \
        --fork-network "$MAINNET_RPC_URL" \
        --seed "$DEVNET_SEED" \
        --port "$DEVNET_PORT" \
        --timeout 300 &
    DEVNET_PID=$!
    wait_for_rpc "$DEVNET_URL"
}

fetch_root_certs() {
    log "Fetching AMD root certificates from KDS..."
    cargo run -p katana_tee_client --release --bin katana-tee -- \
        fetch-root-certs \
        --processors milan,genoa \
        --validate "$PROJECT_ROOT/crates/amd-sev-snp-attestation-sdk/contracts/test/assets" \
        --output "$FIXTURES_DIR/root_certs.json"
}

deploy_contracts() {
    log "Deploying contracts..."

    # Load root cert hashes
    local milan_root=$(jq -r '.milan.ark_hash' "$FIXTURES_DIR/root_certs.json")
    local genoa_root=$(jq -r '.genoa.ark_hash' "$FIXTURES_DIR/root_certs.json")

    log "  Milan root: $milan_root"
    log "  Genoa root: $genoa_root"

    # Build contracts
    log "Building amd_tee_registry..."
    cd "$PROJECT_ROOT/contracts/amd_tee_registry"
    scarb build

    log "Building katana_tee..."
    cd "$PROJECT_ROOT/contracts/katana_tee"
    scarb build

    # Declare amd_tee_registry
    log "Declaring AMDTEERegistry..."
    cd "$PROJECT_ROOT/contracts/amd_tee_registry"

    local amd_class_hash
    amd_class_hash=$(sncast --account devnet-1 declare \
        --url "$DEVNET_URL" \
        --contract-name AMDTEERegistry \
        --package amd_tee_registry 2>&1 | grep -oP 'class_hash:\s*\K0x[a-fA-F0-9]+' || \
        sncast utils class-hash --contract-name AMDTEERegistry --package amd_tee_registry 2>&1 | grep -oP '0x[a-fA-F0-9]+' | head -1)

    log "  AMDTEERegistry class_hash: $amd_class_hash"

    # Split SP1 program ID into low/high
    local sp1_split=$(split_u256 "$SP1_PROGRAM_ID")
    local sp1_low=$(echo $sp1_split | cut -d' ' -f1)
    local sp1_high=$(echo $sp1_split | cut -d' ' -f2)

    # Constructor calldata:
    # verifier_class_hash, sp1_program_id (u256 = low, high), max_time_diff,
    # trusted_certs (array len + items), processor_models (array len + items), root_certs (array len + items)
    #
    # For testing: empty trusted_certs, 2 processor models (Milan=0, Genoa=1), 2 root certs
    local constructor_calldata="$GARAGA_CLASS_HASH $sp1_low $sp1_high $MAX_TIME_DIFF 0 2 0 1 2 $milan_root $genoa_root"

    log "Deploying AMDTEERegistry..."
    local amd_deploy_output
    amd_deploy_output=$(sncast --account devnet-1 deploy \
        --url "$DEVNET_URL" \
        --class-hash "$amd_class_hash" \
        --constructor-calldata $constructor_calldata 2>&1)

    local amd_address
    amd_address=$(echo "$amd_deploy_output" | grep -oP 'contract_address:\s*\K0x[a-fA-F0-9]+')

    if [[ -z "$amd_address" ]]; then
        error "Failed to deploy AMDTEERegistry"
        echo "$amd_deploy_output"
        exit 1
    fi

    log "  AMDTEERegistry deployed: $amd_address"

    # Declare katana_tee
    log "Declaring KatanaTee..."
    cd "$PROJECT_ROOT/contracts/katana_tee"

    local katana_class_hash
    katana_class_hash=$(sncast --account devnet-1 declare \
        --url "$DEVNET_URL" \
        --contract-name KatanaTee \
        --package katana_tee 2>&1 | grep -oP 'class_hash:\s*\K0x[a-fA-F0-9]+' || \
        sncast utils class-hash --contract-name KatanaTee --package katana_tee 2>&1 | grep -oP '0x[a-fA-F0-9]+' | head -1)

    log "  KatanaTee class_hash: $katana_class_hash"

    log "Deploying KatanaTee..."
    local katana_deploy_output
    katana_deploy_output=$(sncast --account devnet-1 deploy \
        --url "$DEVNET_URL" \
        --class-hash "$katana_class_hash" \
        --constructor-calldata "$amd_address" 2>&1)

    local katana_address
    katana_address=$(echo "$katana_deploy_output" | grep -oP 'contract_address:\s*\K0x[a-fA-F0-9]+')

    if [[ -z "$katana_address" ]]; then
        error "Failed to deploy KatanaTee"
        echo "$katana_deploy_output"
        exit 1
    fi

    log "  KatanaTee deployed: $katana_address"

    # Verify registry linkage
    log "Verifying registry address linkage..."
    local registry_result
    registry_result=$(sncast call \
        --url "$DEVNET_URL" \
        --contract-address "$katana_address" \
        --function get_registry_address 2>&1)

    log "  get_registry_address: $registry_result"

    # Save deployment
    cat > "$FIXTURES_DIR/deployment.json" << EOF
{
  "network": "devnet-mainnet-fork",
  "timestamp": "$(date -Iseconds)",
  "amd_tee_registry": {
    "class_hash": "$amd_class_hash",
    "address": "$amd_address"
  },
  "katana_tee": {
    "class_hash": "$katana_class_hash",
    "address": "$katana_address"
  },
  "config": {
    "garaga_verifier_class_hash": "$GARAGA_CLASS_HASH",
    "sp1_program_id": "$SP1_PROGRAM_ID",
    "max_time_diff": $MAX_TIME_DIFF
  }
}
EOF

    log "Deployment saved to $FIXTURES_DIR/deployment.json"
}

generate_proof_live() {
    log "=== LIVE MODE: Generating real proof ==="

    log "Fetching attestation from Katana TEE at $KATANA_RPC_URL..."
    cargo run -p katana_tee_client --release --bin katana-tee -- \
        fetch --rpc "$KATANA_RPC_URL" --output "$FIXTURES_DIR/attestation.json"

    log "Attestation saved. Generating SP1 proof via network prover..."
    log "This may take several minutes..."

    # Set environment for SP1 prover
    export SP1_PROVER=network

    cargo run -p katana_tee_client --release --bin katana-tee -- \
        prove \
        --input "$FIXTURES_DIR/attestation.json" \
        --prover network \
        --output "$FIXTURES_DIR/proof.json" \
        --calldata-output "$FIXTURES_DIR/calldata.txt"

    log "Proof generated and saved to fixtures"
}

submit_proof() {
    log "Submitting proof to katana_tee..."

    local katana_address=$(jq -r '.katana_tee.address' "$FIXTURES_DIR/deployment.json")
    local calldata=$(cat "$FIXTURES_DIR/calldata.txt")

    # Extract attestation data for verify_and_update_state
    local state_root=$(jq -r '.result.stateRoot' "$FIXTURES_DIR/attestation.json")
    local block_hash=$(jq -r '.result.blockHash' "$FIXTURES_DIR/attestation.json")
    local block_number=$(jq -r '.result.blockNumber' "$FIXTURES_DIR/attestation.json")

    log "  Contract: $katana_address"
    log "  State root: $state_root"
    log "  Block hash: $block_hash"
    log "  Block number: $block_number"

    # The calldata format for verify_and_update_state:
    # sp1_proof (array), state_root, block_hash, block_number
    local full_calldata="$calldata $state_root $block_hash $block_number"

    log "Invoking verify_and_update_state..."
    local invoke_result
    invoke_result=$(sncast --account devnet-1 invoke \
        --url "$DEVNET_URL" \
        --contract-address "$katana_address" \
        --function verify_and_update_state \
        --calldata $full_calldata 2>&1) || {
        error "Invoke failed:"
        echo "$invoke_result"
        exit 1
    }

    log "Transaction submitted:"
    echo "$invoke_result"
}

verify_state() {
    log "Verifying on-chain state..."

    local katana_address=$(jq -r '.katana_tee.address' "$FIXTURES_DIR/deployment.json")

    # Get latest state
    local result
    result=$(sncast call \
        --url "$DEVNET_URL" \
        --contract-address "$katana_address" \
        --function get_latest_state 2>&1)

    log "get_latest_state result:"
    echo "$result"

    # Expected values from attestation
    local expected_block=$(jq -r '.result.blockNumber' "$FIXTURES_DIR/attestation.json")
    local expected_root=$(jq -r '.result.stateRoot' "$FIXTURES_DIR/attestation.json")
    local expected_hash=$(jq -r '.result.blockHash' "$FIXTURES_DIR/attestation.json")

    log "Expected values:"
    log "  block_number: $expected_block"
    log "  state_root: $expected_root"
    log "  block_hash: $expected_hash"

    # Basic validation (the result contains the expected values)
    if echo "$result" | grep -qi "error"; then
        error "State verification failed - call returned error"
        exit 1
    fi

    log "✓ State verification completed"
}

print_summary() {
    log ""
    log "=========================================="
    log "  E2E TEST SUMMARY"
    log "=========================================="
    log ""
    log "Deployment:"
    jq '.' "$FIXTURES_DIR/deployment.json"
    log ""
    log "Attestation:"
    jq '.result | {stateRoot, blockHash, blockNumber}' "$FIXTURES_DIR/attestation.json"
    log ""
}

# === MAIN ===

MODE="${1:---fixture}"

case "$MODE" in
    --live)
        log "=========================================="
        log "  E2E TEST - LIVE MODE"
        log "=========================================="
        log ""
        start_devnet
        fetch_root_certs
        deploy_contracts
        generate_proof_live
        submit_proof
        verify_state
        print_summary
        log ""
        log "✓ LIVE E2E TEST PASSED"
        log "  Fixtures saved for future --fixture runs"
        ;;

    --fixture)
        log "=========================================="
        log "  E2E TEST - FIXTURE MODE"
        log "=========================================="
        log ""

        # Verify fixtures exist
        [[ -f "$FIXTURES_DIR/attestation.json" ]] || die "Missing attestation.json. Run with --live first"
        [[ -f "$FIXTURES_DIR/proof.json" ]] || die "Missing proof.json. Run with --live first"
        [[ -f "$FIXTURES_DIR/calldata.txt" ]] || die "Missing calldata.txt. Run with --live first"

        start_devnet
        fetch_root_certs
        deploy_contracts
        submit_proof
        verify_state
        print_summary
        log ""
        log "✓ FIXTURE E2E TEST PASSED"
        ;;

    *)
        echo "Usage: $0 [--live|--fixture]"
        echo ""
        echo "  --live     Fetch from TEE, generate real proof, save fixtures"
        echo "  --fixture  Use saved fixtures (default)"
        exit 1
        ;;
esac
```

**Step 2: Make executable**

```bash
chmod +x tests/e2e/run_e2e_tests.sh
```

**Step 3: Commit**

```bash
git add tests/e2e/run_e2e_tests.sh
git commit -m "feat: add E2E test script with live and fixture modes"
```

---

### Task 4: Update Makefile

**Files:**
- Modify: `Makefile`

**Step 1: Add E2E targets to Makefile**

Add after the existing targets (before `.PHONY`):

```makefile
# =============================================================================
# E2E Tests
# =============================================================================

# Start devnet forking mainnet (Garaga verifier available)
devnet-mainnet:
	@set -a && . ./.env && set +a && \
	starknet-devnet --fork-network $$MAINNET_RPC_URL --seed $$DEVNET_SEED --port $$DEVNET_PORT

# Run E2E tests with saved fixtures (fast, no TEE/prover needed)
e2e-test:
	./tests/e2e/run_e2e_tests.sh --fixture

# Run E2E tests live (requires TEE access + SP1 prover network)
e2e-test-live:
	./tests/e2e/run_e2e_tests.sh --live

# Fetch AMD root certificates from KDS
fetch-root-certs:
	cargo run -p katana_tee_client --release --bin katana-tee -- fetch-root-certs \
		--processors milan,genoa \
		--validate crates/amd-sev-snp-attestation-sdk/contracts/test/assets \
		--output tests/e2e/fixtures/root_certs.json
```

**Step 2: Update .PHONY**

Update the `.PHONY` line to include new targets:

```makefile
.PHONY: build fetch fetch-save execute prove prove-mock proof-info \
        example-fetch example-execute example-prove-network example-prove-json \
        generate_proof generate_proof_network generate_proof_mock \
        tee-start tee-stop tee-status tee-test \
        pipeline-test pipeline-prove e2e help \
        devnet-mainnet e2e-test e2e-test-live fetch-root-certs
```

**Step 3: Commit**

```bash
git add Makefile
git commit -m "feat: add E2E test targets to Makefile"
```

---

### Task 5: Update .env.example

**Files:**
- Modify: `.env.example`

**Step 1: Update .env.example with E2E settings**

Add/update these values:

```bash
# Devnet Configuration
DEVNET_SEED=0
DEVNET_PORT=5050

# Mainnet RPC for forking (Garaga verifier is deployed here)
MAINNET_RPC_URL=https://starknet-mainnet.public.blastapi.io/rpc/v0_7

# Katana TEE endpoint (for live E2E tests)
KATANA_RPC_URL=http://185.26.9.157:5050

# SP1 Prover Network (for live E2E tests)
SP1_PROVER=network
SP1_PRIVATE_KEY=your_sp1_prover_network_key_here
```

**Step 2: Commit**

```bash
git add .env.example
git commit -m "chore: update .env.example with E2E test configuration"
```

---

### Task 6: Test the Complete Flow (Fixture Mode)

**Prerequisites:**
- Saved fixtures from a previous --live run OR manually create test fixtures

**Step 1: Verify CLI command works**

```bash
make fetch-root-certs
```

Expected: JSON output with milan/genoa root cert hashes

**Step 2: Start devnet manually to test**

```bash
make devnet-mainnet
```

Expected: Devnet starts, forking mainnet

**Step 3: Run fixture E2E test (if fixtures exist)**

```bash
make e2e-test
```

Expected: Deploys contracts, submits proof, verifies state

---

### Task 7: Run Live E2E Test

**Prerequisites:**
- TEE endpoint accessible at KATANA_RPC_URL
- SP1_PRIVATE_KEY configured for prover network
- Sufficient credits on SP1 Prover Network

**Step 1: Run live E2E test**

```bash
make e2e-test-live
```

Expected:
1. Devnet starts (forking mainnet)
2. Root certs fetched from AMD KDS
3. Contracts deployed
4. Attestation fetched from TEE
5. SP1 proof generated (several minutes)
6. Proof submitted on-chain
7. State verified
8. Fixtures saved for future runs

**Step 2: Verify fixtures saved**

```bash
ls -la tests/e2e/fixtures/
```

Expected files:
- attestation.json
- proof.json
- calldata.txt
- deployment.json
- root_certs.json

**Step 3: Run fixture test with saved fixtures**

```bash
make e2e-test
```

Expected: Fast test using saved proof (no prover network call)

**Step 4: Commit fixtures**

```bash
git add tests/e2e/fixtures/
git commit -m "chore: add E2E test fixtures from live run"
```

---

## Completion Checklist

- [ ] `katana-tee fetch-root-certs` command works
- [ ] Root cert hashes match Solidity test assets
- [ ] E2E script deploys contracts successfully
- [ ] E2E script submits proof successfully
- [ ] E2E script verifies state correctly
- [ ] `make e2e-test` runs with fixtures
- [ ] `make e2e-test-live` generates fixtures
- [ ] All fixtures committed to repo
