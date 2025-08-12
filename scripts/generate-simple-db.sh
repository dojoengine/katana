#!/bin/bash

# This script generates a test database using the Dojo "simple" example.
# It is meant to be run from the root directory of the repository.
#
# Usage:
#   ./scripts/generate-simple-db.sh [tag]
#
# Examples:
#   ./scripts/generate-simple-db.sh           # Uses main branch
#   ./scripts/generate-simple-db.sh v1.6.0    # Uses tag v1.6.0
#   DOJO_TAG=v1.5.0 ./scripts/generate-simple-db.sh  # Uses tag v1.5.0 via env var

set -e
set -o pipefail

# Accept tag as first argument or from environment variable
DOJO_TAG=${1:-${DOJO_TAG:-""}}

# Configuration with defaults that can be overridden through environment variables
DOJO_PATH=${DOJO_PATH:-"/tmp/dojo"}
ACCOUNT_ADDRESS=${ACCOUNT_ADDRESS:-"0x1f401c745d3dba9b9da11921d1fb006c96f571e9039a0ece3f3b0dc14f04c3d"}
PRIVATE_KEY=${PRIVATE_KEY:-"0x7230b49615d175307d580c33d6fda61fc7b9aec91df0f5c1a5ebe3b8cbfee02"}
DOJO_REPO=${DOJO_REPO:-"https://github.com/dojoengine/dojo.git"}
KATANA_DB_PATH=${KATANA_DB_PATH:-"/tmp/simple_db"}
KATANA_CHAIN_CONFIG_DIR=${KATANA_CHAIN_CONFIG_DIR:-"$(pwd)/tests/fixtures/test-chain"}
OUTPUT_DIR=${OUTPUT_DIR:-"$(pwd)/tests/fixtures/db"}
KATANA_PORT=${KATANA_PORT:-"5050"}
KATANA_LOG_LEVEL=${KATANA_LOG_LEVEL:-"info"}

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
echo "Simple Test Database Generation Script"
echo "========================================="
echo "Configuration:"
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
echo "========================================="

# Build katana
echo "Building katana..."
cargo build --release --bin katana

# Create database directory
echo "Creating database directory at $KATANA_DB_PATH"
rm -rf "$KATANA_DB_PATH"
mkdir -p "$KATANA_DB_PATH"

# Start katana with database
echo "Starting katana with database at $KATANA_DB_PATH"
./target/release/katana \
  --db-dir "$KATANA_DB_PATH" \
  --chain "$KATANA_CHAIN_CONFIG_DIR" \
  --port $KATANA_PORT \
  --log-level $KATANA_LOG_LEVEL \
  --disable-fee &
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

# Build and migrate the simple example
echo "Building and migrating Dojo simple example"
cd "$DOJO_PATH/examples/simple"

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

# Stop katana gracefully
echo "Stopping katana..."
kill $KATANA_PID || true
wait $KATANA_PID 2>/dev/null || true
KATANA_PID=""

# Archive the database
if [ ! -z "$DOJO_TAG" ]; then
  # Include tag in filename for versioned databases
  ARCHIVE_NAME="simple-${DOJO_TAG}.tar.gz"
else
  ARCHIVE_NAME="simple.tar.gz"
fi
ARCHIVE_PATH="$OUTPUT_DIR/$ARCHIVE_NAME"

echo "Creating archive at $ARCHIVE_PATH"
mkdir -p "$OUTPUT_DIR"
tar -czf "$ARCHIVE_PATH" -C "$(dirname $KATANA_DB_PATH)" "$(basename $KATANA_DB_PATH)"

# Get the actual Dojo commit hash for reference
cd "$DOJO_PATH"
DOJO_COMMIT=$(git rev-parse --short HEAD)

echo "========================================="
echo "Database generation complete!"
echo "Archive created at: $ARCHIVE_PATH"
echo "Archive size: $(du -h $ARCHIVE_PATH | cut -f1)"
if [ ! -z "$DOJO_TAG" ]; then
  echo "Dojo version: $DOJO_TAG (commit: $DOJO_COMMIT)"
else
  echo "Dojo branch: main (commit: $DOJO_COMMIT)"
fi
echo "========================================="