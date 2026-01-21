# E2E Multi-Block Test Fixtures Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Extend E2E tests to generate fixtures for 3 Katana blocks, add Rust CLI for Cairo fixture generation, and create comprehensive Cairo tests for journal decoding and proof verification.

**Architecture:** Multi-phase approach: (1) Modify E2E bash script to loop through 3 blocks on remote Katana TEE, saving artifacts per block; (2) Add Rust CLI subcommand to parse proof.json and generate Cairo test fixtures; (3) Add Cairo unit tests for journal_decode using generated fixtures; (4) Add Cairo contract integration tests using snforge fork testing with filesystem calldata loading.

**Tech Stack:** Bash, Rust (clap CLI), Cairo, Starknet Foundry (snforge), Garaga SP1 Verifier

---

## Task 1: Create Nested Fixture Directory Structure

**Files:**
- Create: `tests/fixtures/block_0/.gitkeep`
- Create: `tests/fixtures/block_1/.gitkeep`
- Create: `tests/fixtures/block_2/.gitkeep`

**Step 1: Create directories**

```bash
mkdir -p tests/fixtures/block_0 tests/fixtures/block_1 tests/fixtures/block_2
touch tests/fixtures/block_0/.gitkeep tests/fixtures/block_1/.gitkeep tests/fixtures/block_2/.gitkeep
```

**Step 2: Verify structure**

Run: `ls -la tests/fixtures/`
Expected: Shows block_0, block_1, block_2 directories

**Step 3: Commit**

```bash
git add tests/fixtures/block_0 tests/fixtures/block_1 tests/fixtures/block_2
git commit -m "chore: add nested fixture directories for multi-block E2E tests"
```

---

## Task 2: Refactor E2E Script for Multi-Block Support

**Files:**
- Modify: `tests/e2e/run_e2e_tests.sh`

**Step 1: Add block advancement function**

Add this function after the `generate_proof_live` function (around line 306):

```bash
# Advance Katana by mining an empty block
advance_katana_block() {
    log "Advancing Katana to next block..."
    local result
    result=$(curl -s "$KATANA_RPC_URL" -X POST -H "Content-Type: application/json" \
        -d '{"jsonrpc":"2.0","method":"dev_generateBlock","params":[],"id":1}')

    if echo "$result" | grep -q "error"; then
        warn "Failed to advance block: $result"
        return 1
    fi
    log "Block advanced successfully"
}
```

**Step 2: Add multi-block proof generation function**

Add this function after `advance_katana_block`:

```bash
# Generate proofs for multiple blocks (0, 1, 2)
generate_multi_block_proofs() {
    log "=== MULTI-BLOCK MODE: Generating proofs for blocks 0, 1, 2 ==="

    local katana_address=$(jq -r '.katana_tee.address' "$FIXTURES_DIR/deployment.json")

    for block_num in 0 1 2; do
        local block_dir="$PROJECT_ROOT/tests/fixtures/block_${block_num}"
        mkdir -p "$block_dir"

        log "--- Block $block_num ---"

        # Fetch attestation
        log "Fetching attestation for block $block_num..."
        cargo run -p katana_tee_client --release --bin katana-tee -- \
            fetch --rpc "$KATANA_RPC_URL" --output "$block_dir/attestation.json"

        # Generate proof
        log "Generating SP1 proof for block $block_num (this may take 1-2 minutes)..."
        cargo run -p katana_tee_client --release --bin katana-tee -- \
            pipeline \
            --json "$block_dir/attestation.json" \
            --starknet-rpc "$DEVNET_URL" \
            --katana-tee "$katana_address" \
            --prover network \
            --proof-output "$block_dir/proof.json" \
            --calldata-output "$block_dir/calldata.txt" \
            --skip-cache \
            --dry-run

        log "Block $block_num artifacts saved to $block_dir"

        # Advance to next block (except after last iteration)
        if [[ $block_num -lt 2 ]]; then
            advance_katana_block
            sleep 2  # Brief pause for block propagation
        fi
    done

    log "=== Multi-block proof generation complete ==="
}
```

**Step 3: Add --multi-block mode to main case statement**

Replace the main case statement (around line 414-460) with:

