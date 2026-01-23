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

This generates `contracts/amd_tee_registry/tests/test_journal_decode_from_fixtures/test_journal_decode_fixtures.cairo`.

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
│       └── test_journal_decode_from_fixtures/
│           └── test_journal_decode_fixtures.cairo  # Generated fixtures
└── katana_tee/
    └── tests/
        ├── test_contract.cairo                     # Basic contract tests
        └── test_verify_with_fixtures.cairo         # Fork integration tests
```
