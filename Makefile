# =============================================================================
# Katana TEE Client - Makefile
# =============================================================================

# Default RPC URL (can be overridden)
RPC_URL ?= http://185.26.9.157:5050

# =============================================================================
# Setup
# =============================================================================

# Clone garaga repository for Starknet calldata generation
setup-garaga:
	@if [ ! -d "crates/garaga" ]; then \
		echo "Cloning garaga repository..."; \
		git clone --depth 1 https://github.com/keep-starknet-strange/garaga.git crates/garaga; \
		echo "Updating starknet-types-core to v1.0..."; \
		sed -i 's/starknet-types-core = { version = "0.1.7"/starknet-types-core = { version = "1.0"/' crates/garaga/tools/garaga_rs/Cargo.toml; \
		echo "Garaga setup complete!"; \
	else \
		echo "Garaga already cloned at crates/garaga"; \
	fi

# Full setup (all dependencies)
setup: setup-garaga
	@echo "Setup complete!"

# =============================================================================
# CLI Commands (using the katana-tee binary)
# =============================================================================

# Build the CLI
build: setup-garaga
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
# Example Commands (direct example execution)
# =============================================================================

# Fetch attestation example
example-fetch:
	@if [ -f .env ]; then set -a && . ./.env && set +a; fi && \
	cargo run --example fetch_attestation -p katana_tee_client --release -- --rpc $(RPC_URL)

# Execute proof example (mock mode)
example-execute:
	@if [ -f .env ]; then set -a && . ./.env && set +a; fi && \
	cargo run --example execute_proof -p katana_tee_client --release -- --rpc $(RPC_URL)

# Generate proof from RPC via network
example-prove-network:
	@if [ -f .env ]; then set -a && . ./.env && set +a; fi && \
	SP1_PROVER=network RUST_LOG=info cargo run --example generate_proof -p katana_tee_client --release -- --rpc $(RPC_URL)

# Generate proof from JSON file via network
example-prove-json:
	@if [ -f .env ]; then set -a && . ./.env && set +a; fi && \
	SP1_PROVER=network RUST_LOG=info cargo run --example generate_proof -p katana_tee_client --release -- --json $(JSON_FILE)

# =============================================================================
# Legacy targets (for backward compatibility)
# =============================================================================

# Local CPU proving (slower, no network needed)
generate_proof:
	RUSTFLAGS="-C target-cpu=native" SP1_PROVER=cpu RUST_LOG=info cargo run --example generate_proof -p katana_tee_client --release

# Network proving using SP1 Prover Network
generate_proof_network:
	@if [ -f .env ]; then set -a && . ./.env && set +a; fi && \
	SP1_PROVER=network RUST_LOG=info cargo run --example generate_proof -p katana_tee_client --release

# Mock proving for testing
generate_proof_mock:
	SP1_PROVER=mock RUST_LOG=info cargo run --example generate_proof -p katana_tee_client --release

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
	@echo "  JSON_FILE=<path>    - JSON file for example-prove-json"
	@echo ""
	@echo "Example:"
	@echo "  make prove RPC_URL=http://localhost:5050"

.PHONY: build fetch fetch-save execute prove prove-mock proof-info \
        example-fetch example-execute example-prove-network example-prove-json \
        generate_proof generate_proof_network generate_proof_mock \
        tee-start tee-stop tee-status tee-test \
        pipeline-test pipeline-prove help
