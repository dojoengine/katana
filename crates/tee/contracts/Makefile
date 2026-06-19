usage help:
	@echo "Katana TEE Contracts - Available Commands"
	@echo ""
	@echo "Build:"
	@echo "  make build             - Build Cairo contracts (scarb build)"
	@echo ""
	@echo "Tests:"
	@echo "  make test              - Run all Cairo tests (snforge test --workspace)"
	@echo "  make test-fork         - Include fork-based tests (needs MAINNET/SEPOLIA RPC URL)"
	@echo ""
	@echo "Fixtures:"
	@echo "  make fetch-root-certs  - (Re)fetch AMD root certs from KDS into amd_root_certs.json"
	@echo ""
	@echo "Devnet & Deployment:"
	@echo "  make devnet-mainnet    - Start Katana forking mainnet"
	@echo "  make deploy-devnet     - Deploy contracts to devnet via sncast"

ROOT_CERTS := amd_root_certs.json

.PHONY: build test test-fork devnet-mainnet deploy-devnet fetch-root-certs help

# =============================================================================
# Build
# =============================================================================

build:
	scarb build

# =============================================================================
# Tests
# =============================================================================

# Run all Cairo tests
test:
	@if [ -f .env ]; then set -a && . ./.env && set +a; fi && \
	snforge test --workspace

# Run fork-based Cairo tests (requires MAINNET_RPC_URL / SEPOLIA_RPC_URL)
test-fork:
	@set -a && . ./.env && set +a && \
	snforge test --workspace --include-ignored

# =============================================================================
# Devnet & Deployment
# =============================================================================

# Start Katana forking mainnet (Garaga verifier available)
devnet-mainnet:
	@set -a && . ./.env && set +a && \
	katana --fork.provider $$MAINNET_RPC_URL --dev --http.port $$DEVNET_PORT

# =============================================================================
# Fixtures
# =============================================================================

# Fetch AMD root cert hashes from KDS and write them in the flat low/high
# schema expected by `snforge_std::FileParser::<RootCerts>::parse_json`.
$(ROOT_CERTS):
	./fetch-root-certs.sh

# Force a refresh of the root cert fixture.
fetch-root-certs:
	rm -f $(ROOT_CERTS)
	$(MAKE) $(ROOT_CERTS)

# Deploy contracts to devnet via sncast script. Ensures the root cert fixture
# exists first (the deployment script reads `../amd_root_certs.json`).
deploy-devnet: $(ROOT_CERTS)
	@set -a && . ./.env && set +a && \
	sncast --account "$$STARKNET_ACCOUNT" script run deployment --network devnet --package deployment --no-state-file