```bash
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
        log "LIVE E2E TEST PASSED"
        log "  Fixtures saved for future --fixture runs"
        ;;

    --multi-block)
        log "=========================================="
        log "  E2E TEST - MULTI-BLOCK MODE"
        log "=========================================="
        log ""
        start_devnet
        fetch_root_certs
        deploy_contracts
        generate_multi_block_proofs
        log ""
        log "MULTI-BLOCK FIXTURE GENERATION COMPLETE"
        log "  Fixtures saved to tests/fixtures/block_N/"
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
        log "FIXTURE E2E TEST PASSED"
        ;;

    *)
        echo "Usage: $0 [--live|--fixture|--multi-block]"
        echo ""
        echo "  --live         Fetch from TEE, generate real proof, save fixtures"
        echo "  --fixture      Use saved fixtures (default)"
        echo "  --multi-block  Generate fixtures for blocks 0, 1, 2"
        exit 1
        ;;
esac
```

**Step 4: Verify script syntax**

Run: `bash -n tests/e2e/run_e2e_tests.sh`
Expected: No output (no syntax errors)

**Step 5: Commit**

```bash
git add tests/e2e/run_e2e_tests.sh
git commit -m "feat(e2e): add multi-block fixture generation mode

Adds --multi-block flag to generate attestations, SP1 proofs, and
calldata for blocks 0, 1, 2 from remote Katana TEE RPC."
```

---

## Task 3: Add PartialEq Derive to VerifierJournal

**Files:**
- Modify: `contracts/amd_tee_registry/src/tee_types.cairo:87-103`

**Step 1: Read current struct definition**

The `VerifierJournal` struct at line 87-103 needs `PartialEq` derive for test assertions.

**Step 2: Add PartialEq derive**

Change from:
```cairo
/// Journal output from the verifier
#[derive(Drop, Debug)]
pub struct VerifierJournal {
```

To:
```cairo
/// Journal output from the verifier
#[derive(Drop, Debug, PartialEq)]
pub struct VerifierJournal {
```

**Step 3: Also add PartialEq to VerificationResult if missing**

The enum at line 52-62 already has `PartialEq`. Verify this.

**Step 4: Build to verify**

Run: `cd contracts/amd_tee_registry && scarb build`
Expected: Build succeeds

**Step 5: Run existing tests**

Run: `cd contracts/amd_tee_registry && snforge test`
Expected: All tests pass

**Step 6: Commit**

```bash
git add contracts/amd_tee_registry/src/tee_types.cairo
git commit -m "feat(cairo): add PartialEq derive to VerifierJournal for test assertions"
```

---

## Task 4: Create Rust Module for Cairo Fixture Generation

**Files:**
- Create: `clients/amd_tee_registry_client/src/cairo_fixtures.rs`
- Modify: `clients/amd_tee_registry_client/src/lib.rs`

**Step 1: Create cairo_fixtures module**

Create `clients/amd_tee_registry_client/src/cairo_fixtures.rs`:

