#!/bin/bash

# This script is meant to be run from the root directory of the repository.

set -e
set -o xtrace

# Configuration with defaults that can be overridden thru environment variables.

DOJO_PATH=${DOJO_PATH:-"/tmp/dojo"}
ACCOUNT_ADDRESS=${ACCOUNT_ADDRESS:-"0x1f401c745d3dba9b9da11921d1fb006c96f571e9039a0ece3f3b0dc14f04c3d"}
PRIVATE_KEY=${PRIVATE_KEY:-"0x7230b49615d175307d580c33d6fda61fc7b9aec91df0f5c1a5ebe3b8cbfee02"}
DOJO_REPO=${DOJO_REPO:-"https://github.com/dojoengine/dojo.git"}
DOJO_EXAMPLE=${DOJO_EXAMPLE:-"examples/spawn-and-move"}
KATANA_DB_PATH=${KATANA_DB_PATH:-"/tmp/katana_db"}
KATANA_CHAIN_CONFIG_DIR=${KATANA_CHAIN_CONFIG_DIR:-"$(pwd)/tests/fixtures/test-chain"}
OUTPUT_DIR=${OUTPUT_DIR:-"$(pwd)/tests/fixtures"}

KATANA_PID=""

cleanup() {
  rm -rf "$DOJO_PATH"
}

# Set trap to call cleanup function on script exit
trap cleanup EXIT

echo "Creating database directory at $KATANA_DB_PATH"
mkdir -p $KATANA_DB_PATH

echo "Starting katana with database at $KATANA_DB_PATH"
katana --db-dir "$KATANA_DB_PATH" --chain "$KATANA_CHAIN_CONFIG_DIR" &
KATANA_PID=$!
sleep 5

# Check if katana is still running after the sleep
if ! kill -0 $KATANA_PID 2>/dev/null; then
  echo "Error: Katana process failed to start or terminated unexpectedly"
  exit 1
fi

# Clone Dojo repository if not already present
if [ ! -d "$DOJO_PATH" ]; then
  echo "Cloning Dojo repository to $DOJO_PATH"
  git clone "$DOJO_REPO" "$DOJO_PATH" --depth 1
fi

# Build and migrate Dojo example project
echo "Building and migrating Dojo example project at $DOJO_PATH/$DOJO_EXAMPLE"
cd $DOJO_PATH/$DOJO_EXAMPLE
sozo build
sozo migrate --account-address $ACCOUNT_ADDRESS --private-key $PRIVATE_KEY

echo "Stopping katana"
kill $KATANA_PID || true
while kill -0 $KATANA_PID 2>/dev/null; do
    echo "Waiting for katana to stop..."
    sleep 2
done

ARCHIVE_NAME="katana_db.tar.gz"
ARCHIVE_PATH="$OUTPUT_DIR/$ARCHIVE_NAME"

mkdir -p $OUTPUT_DIR
tar -czvf $ARCHIVE_PATH -C $(dirname $KATANA_DB_PATH) $(basename $KATANA_DB_PATH)
echo "Database generation complete. Archive created at $ARCHIVE_PATH"
