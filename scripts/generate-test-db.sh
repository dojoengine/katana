#!/bin/bash

# This script generates a test database using the Dojo example project (defaults to 'spawn-and-move') in provable mode.
# It is meant to be run from the root directory of the repository.
#
# Usage:
#   ./scripts/generate-test-db.sh [tag]
#
# Examples:
#   ./scripts/generate-test-db.sh           # Uses main branch of dojoengine/dojo
#   ./scripts/generate-test-db.sh v1.6.0    # Uses tag v1.6.0 of dojoengine/dojo
#   DOJO_TAG=v1.5.0 ./scripts/generate-test-db.sh  # Uses tag v1.5.0 via env var

set -e
set -o pipefail

################################################################################################
#                                      CONFIGURATIONS
################################################################################################

# Can be overridden via environment variables

ACCOUNT_ADDRESS=${ACCOUNT_ADDRESS:-"0x1f401c745d3dba9b9da11921d1fb006c96f571e9039a0ece3f3b0dc14f04c3d"}
PRIVATE_KEY=${PRIVATE_KEY:-"0x7230b49615d175307d580c33d6fda61fc7b9aec91df0f5c1a5ebe3b8cbfee02"}

# Accept tag as first argument or from environment variable
DOJO_TAG=${1:-${DOJO_TAG:-""}}
DOJO_EXAMPLE_PROJECT=${DOJO_EXAMPLE_PROJECT:-"spawn-and-move"}
DOJO_EXAMPLE_DIR="examples"
DOJO_PATH=${DOJO_PATH:-"/tmp/dojo"}
DOJO_REPO=${DOJO_REPO:-"https://github.com/dojoengine/dojo.git"}

KATANA_PORT=${KATANA_PORT:-"5050"}
KATANA_LOG_LEVEL=${KATANA_LOG_LEVEL:-"info"}
KATANA_DB_PATH=${KATANA_DB_PATH:-"/tmp/simple_db"}
KATANA_CHAIN_CONFIG_DIR=${KATANA_CHAIN_CONFIG_DIR:-"$(pwd)/tests/fixtures/test-chain"}

OUTPUT_DIR=${OUTPUT_DIR:-"$(pwd)/tests/fixtures/db"}

################################################################################################

KATANA_PID=""

# Cleanup function to ensure resources are freed
cleanup() {
  echo "Cleaning up..."

  if [ ! -z "$KATANA_PID" ] && kill -0 $KATANA_PID 2>/dev/null; then
    echo "Stopping katana (PID: $KATANA_PID)"
    kill $KATANA_PID || true
    wait $KATANA_PID 2>/dev/null || true
  fi

  rm -rf "$DOJO_PATH"
  rm -rf "$KATANA_DB_PATH"
}

# Set trap to call cleanup function on script exit
trap cleanup EXIT

echo "========================================="
echo "Test Database Generation Script"
echo "========================================="
echo "\nConfiguration:"
echo "  DOJO_EXAMPLE_PROJECT: $DOJO_EXAMPLE_PROJECT"
echo "  DOJO_PATH: $DOJO_PATH"
echo "  DOJO_REPO: $DOJO_REPO"
if [ ! -z "$DOJO_TAG" ]; then
echo "  DOJO_TAG: $DOJO_TAG"
else
echo "  DOJO_BRANCH: main (default)"
fi
echo "  KATANA_DB_PATH: $KATANA_DB_PATH"
echo "  KATANA_PORT: $KATANA_PORT"
echo "  OUTPUT_DIR: $OUTPUT_DIR"
echo "\n========================================="

# Build katana
echo "Building katana..."
cargo build --bin katana

# Create database directory
echo "Creating database directory at $KATANA_DB_PATH"
rm -rf "$KATANA_DB_PATH"
mkdir -p "$KATANA_DB_PATH"