```rust
//! Cairo Test Fixture Generator
//!
//! Generate Cairo test fixtures from SP1 proof artifacts.

use crate::prover::OnchainProof;
use crate::Error;
use amd_sev_snp_attestation_sdk::guest::attestation::{
    parse_attestation_report, AttestationReport,
};
use std::path::Path;

/// Represents a decoded VerifierJournal for Cairo fixture generation
#[derive(Debug)]
pub struct DecodedJournal {
    pub result: u8,
    pub timestamp: u64,
    pub processor_model: u8,
    pub raw_report: Vec<u32>,
    pub certs: Vec<[u8; 32]>,
    pub cert_serials: Vec<[u8; 20]>,
    pub trusted_certs_prefix_len: u8,
}

/// Parse the journal from a proof file and decode it into a VerifierJournal structure.
///
/// The journal is ABI-encoded as a Solidity struct with dynamic fields.
pub fn decode_journal_from_proof(proof: &OnchainProof) -> Result<DecodedJournal, Error> {
    let journal_bytes = &proof.raw_proof.journal;

    if journal_bytes.len() < 256 {
        return Err(Error::Calldata(format!(
            "Journal too short: {} bytes",
            journal_bytes.len()
        )));
    }

    // Skip the first 32 bytes (ABI offset pointer 0x20)
    let data = &journal_bytes[32..];

    // Parse fixed fields (first 7 words = 224 bytes)
    let result = data[31]; // Last byte of word 0
    let timestamp = u64::from_be_bytes(data[56..64].try_into().unwrap()); // Word 1
    let processor_model = data[95]; // Last byte of word 2
    let raw_report_offset = u64::from_be_bytes(data[120..128].try_into().unwrap()) as usize; // Word 3
    let certs_offset = u64::from_be_bytes(data[152..160].try_into().unwrap()) as usize; // Word 4
    let cert_serials_offset = u64::from_be_bytes(data[184..192].try_into().unwrap()) as usize; // Word 5
    let trusted_certs_prefix_len = data[223]; // Last byte of word 6

    // Parse raw_report (dynamic bytes)
    let raw_report_len_bytes =
        u64::from_be_bytes(data[raw_report_offset + 24..raw_report_offset + 32].try_into().unwrap())
            as usize;
    let raw_report_start = raw_report_offset + 32;
    let raw_report_bytes = &data[raw_report_start..raw_report_start + raw_report_len_bytes];

    // Convert to u32 array (little-endian as stored in attestation report)
    let mut raw_report = Vec::with_capacity(raw_report_len_bytes / 4);
    for chunk in raw_report_bytes.chunks(4) {
        raw_report.push(u32::from_le_bytes(chunk.try_into().unwrap()));
    }

    // Parse certs array (array of bytes32)
    let certs_len =
        u64::from_be_bytes(data[certs_offset + 24..certs_offset + 32].try_into().unwrap()) as usize;
    let mut certs = Vec::with_capacity(certs_len);
    for i in 0..certs_len {
        let cert_start = certs_offset + 32 + i * 32;
        let mut cert = [0u8; 32];
        cert.copy_from_slice(&data[cert_start..cert_start + 32]);
        certs.push(cert);
    }

    // Parse cert_serials array (array of uint160, but stored as bytes32)
    let cert_serials_len = u64::from_be_bytes(
        data[cert_serials_offset + 24..cert_serials_offset + 32]
            .try_into()
            .unwrap(),
    ) as usize;
    let mut cert_serials = Vec::with_capacity(cert_serials_len);
    for i in 0..cert_serials_len {
        let serial_start = cert_serials_offset + 32 + i * 32;
        // uint160 is in the last 20 bytes of the 32-byte word
        let mut serial = [0u8; 20];
        serial.copy_from_slice(&data[serial_start + 12..serial_start + 32]);
        cert_serials.push(serial);
    }

    Ok(DecodedJournal {
        result,
        timestamp,
        processor_model,
        raw_report,
        certs,
        cert_serials,
        trusted_certs_prefix_len,
    })
}

/// Generate Cairo test fixture code for a single block.
fn generate_block_fixture(block_num: usize, proof: &OnchainProof) -> Result<String, Error> {
    let journal = decode_journal_from_proof(proof)?;
    let journal_bytes = &proof.raw_proof.journal;

    // Convert journal bytes to u256 array (32 bytes per u256)
    let mut u256_inputs = Vec::new();
    for chunk in journal_bytes.chunks(32) {
        let mut padded = [0u8; 32];
        padded[32 - chunk.len()..].copy_from_slice(chunk);
        u256_inputs.push(padded);
    }

    let mut output = String::new();

    // Generate inputs function
    output.push_str(&format!(
        "pub fn get_block_{}_inputs() -> Array<u256> {{\n",
        block_num
    ));
    output.push_str("    array![\n");
    for (i, bytes) in u256_inputs.iter().enumerate() {
        let high = u128::from_be_bytes(bytes[0..16].try_into().unwrap());
        let low = u128::from_be_bytes(bytes[16..32].try_into().unwrap());
        output.push_str(&format!(
            "        u256 {{ low: 0x{:032x}, high: 0x{:032x} }},",
            low, high
        ));
        if i < u256_inputs.len() - 1 {
            output.push('\n');
        } else {
            output.push_str("\n");
        }
    }
    output.push_str("    ]\n");
    output.push_str("}\n\n");

    // Generate expected function
    output.push_str(&format!(
        "pub fn get_block_{}_expected() -> VerifierJournal {{\n",
        block_num
    ));

    // Result enum
    let result_variant = match journal.result {
        0 => "VerificationResult::Success",
        1 => "VerificationResult::RootCertNotTrusted",
        2 => "VerificationResult::IntermediateCertsNotTrusted",
        3 => "VerificationResult::InvalidTimestamp",
        _ => return Err(Error::Calldata(format!("Unknown result: {}", journal.result))),
    };

    output.push_str(&format!("    let result = {};\n", result_variant));
    output.push_str(&format!("    let timestamp: u64 = {};\n", journal.timestamp));
    output.push_str(&format!(
        "    let processor_model: u8 = {};\n",
        journal.processor_model
    ));
    output.push_str(&format!(
        "    let trusted_certs_prefix_len: u8 = {};\n\n",
        journal.trusted_certs_prefix_len
    ));

    // Raw report array
    output.push_str("    let mut raw_report: Array<u32> = array![\n");
    for (i, word) in journal.raw_report.iter().enumerate() {
        if i > 0 && i % 8 == 0 {
            output.push_str("\n");
        }
        output.push_str(&format!("        0x{:08x},", word));
    }
    output.push_str("\n    ];\n\n");

    // Certs array
    output.push_str("    let certs: Array<u256> = array![\n");
    for cert in &journal.certs {
        let high = u128::from_be_bytes(cert[0..16].try_into().unwrap());
        let low = u128::from_be_bytes(cert[16..32].try_into().unwrap());
        output.push_str(&format!(
            "        u256 {{ low: 0x{:032x}, high: 0x{:032x} }},\n",
            low, high
        ));
    }
    output.push_str("    ];\n\n");

    // Cert serials array (uint160 fits in felt252)
    output.push_str("    let cert_serials: Array<felt252> = array![\n");
    for serial in &journal.cert_serials {
        // Convert 20 bytes to hex
        let hex: String = serial.iter().map(|b| format!("{:02x}", b)).collect();
        output.push_str(&format!("        0x{},\n", hex));
    }
    output.push_str("    ];\n\n");

    output.push_str("    VerifierJournal {\n");
    output.push_str("        result,\n");
    output.push_str("        timestamp,\n");
    output.push_str("        processor_model,\n");
    output.push_str("        raw_report: raw_report.span(),\n");
    output.push_str("        certs,\n");
    output.push_str("        cert_serials,\n");
    output.push_str("        trusted_certs_prefix_len,\n");
    output.push_str("    }\n");
    output.push_str("}\n");

    Ok(output)
}

/// Generate complete Cairo test fixtures file from proof files.
pub fn generate_cairo_fixtures(
    fixture_dir: &Path,
    output_path: &Path,
) -> Result<(), Error> {
    let mut output = String::new();

    // File header
    output.push_str("// Auto-generated by katana-tee generate-cairo-fixtures\n");
    output.push_str("// DO NOT EDIT MANUALLY\n\n");
    output.push_str("use amd_tee_registry::tee_types::{VerifierJournal, VerificationResult};\n\n");

    // Generate fixtures for each block
    for block_num in 0..3 {
        let proof_path = fixture_dir.join(format!("block_{}/proof.json", block_num));

        if !proof_path.exists() {
            return Err(Error::Calldata(format!(
                "Proof file not found: {}",
                proof_path.display()
            )));
        }

        let proof_data = std::fs::read(&proof_path)
            .map_err(|e| Error::Calldata(format!("Failed to read {}: {}", proof_path.display(), e)))?;

        let proof = OnchainProof::decode_json(&proof_data)
            .map_err(|e| Error::Calldata(format!("Failed to parse proof: {}", e)))?;

        let fixture_code = generate_block_fixture(block_num, &proof)?;
        output.push_str(&fixture_code);
        output.push('\n');
    }

    // Write output file
    std::fs::write(output_path, &output)
        .map_err(|e| Error::Calldata(format!("Failed to write output: {}", e)))?;

    Ok(())
}
```

