#!/bin/bash

# This script is meant to be run from the root directory of the repository.
# Prerequisites: katana and sozo binaries must be installed and available in PATH.
# Binary paths can be overridden using KATANA_BIN and SOZO_BIN environment variables.

# set -e
# set -o xtrace

# Parse command line arguments
DOJO_TAG=""
while [[ $# -gt 0 ]]; do
  case $1 in
    --dojo-tag)
      DOJO_TAG="$2"
      shift 2
      ;;
    -h|--help)
      echo "Usage: $0 [--dojo-tag <tag>] [--help]"
      echo "  --dojo-tag <tag>  Use specific Dojo repository tag/branch for the examples/spawn-and-move"
      echo "                    project that will be migrated to populate the Katana database."
      echo "  --help            Show this help message"
      exit 0
      ;;
    *)
      echo "Unknown option $1"
      echo "Use --help for usage information"
      exit 1
      ;;
  esac
done

# Add 'v' prefix to DOJO_TAG if it doesn't start with 'v' and is not empty
if [ -n "$DOJO_TAG" ] && [[ ! "$DOJO_TAG" =~ ^v ]]; then
  DOJO_TAG="v$DOJO_TAG"
fi

# Configuration with defaults that can be overridden thru environment variables.

KATANA_BIN=${KATANA_BIN:-"katana"}
SOZO_BIN=${SOZO_BIN:-"sozo"}
DOJO_PATH=${DOJO_PATH:-"/tmp/dojo"}
ACCOUNT_ADDRESS=${ACCOUNT_ADDRESS:-"0x1f401c745d3dba9b9da11921d1fb006c96f571e9039a0ece3f3b0dc14f04c3d"}
PRIVATE_KEY=${PRIVATE_KEY:-"0x7230b49615d175307d580c33d6fda61fc7b9aec91df0f5c1a5ebe3b8cbfee02"}
DOJO_REPO=${DOJO_REPO:-"https://github.com/dojoengine/dojo.git"}
DOJO_EXAMPLE=${DOJO_EXAMPLE:-"examples/spawn-and-move"}
KATANA_DB_PATH=${KATANA_DB_PATH:-"/tmp/katana_db"}
OUTPUT_DIR=${OUTPUT_DIR:-"$(pwd)/tests/fixtures/db"}

KATANA_PID=""
KATANA_LOG_FILE="/tmp/katana_generate-test-db.log"

# Function to print error messages with "Error" in red
print_error() {
  echo -e "\033[1;31merror:\033[0m $1"
}

cleanup() {
  # Kill katana process if it's running
  if [ -n "$KATANA_PID" ] && kill -0 $KATANA_PID 2>/dev/null; then
    echo "Stopping katana process..."
    kill $KATANA_PID || true
    # Wait for katana to stop
    local count=0
    while kill -0 $KATANA_PID 2>/dev/null && [ $count -lt 10 ]; do
      sleep 1
      count=$((count + 1))
    done
    # Force kill if still running
    if kill -0 $KATANA_PID 2>/dev/null; then
      kill -9 $KATANA_PID || true
    fi
  fi

  # Show katana logs if script failed (exit code != 0) and log file exists
  if [ $? -ne 0 ] && [ -f "$KATANA_LOG_FILE" ]; then
  	echo
    tail -n 50 "$KATANA_LOG_FILE"
  fi

  rm -rf "$DOJO_PATH"
  rm -f "$KATANA_LOG_FILE"
}

# Set trap to call cleanup function on script exit
trap cleanup EXIT

# Show binary versions
echo
echo -e "\033[1mComponent Versions:\033[0m"
echo "┌────────────┬─────────────────────────────────────────────┐"
echo "│ Component  │ Version                                     │"
echo "├────────────┼─────────────────────────────────────────────┤"
printf "│ %-7s    │ %-43s │\n" "katana" "$($KATANA_BIN -V)"
printf "│ %-7s    │ %-43s │\n" "sozo" "$($SOZO_BIN -V | head -n 1)"
echo "└────────────┴─────────────────────────────────────────────┘"

echo
echo -e "\033[1mSetting up Katana:\033[0m"
echo "  * Creating database directory at \`$KATANA_DB_PATH\`"
mkdir -p $KATANA_DB_PATH
echo "  * Starting katana: \`$KATANA_BIN --db-dir \"$KATANA_DB_PATH\"\`"
$KATANA_BIN --db-dir "$KATANA_DB_PATH" > "$KATANA_LOG_FILE" 2>&1 &
KATANA_PID=$!
sleep 5
echo

# Check if katana is still running after the sleep
if ! kill -0 $KATANA_PID 2>/dev/null; then
  print_error "Katana process failed to start or terminated unexpectedly"
  exit 1
fi

echo -e "\033[1mSetting up $DOJO_EXAMPLE project:\033[0m"
# Clone Dojo repository if not already present
if [ ! -d "$DOJO_PATH" ]; then
  if [ -n "$DOJO_TAG" ]; then
    echo "Cloning Dojo repository (tag: $DOJO_TAG) to $DOJO_PATH"
    git clone --depth 1 --branch "$DOJO_TAG" "$DOJO_REPO" "$DOJO_PATH"
  else
    echo "Cloning Dojo repository to $DOJO_PATH"
    git clone "$DOJO_REPO" "$DOJO_PATH" --depth 1
  fi
fi

# Build and migrate Dojo example project
echo "Building and migrating Dojo example project at $DOJO_PATH/$DOJO_EXAMPLE"
cd $DOJO_PATH/$DOJO_EXAMPLE
$SOZO_BIN build
$SOZO_BIN migrate --account-address $ACCOUNT_ADDRESS --private-key $PRIVATE_KEY

echo "Stopping katana"
kill $KATANA_PID || true
while kill -0 $KATANA_PID 2>/dev/null; do
    echo "Waiting for katana to stop..."
    sleep 2
done

# Get katana version for archive name
KATANA_VERSION=$($KATANA_BIN -V | cut -d' ' -f2)
KATANA_VERSION_SAFE=$(echo $KATANA_VERSION | tr '.' '_')
ARCHIVE_NAME="${KATANA_VERSION_SAFE}.tar.gz"
ARCHIVE_PATH="$OUTPUT_DIR/$ARCHIVE_NAME"

mkdir -p $DB_OUTPUT_DIR
tar -czf $ARCHIVE_PATH -C $(dirname $KATANA_DB_PATH) $(basename $KATANA_DB_PATH)
echo "Database generation complete. Archive created at $ARCHIVE_PATH"
