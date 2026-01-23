# =============================================================================
# Katana TEE Client - Makefile
# =============================================================================

# Default RPC URL (can be overridden)
RPC_URL ?= http://185.26.9.157:5050


# =============================================================================
# CLI Commands (using the katana-tee binary)
# =============================================================================

# Build the CLI
build: 
	cargo build -p katana_tee_client --release

# Fetch attestation from RPC and print to stdout
fetch:
	@if [ -f .env ]; then set -a && . ./.env && set +a; fi && \
	cargo run -p katana_tee_client --release --bin katana-tee -- fetch --rpc $(RPC_URL)

# Fetch attestation and save to file
fetch-save:
	@if [ -f .env ]; then set -a && . ./.env && set +a; fi && \
	cargo run -p katana_tee_client --release --bin katana-tee -- fetch --rpc $(RPC_URL) --output attestation.json

# Execute SP1 program (mock mode, fast)
execute:
	@if [ -f .env ]; then set -a && . ./.env && set +a; fi && \
	cargo run -p katana_tee_client --release --bin katana-tee -- execute --rpc $(RPC_URL)

# Generate proof via SP1 Network (Groth16)
prove:
	@if [ -f .env ]; then set -a && . ./.env && set +a; fi && \
	cargo run -p katana_tee_client --release --bin katana-tee -- prove --rpc $(RPC_URL) --prover network

# Generate proof in mock mode (for testing)
prove-mock:
	@if [ -f .env ]; then set -a && . ./.env && set +a; fi && \
	cargo run -p katana_tee_client --release --bin katana-tee -- prove --rpc $(RPC_URL) --prover mock

# Show proof info
proof-info:
	cargo run -p katana_tee_client --release --bin katana-tee -- info proof_output.json

# =============================================================================
# TEE VM Management
# =============================================================================

# Start the TEE VM and Katana
tee-start:
	./katana-tee-setup.sh start

# Stop the TEE VM
tee-stop:
	./katana-tee-setup.sh stop

# Check TEE VM status
tee-status:
	./katana-tee-setup.sh status

# Test TEE attestation endpoint
tee-test:
	./katana-tee-setup.sh test

# =============================================================================
# Full Pipeline Examples
# =============================================================================

# Full pipeline: Fetch -> Execute (quick test)
pipeline-test:
	@echo "=== Fetching attestation ==="
	@$(MAKE) fetch-save
	@echo ""
	@echo "=== Executing SP1 program ==="
	@$(MAKE) execute

# Full pipeline: Fetch -> Prove (network)
pipeline-prove:
	@echo "=== Starting TEE VM ==="
	@$(MAKE) tee-start
	@echo ""
	@echo "=== Generating SP1 Groth16 Proof ==="
	@$(MAKE) prove
	@echo ""
	@echo "=== Proof Info ==="
	@$(MAKE) proof-info

# =============================================================================
# Help
# =============================================================================

help:
	@echo "Katana TEE Client - Available Commands"
	@echo ""
	@echo "CLI Commands:"
	@echo "  make build          - Build the CLI"
	@echo "  make fetch          - Fetch attestation from RPC"
	@echo "  make fetch-save     - Fetch and save to attestation.json"
	@echo "  make execute        - Execute SP1 (mock mode, fast)"
	@echo "  make prove          - Generate Groth16 proof via network"
	@echo "  make prove-mock     - Generate mock proof (testing)"
	@echo "  make proof-info     - Show proof_output.json details"
	@echo ""
	@echo "TEE VM Management:"
	@echo "  make tee-start      - Start TEE VM and Katana"
	@echo "  make tee-stop       - Stop TEE VM"
	@echo "  make tee-status     - Check VM status"
	@echo "  make tee-test       - Test attestation endpoint"
	@echo ""
	@echo "Pipelines:"
	@echo "  make pipeline-test  - Fetch + Execute (quick test)"
	@echo "  make pipeline-prove - Start VM + Prove (full)"
	@echo ""
	@echo "Variables:"
	@echo "  RPC_URL=<url>       - Override RPC endpoint"
	@echo ""
	@echo "Example:"
	@echo "  make prove RPC_URL=http://localhost:5050"

.PHONY: build fetch fetch-save execute prove prove-mock proof-info \
        tee-start tee-stop tee-status tee-test \
        pipeline-test pipeline-prove help \
        devnet-mainnet e2e-test e2e-test-live fetch-root-certs


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