**Step 2: Add module to lib.rs**

Add to `clients/amd_tee_registry_client/src/lib.rs` after line 43 (`pub mod starknet;`):

```rust
pub mod cairo_fixtures;
```

And add to exports after line 51:

```rust
pub use cairo_fixtures::generate_cairo_fixtures;
```

**Step 3: Verify compilation**

Run: `cargo build -p amd_tee_registry_client`
Expected: Build succeeds

**Step 4: Commit**

```bash
git add clients/amd_tee_registry_client/src/cairo_fixtures.rs clients/amd_tee_registry_client/src/lib.rs
git commit -m "feat(rust): add Cairo fixture generator module

Parses SP1 proof journals and generates Cairo test fixtures with
input arrays and expected VerifierJournal structs."
```

---

## Task 5: Add CLI Subcommand for Cairo Fixture Generation

**Files:**
- Modify: `clients/katana_tee_client/src/bin/cli.rs`

**Step 1: Add GenerateCairoFixtures command**

Add to the `Commands` enum (after `FetchRootCerts`, around line 186):

```rust
    /// Generate Cairo test fixtures from proof files
    GenerateCairoFixtures {
        /// Directory containing block_N subdirectories with proof.json files
        #[arg(long, default_value = "tests/fixtures")]
        fixture_dir: PathBuf,

        /// Output Cairo file path
        #[arg(short, long, default_value = "contracts/amd_tee_registry/tests/test_journal_decode_fixtures.cairo")]
        output: PathBuf,
    },
```

