# Testing Guide

## Quick Reference

```bash
make test              # Full suite: Rust + Cairo + E2E
make test-e2e-reuse    # E2E with existing proofs (fast)
make test-fork         # Fork-based Cairo tests (needs MAINNET_RPC_URL)
make test-rust         # Rust tests only
make test-cairo        # Cairo tests only
```

## E2E Tests

The E2E test script processes blocks 0, 1, 2 to test the certificate cache flow:
- Block 0: `prefix_len=1` (live mode, caches ASK)
- Block 1-2: `prefix_len=2` (uses cached ASK)

```bash
# Generate fresh proofs (requires KATANA_RPC_URL + SP1 network access)
./tests/e2e/run_e2e_tests.sh

# Reuse existing proofs (skip SP1 network, much faster)
./tests/e2e/run_e2e_tests.sh --reuse-proofs
```

The script:
1. Starts devnet (forking mainnet for Garaga verifier)
2. Fetches AMD root certificates from KDS
3. Deploys AMDTEERegistry + KatanaTee contracts
4. For each block: fetch attestation → prove → submit → verify state

## Test Fixtures

Located in `tests/fixtures/`:

```
tests/fixtures/
├── root_certs.json       # AMD root certificate hashes (Milan, Genoa)
├── deployment.json       # Contract addresses from last E2E run
├── block_0/
│   ├── attestation.json  # TEE attestation quote
│   ├── proof.json        # SP1 Groth16 proof
│   └── calldata.txt      # Starknet calldata
├── block_1/
│   └── ...
└── block_2/
    └── ...
```

## Cairo Tests

### Generate Test Fixtures

After running E2E tests, regenerate Cairo fixtures from the proof files:

```bash
make generate-cairo-fixtures
# or
cargo run -p katana_tee_client --bin katana-tee -- generate-cairo-fixtures
```

This generates `contracts/amd_tee_registry/tests/test_journal_decode_from_fixtures/test_journal_decode_fixtures.cairo`.

### Run Cairo Tests

```bash
# All Cairo tests
snforge test --workspace

# Fork-based tests (requires MAINNET_RPC_URL in .env)
snforge test --workspace --include-ignored
```

## Test Structure

```
tests/
├── e2e/
│   └── run_e2e_tests.sh        # E2E test runner
└── fixtures/
    ├── root_certs.json         # AMD root cert hashes
    ├── deployment.json         # Contract addresses
    └── block_N/                # Per-block fixtures

contracts/
├── amd_tee_registry/
│   └── tests/
│       ├── test_contract.cairo                     # Basic contract tests
│       ├── journal_decode.cairo                    # Journal decoding tests
│       ├── test_journal_decode_from_fixtures.cairo # Fixture-based decode tests
│       ├── test_cache_diagnostic.cairo             # Cache behavior tests
│       ├── test_e2e_cache_flow.cairo               # E2E cache flow tests
│       ├── root_certs_helper.cairo                 # Root cert test utilities
│       └── test_journal_decode_from_fixtures/
│           └── test_journal_decode_fixtures.cairo  # Generated fixtures
└── katana_tee/
    └── tests/
        ├── test_contract.cairo                     # Basic contract tests
        ├── test_verify_with_fixtures.cairo         # Fork integration tests
        ├── test_verify_katana_report_data.cairo    # Report data verification
        └── report_utils.cairo                      # Test utilities
```

## Environment Setup

Required in `.env` for full test suite:

```bash
# E2E tests
KATANA_RPC_URL=http://...        # TEE Katana instance
MAINNET_RPC_URL=https://...      # For devnet fork
DEVNET_SEED=0
DEVNET_PORT=5050
DEVNET_ACCOUNT_ADDRESS=0x...
DEVNET_ACCOUNT_PRIVATE_KEY=0x...

# SP1 proving (for fresh proof generation)
NETWORK_PRIVATE_KEY=...          # SP1 network key
```
