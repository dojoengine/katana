#!/bin/bash

# Test script to verify forking works
# Usage: ./test_fork.sh <mainnet_rpc_url> <fork_rpc_url> [fork_block_number]

MAINNET_RPC="${1:-http://127.0.0.1:5050}"
FORK_RPC="${2:-http://127.0.0.1:5051}"
FORK_BLOCK="${3:-11}"

echo "🔍 Testing fork..."
echo "Mainnet RPC: $MAINNET_RPC"
echo "Fork RPC: $FORK_RPC"
echo "Fork block: $FORK_BLOCK"
echo ""

# ETH Fee Token address
ETH_FEE_TOKEN="0x049d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7"
# Storage slot for ERC20 total_supply
TOTAL_SUPPLY_SLOT="0x110e2f729c9c2b988559994a3daccd838cf52faf88e18101373e67dd061455a"

echo "1️⃣ Checking block numbers..."
MAINNET_BLOCK=$(curl -s -X POST "$MAINNET_RPC" \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"starknet_blockNumber","params":[],"id":1}' | jq -r '.result')

FORK_BLOCK_NUM=$(curl -s -X POST "$FORK_RPC" \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"starknet_blockNumber","params":[],"id":1}' | jq -r '.result')

echo "   Mainnet block number: $MAINNET_BLOCK"
echo "   Fork block number: $FORK_BLOCK_NUM"
echo ""

if [ "$FORK_BLOCK_NUM" -lt "$FORK_BLOCK" ]; then
  echo "   ⚠️  Fork block number should be >= $FORK_BLOCK (fork point)"
else
  echo "   ✅ Fork block number is correct"
fi
echo ""

echo "2️⃣ Checking storage at fork point (block $FORK_BLOCK)..."
MAINNET_STORAGE=$(curl -s -X POST "$MAINNET_RPC" \
  -H "Content-Type: application/json" \
  -d "{\"jsonrpc\":\"2.0\",\"method\":\"starknet_getStorageAt\",\"params\":[\"$ETH_FEE_TOKEN\",\"$TOTAL_SUPPLY_SLOT\",{\"block_number\":$FORK_BLOCK}],\"id\":2}" | jq -r '.result')

FORK_STORAGE=$(curl -s -X POST "$FORK_RPC" \
  -H "Content-Type: application/json" \
  -d "{\"jsonrpc\":\"2.0\",\"method\":\"starknet_getStorageAt\",\"params\":[\"$ETH_FEE_TOKEN\",\"$TOTAL_SUPPLY_SLOT\",{\"block_number\":$FORK_BLOCK}],\"id\":2}" | jq -r '.result')

echo "   Mainnet storage at block $FORK_BLOCK: $MAINNET_STORAGE"
echo "   Fork storage at block $FORK_BLOCK: $FORK_STORAGE"
echo ""

if [ "$MAINNET_STORAGE" = "$FORK_STORAGE" ]; then
  echo "   ✅ Storage values match at fork point!"
else
  echo "   ❌ Storage values do NOT match at fork point!"
fi
echo ""

echo "3️⃣ Checking latest block storage..."
MAINNET_LATEST_STORAGE=$(curl -s -X POST "$MAINNET_RPC" \
  -H "Content-Type: application/json" \
  -d "{\"jsonrpc\":\"2.0\",\"method\":\"starknet_getStorageAt\",\"params\":[\"$ETH_FEE_TOKEN\",\"$TOTAL_SUPPLY_SLOT\",\"latest\"],\"id\":3}" | jq -r '.result')

FORK_LATEST_STORAGE=$(curl -s -X POST "$FORK_RPC" \
  -H "Content-Type: application/json" \
  -d "{\"jsonrpc\":\"2.0\",\"method\":\"starknet_getStorageAt\",\"params\":[\"$ETH_FEE_TOKEN\",\"$TOTAL_SUPPLY_SLOT\",\"latest\"],\"id\":3}" | jq -r '.result')

echo "   Mainnet latest storage: $MAINNET_LATEST_STORAGE"
echo "   Fork latest storage: $FORK_LATEST_STORAGE"
echo ""

if [ "$MAINNET_LATEST_STORAGE" = "$FORK_LATEST_STORAGE" ]; then
  echo "   ✅ Latest storage values match!"
else
  echo "   ⚠️  Latest storage values differ (this is expected if you made changes in fork)"
fi
echo ""

echo "4️⃣ Checking if fork can read block from mainnet..."
FORK_BLOCK_11=$(curl -s -X POST "$FORK_RPC" \
  -H "Content-Type: application/json" \
  -d "{\"jsonrpc\":\"2.0\",\"method\":\"starknet_getBlockWithTxHashes\",\"params\":[{\"block_number\":$FORK_BLOCK}],\"id\":4}" | jq -r '.result.block_hash // "error"')

if [ "$FORK_BLOCK_11" != "error" ] && [ "$FORK_BLOCK_11" != "null" ]; then
  echo "   ✅ Fork can read block $FORK_BLOCK from mainnet (hash: $FORK_BLOCK_11)"
else
  echo "   ❌ Fork cannot read block $FORK_BLOCK from mainnet"
fi
echo ""

echo "✅ Fork test completed!"