**Step 2: Add command handler in main match**

Add to the main match statement (after `Commands::FetchRootCerts`, around line 267):

```rust
        Commands::GenerateCairoFixtures { fixture_dir, output } => {
            cmd_generate_cairo_fixtures(&fixture_dir, &output)
        }
```

**Step 3: Add command function**

Add after `cmd_fetch_root_certs` function (around line 657):

```rust
fn cmd_generate_cairo_fixtures(fixture_dir: &PathBuf, output: &PathBuf) -> anyhow::Result<()> {
    use amd_tee_registry_client::generate_cairo_fixtures;

    println!("Generating Cairo test fixtures...");
    println!("  Fixture dir: {}", fixture_dir.display());
    println!("  Output: {}", output.display());

    generate_cairo_fixtures(fixture_dir, output)?;

    println!("Cairo fixtures generated successfully!");
    Ok(())
}
```

**Step 4: Verify compilation**

Run: `cargo build -p katana_tee_client`
Expected: Build succeeds

**Step 5: Test help output**

Run: `cargo run -p katana_tee_client --bin katana-tee -- generate-cairo-fixtures --help`
Expected: Shows help for the new subcommand

**Step 6: Commit**

```bash
git add clients/katana_tee_client/src/bin/cli.rs
git commit -m "feat(cli): add generate-cairo-fixtures subcommand

Generates Cairo test fixtures from multi-block proof files."
```

---

## Task 6: Create Cairo Unit Tests for journal_decode

**Files:**
- Create: `contracts/amd_tee_registry/tests/test_journal_decode_from_fixtures.cairo`
- Modify: `contracts/amd_tee_registry/src/lib.cairo` (if needed for test module)

**Step 1: Create test file**

Create `contracts/amd_tee_registry/tests/test_journal_decode_from_fixtures.cairo`:

```cairo
use amd_tee_registry::journal_decode::decode_verifier_journal;

// Import generated fixtures (will be generated by Rust CLI)
// For now, create a placeholder that can be updated after fixtures are generated
mod test_journal_decode_fixtures;

use test_journal_decode_fixtures::{
    get_block_0_inputs, get_block_0_expected,
    get_block_1_inputs, get_block_1_expected,
    get_block_2_inputs, get_block_2_expected,
};

/// Helper to compare VerifierJournal structs field by field for better error messages
fn assert_journal_eq(
    actual: amd_tee_registry::tee_types::VerifierJournal,
    expected: amd_tee_registry::tee_types::VerifierJournal,
    block_name: felt252,
) {
    assert(actual.result == expected.result, 'result mismatch');
    assert(actual.timestamp == expected.timestamp, 'timestamp mismatch');
    assert(actual.processor_model == expected.processor_model, 'processor_model mismatch');
    assert(
        actual.trusted_certs_prefix_len == expected.trusted_certs_prefix_len,
        'trusted_prefix mismatch',
    );
    assert(actual.raw_report.len() == expected.raw_report.len(), 'raw_report len mismatch');
    assert(actual.certs.len() == expected.certs.len(), 'certs len mismatch');
    assert(actual.cert_serials.len() == expected.cert_serials.len(), 'serials len mismatch');
}

#[test]
fn test_decode_block_0() {
    let inputs = get_block_0_inputs();
    let expected = get_block_0_expected();
    let result = decode_verifier_journal(inputs.span());
    assert_journal_eq(result, expected, 'block_0');
}

#[test]
fn test_decode_block_1() {
    let inputs = get_block_1_inputs();
    let expected = get_block_1_expected();
    let result = decode_verifier_journal(inputs.span());
    assert_journal_eq(result, expected, 'block_1');
}

#[test]
fn test_decode_block_2() {
    let inputs = get_block_2_inputs();
    let expected = get_block_2_expected();
    let result = decode_verifier_journal(inputs.span());
    assert_journal_eq(result, expected, 'block_2');
}
```