run_katana() {
	# Start katana with database
	echo "Starting katana with database at $KATANA_DB_PATH"
	./target/debug/katana --db-dir "$KATANA_DB_PATH" --chain "$KATANA_CHAIN_CONFIG_DIR" --http.port $KATANA_PORT &
	KATANA_PID=$!

	# Wait for katana to be ready
	echo "Waiting for katana to be ready..."
	MAX_RETRIES=30
	RETRY_COUNT=0

	while ! curl -s http://127.0.0.1:$KATANA_PORT >/dev/null 2>&1; do
		if [ $RETRY_COUNT -ge $MAX_RETRIES ]; then
			echo "Error: Katana failed to start after $MAX_RETRIES attempts"
			exit 1
		fi

		# Check if katana is still running
		if ! kill -0 $KATANA_PID 2>/dev/null; then
			echo "Error: Katana process terminated unexpectedly"
			exit 1
		fi

		RETRY_COUNT=$((RETRY_COUNT + 1))
		echo "  Attempt $RETRY_COUNT/$MAX_RETRIES..."
		sleep 2
	done

	echo "Katana is ready!"
}

clone_dojo() {
	# Clone or update Dojo repository
	if [ ! -d "$DOJO_PATH" ]; then
			echo "Cloning Dojo repository to $DOJO_PATH"
		if [ ! -z "$DOJO_TAG" ]; then
			# Clone without depth to ensure we can checkout tags
			git clone "$DOJO_REPO" "$DOJO_PATH"
			cd "$DOJO_PATH"
			echo "Checking out tag: $DOJO_TAG"
			git checkout "tags/$DOJO_TAG" -b "temp-$DOJO_TAG" 2>/dev/null || git checkout "tags/$DOJO_TAG"
		else
			# Clone main branch with shallow depth
			git clone "$DOJO_REPO" "$DOJO_PATH" --depth 1 --branch main
		fi
	else
		echo "Using existing Dojo repository at $DOJO_PATH"
		cd "$DOJO_PATH"
		# Fetch latest changes
		git fetch --all --tags
		if [ ! -z "$DOJO_TAG" ]; then
			echo "Checking out tag: $DOJO_TAG"
			git checkout "tags/$DOJO_TAG" -b "temp-$DOJO_TAG" 2>/dev/null || git checkout "tags/$DOJO_TAG"
		else
			echo "Checking out main branch"
			git checkout main
			git pull origin main
		fi
	fi
}

migrate_dojo_project() {
	dojo_project_path="$DOJO_EXAMPLE_DIR/$DOJO_EXAMPLE_PROJECT"

	# Build and migrate the simple example
	echo "Building and migrating Dojo example project: $DOJO_EXAMPLE_PROJECT"
	cd "$dojo_project_path"

	# Check if sozo is installed
	if ! command -v sozo &> /dev/null; then
	  echo "Error: sozo is not installed. Please install sozo first."
	  exit 1
	fi

	echo "Running sozo build..."
	sozo build

	echo "Running sozo migrate..."
	sozo migrate \
	  --account-address $ACCOUNT_ADDRESS \
	  --private-key $PRIVATE_KEY \
	  --rpc-url http://127.0.0.1:$KATANA_PORT

	echo "Migration completed successfully!"
}

run_katana
clone_dojo
migrate_dojo_project

# Stop katana gracefully
echo "Stopping katana..."
kill $KATANA_PID || true
wait $KATANA_PID 2>/dev/null || true
KATANA_PID=""

# Archive the database
if [ ! -z "$DOJO_TAG" ]; then
  # Include tag in filename for versioned databases
  archive_name="$DOJO_EXAMPLE_PROJECT-${DOJO_TAG}.tar.gz"
else
  archive_name="$DOJO_EXAMPLE_PROJECT"
fi

archive_path="$OUTPUT_DIR/$ARCHIVE_NAME"

echo "Creating archive at $archive_path"
mkdir -p "$OUTPUT_DIR"
tar -czf "$archive_path" -C "$(dirname $KATANA_DB_PATH)" "$(basename $KATANA_DB_PATH)"

# Get the actual Dojo commit hash for reference
cd "$DOJO_PATH"
DOJO_COMMIT=$(git rev-parse --short HEAD)

echo "========================================="
echo "Database generation complete!"
echo "Archive created at: $archive_path"
echo "Archive size: $(du -h $archive_path | cut -f1)"
if [ ! -z "$DOJO_TAG" ]; then
  echo "Dojo version: $DOJO_TAG (commit: $DOJO_COMMIT)"
else
  echo "Dojo branch: main (commit: $DOJO_COMMIT)"
fi
echo "========================================="
