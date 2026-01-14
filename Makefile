# Local CPU proving (slower, no network needed)
generate_proof:
	RUSTFLAGS="-C target-cpu=native" SP1_PROVER=cpu RUST_LOG=info cargo run --example generate_proof -p katana_tee_client --release

# Network proving using SP1 Prover Network (faster, requires NETWORK_PRIVATE_KEY in .env)
# Generates Groth16 proof and saves to proof_output.json
generate_proof_network:
	@if [ -f .env ]; then set -a && . ./.env && set +a; fi && \
	SP1_PROVER=network RUST_LOG=info cargo run --example generate_proof -p katana_tee_client --release

# Mock proving for testing (fast, no real proof generated)
generate_proof_mock:
	SP1_PROVER=mock RUST_LOG=info cargo run --example generate_proof -p katana_tee_client --release