**Step 2: Create placeholder fixtures file**

Create `contracts/amd_tee_registry/tests/test_journal_decode_fixtures.cairo`:

```cairo
// Placeholder - will be generated by: katana-tee generate-cairo-fixtures
// Run: cargo run -p katana_tee_client --bin katana-tee -- generate-cairo-fixtures

use amd_tee_registry::tee_types::{VerifierJournal, VerificationResult};

// Temporary stub implementations until real fixtures are generated
pub fn get_block_0_inputs() -> Array<u256> {
    // TODO: Replace with generated fixture
    array![]
}

pub fn get_block_0_expected() -> VerifierJournal {
    // TODO: Replace with generated fixture
    VerifierJournal {
        result: VerificationResult::Success,
        timestamp: 0,
        processor_model: 0,
        raw_report: array![].span(),
        certs: array![],
        cert_serials: array![],
        trusted_certs_prefix_len: 0,
    }
}

pub fn get_block_1_inputs() -> Array<u256> {
    array![]
}

pub fn get_block_1_expected() -> VerifierJournal {
    VerifierJournal {
        result: VerificationResult::Success,
        timestamp: 0,
        processor_model: 0,
        raw_report: array![].span(),
        certs: array![],
        cert_serials: array![],
        trusted_certs_prefix_len: 0,
    }
}

pub fn get_block_2_inputs() -> Array<u256> {
    array![]
}

pub fn get_block_2_expected() -> VerifierJournal {
    VerifierJournal {
        result: VerificationResult::Success,
        timestamp: 0,
        processor_model: 0,
        raw_report: array![].span(),
        certs: array![],
        cert_serials: array![],
        trusted_certs_prefix_len: 0,
    }
}
```

**Step 3: Verify Cairo build**

Run: `cd contracts/amd_tee_registry && scarb build`
Expected: Build succeeds

**Step 4: Commit**

```bash
git add contracts/amd_tee_registry/tests/test_journal_decode_from_fixtures.cairo contracts/amd_tee_registry/tests/test_journal_decode_fixtures.cairo
git commit -m "feat(cairo): add journal_decode unit tests with fixture placeholders

Tests will pass once fixtures are generated from real proof files."
```

---

## Task 7: Create Cairo Contract Integration Tests

**Files:**
- Create: `contracts/katana_tee/tests/test_verify_with_fixtures.cairo`
- Modify: `contracts/katana_tee/Scarb.toml`

**Step 1: Add fork configuration to Scarb.toml**

Add to `contracts/katana_tee/Scarb.toml` at the end:

```toml
[[tool.snforge.fork]]
name = "MAINNET"
url = "${STARKNET_RPC_URL_MAINNET}"
block_id.tag = "latest"

[[tool.snforge.fork]]
name = "SEPOLIA"
url = "${STARKNET_RPC_URL_SEPOLIA}"
block_id.tag = "latest"
```

**Step 2: Create integration test file**

Create `contracts/katana_tee/tests/test_verify_with_fixtures.cairo`:

