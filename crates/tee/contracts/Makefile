usage help:
	@echo "Katana TEE Contracts - Available Commands"
	@echo ""
	@echo "Build:"
	@echo "  make build          - Build Cairo contracts (scarb build)"
	@echo ""
	@echo "Tests:"
	@echo "  make test           - Run all Cairo tests (snforge test --workspace)"
	@echo "  make test-fork      - Include fork-based tests (needs MAINNET/SEPOLIA RPC URL)"
	@echo ""
	@echo "Devnet & Deployment:"
	@echo "  make devnet-mainnet - Start Katana forking mainnet"
	@echo "  make deploy-devnet  - Deploy contracts to devnet via sncast"

.PHONY: build test test-fork devnet-mainnet deploy-devnet help

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

# Deploy contracts to devnet via sncast script
deploy-devnet:
	@set -a && . ./.env && set +a && \
	sncast --account "$$STARKNET_ACCOUNT" script run deployment --network devnet --package deployment --no-state-file