```cairo
//! Integration tests for KatanaTee proof verification using fixture files.
//!
//! These tests use fork testing against Starknet mainnet/sepolia to access
//! the Garaga SP1 verifier contract.

use snforge_std::fs::{FileTrait, read_txt};
use snforge_std::{ContractClassTrait, DeclareResultTrait, declare};
use starknet::ContractAddress;
use katana_tee::{IKatanaTeeDispatcher, IKatanaTeeDispatcherTrait};
use amd_tee_registry::tee_registry::{IAMDTeeRegistryDispatcher, IAMDTeeRegistryDispatcherTrait};

/// Garaga SP1 Groth16 Verifier class hash (deployed on mainnet and sepolia)
const GARAGA_CLASS_HASH: felt252 = 0x4b22453df42037dd61390736454e8390910adfbbc1fa9d85613e6f375f4de22;

/// SP1 program ID for the AMD attestation verifier
const SP1_PROGRAM_ID_LOW: felt252 = 0x2c621bae91a0626796ce637f01c928d8;
const SP1_PROGRAM_ID_HIGH: felt252 = 0x00d2342d2400bed28302507269281dcb;

/// Max time difference for attestation validation (1 day)
const MAX_TIME_DIFF: u64 = 86400;

/// Deploy the AMDTEERegistry contract for testing
fn deploy_amd_registry() -> ContractAddress {
    let contract = declare("AMDTEERegistry").unwrap().contract_class();

    // Constructor: verifier_class_hash, sp1_program_id (u256), max_time_diff,
    //              trusted_certs (array), processor_models (array), root_certs (array)
    let mut calldata: Array<felt252> = array![
        GARAGA_CLASS_HASH,
        SP1_PROGRAM_ID_LOW,
        SP1_PROGRAM_ID_HIGH,
        MAX_TIME_DIFF.into(),
        0,  // trusted_certs array length = 0
        2,  // processor_models array length = 2
        0,  // Milan = 0
        1,  // Genoa = 1
        0,  // root_certs array length = 0 (would need real hashes for full verification)
    ];

    let (contract_address, _) = contract.deploy(@calldata).unwrap();
    contract_address
}

/// Deploy the KatanaTee contract for testing
fn deploy_katana_tee(registry_address: ContractAddress) -> ContractAddress {
    let contract = declare("KatanaTee").unwrap().contract_class();

    let mut calldata: Array<felt252> = array![];
    calldata.append(registry_address.into());

    let (contract_address, _) = contract.deploy(@calldata).unwrap();
    contract_address
}

/// Load calldata from a fixture file
fn load_calldata_from_fixture(path: ByteArray) -> Array<felt252> {
    let file = FileTrait::new(path);
    read_txt(@file)
}

/// Test verification of block 0 proof
#[test]
#[fork("MAINNET")]
fn test_verify_block_0() {
    // Deploy contracts
    let registry_address = deploy_amd_registry();
    let katana_address = deploy_katana_tee(registry_address);
    let dispatcher = IKatanaTeeDispatcher { contract_address: katana_address };

    // Load calldata from fixture
    let calldata = load_calldata_from_fixture("../../tests/fixtures/block_0/calldata.txt");

    // Verify proof returns public inputs
    let result = dispatcher.verify_sp1_proof(calldata);
    assert(result.is_some(), 'Block 0 verification failed');
}

/// Test verification of block 1 proof
#[test]
#[fork("MAINNET")]
fn test_verify_block_1() {
    let registry_address = deploy_amd_registry();
    let katana_address = deploy_katana_tee(registry_address);
    let dispatcher = IKatanaTeeDispatcher { contract_address: katana_address };

    let calldata = load_calldata_from_fixture("../../tests/fixtures/block_1/calldata.txt");

    let result = dispatcher.verify_sp1_proof(calldata);
    assert(result.is_some(), 'Block 1 verification failed');
}

/// Test verification of block 2 proof
#[test]
#[fork("MAINNET")]
fn test_verify_block_2() {
    let registry_address = deploy_amd_registry();
    let katana_address = deploy_katana_tee(registry_address);
    let dispatcher = IKatanaTeeDispatcher { contract_address: katana_address };

    let calldata = load_calldata_from_fixture("../../tests/fixtures/block_2/calldata.txt");

    let result = dispatcher.verify_sp1_proof(calldata);
    assert(result.is_some(), 'Block 2 verification failed');
}
```

**Step 3: Verify Cairo build**

Run: `cd contracts/katana_tee && scarb build`
Expected: Build succeeds

**Step 4: Commit**

```bash
git add contracts/katana_tee/tests/test_verify_with_fixtures.cairo contracts/katana_tee/Scarb.toml
git commit -m "feat(cairo): add contract integration tests with fork testing

Uses snforge fork testing to verify SP1 proofs against Garaga
verifier on mainnet/sepolia. Loads calldata from fixture files."
```

---

## Task 8: Update .gitignore for Generated Fixtures

**Files:**
- Modify: `.gitignore`

**Step 1: Add entry for generated fixtures file**

Add to `.gitignore`:

```
# Generated Cairo test fixtures (regenerate with: katana-tee generate-cairo-fixtures)
# Uncomment to track fixtures in git:
# !contracts/amd_tee_registry/tests/test_journal_decode_fixtures.cairo
```

**Step 2: Commit**

```bash
git add .gitignore
git commit -m "chore: document Cairo fixture generation in gitignore"
```

---

## Task 9: Add Documentation

**Files:**
- Modify: `README.md` or create `docs/testing.md`

**Step 1: Document the test workflow**

Create `docs/testing.md`:

```markdown
# Testing Guide

## E2E Tests

### Single Block Mode (default)

Generate or use a single proof fixture:

```bash
# Generate new fixtures from live TEE
./tests/e2e/run_e2e_tests.sh --live

# Run tests with existing fixtures
./tests/e2e/run_e2e_tests.sh --fixture
```

### Multi-Block Mode

Generate fixtures for blocks 0, 1, 2:

```bash
./tests/e2e/run_e2e_tests.sh --multi-block
```

This creates:
- `tests/fixtures/block_0/` - attestation.json, proof.json, calldata.txt
- `tests/fixtures/block_1/` - attestation.json, proof.json, calldata.txt
- `tests/fixtures/block_2/` - attestation.json, proof.json, calldata.txt

## Cairo Tests

### Generate Test Fixtures

After generating multi-block fixtures, generate Cairo test fixtures:

```bash
cargo run -p katana_tee_client --bin katana-tee -- generate-cairo-fixtures
```

This generates `contracts/amd_tee_registry/tests/test_journal_decode_fixtures.cairo`.

### Run Unit Tests

```bash
cd contracts/amd_tee_registry
snforge test
```

### Run Integration Tests

Integration tests use fork testing against Starknet mainnet/sepolia:

```bash
# Set RPC URL
export STARKNET_RPC_URL_MAINNET="https://your-rpc-url"

cd contracts/katana_tee
snforge test
```

## Test Structure

```
tests/
├── e2e/
│   ├── run_e2e_tests.sh    # E2E test script
│   └── fixtures/           # Single-block fixtures (legacy)
└── fixtures/
    ├── block_0/            # Multi-block fixtures
    ├── block_1/
    └── block_2/

contracts/
├── amd_tee_registry/
│   └── tests/
│       ├── journal_decode.cairo                    # Manual unit test
│       ├── test_journal_decode_from_fixtures.cairo # Fixture-based tests
│       └── test_journal_decode_fixtures.cairo      # Generated fixtures
└── katana_tee/
    └── tests/
        ├── test_contract.cairo                     # Basic contract tests
        └── test_verify_with_fixtures.cairo         # Fork integration tests
```
```

**Step 2: Commit**

```bash
git add docs/testing.md
git commit -m "docs: add testing guide for multi-block fixtures"
```

---

## Summary

| Task | Description | Files |
|------|-------------|-------|
| 1 | Create fixture directories | tests/fixtures/block_N/ |
| 2 | Modify E2E script | tests/e2e/run_e2e_tests.sh |
| 3 | Add PartialEq to VerifierJournal | contracts/amd_tee_registry/src/tee_types.cairo |
| 4 | Create Rust cairo_fixtures module | clients/amd_tee_registry_client/src/cairo_fixtures.rs |
| 5 | Add CLI subcommand | clients/katana_tee_client/src/bin/cli.rs |
| 6 | Create Cairo unit tests | contracts/amd_tee_registry/tests/*.cairo |
| 7 | Create Cairo integration tests | contracts/katana_tee/tests/test_verify_with_fixtures.cairo |
| 8 | Update .gitignore | .gitignore |
| 9 | Add documentation | docs/testing.md |

After all tasks, run the full workflow:
1. `./tests/e2e/run_e2e_tests.sh --multi-block`
2. `cargo run -p katana_tee_client --bin katana-tee -- generate-cairo-fixtures`
3. `cd contracts/amd_tee_registry && snforge test`
4. `cd contracts/katana_tee && snforge test`
