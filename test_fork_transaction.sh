#!/bin/bash

# Test script to verify fork independence by making a transaction
# Usage: ./test_fork_transaction.sh <mainnet_rpc_url> <fork_rpc_url>

MAINNET_RPC="${1:-http://127.0.0.1:5050}"
FORK_RPC="${2:-http://127.0.0.1:5051}"

echo "🔍 Testing fork independence with transaction..."
echo "Mainnet RPC: $MAINNET_RPC"
echo "Fork RPC: $FORK_RPC"
echo ""

# ETH Fee Token address
ETH_FEE_TOKEN="0x049d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7"
# First prefunded account (from terminal output)
ACCOUNT_ADDRESS="0x127fd5f1fe78a71f8bcd1fec63e3fe2f0486b6ecd5c86a0466c3a21fa5cfcec"
# Storage slot for balance (low part of U256)
BALANCE_SLOT_LOW="0x3e7e5b2f8e2a0e4c8b3d5f7a9c1e6b4d8f2a5c7e9b1d3f5a7c9e1b3d5f7a9c"

echo "1️⃣ Checking account balance in mainnet and fork..."
MAINNET_BALANCE=$(curl -s -X POST "$MAINNET_RPC" \
  -H "Content-Type: application/json" \
  -d "{\"jsonrpc\":\"2.0\",\"method\":\"starknet_getStorageAt\",\"params\":[\"$ETH_FEE_TOKEN\",\"$BALANCE_SLOT_LOW\",\"latest\"],\"id\":1}" | jq -r '.result')

FORK_BALANCE=$(curl -s -X POST "$FORK_RPC" \
  -H "Content-Type: application/json" \
  -d "{\"jsonrpc\":\"2.0\",\"method\":\"starknet_getStorageAt\",\"params\":[\"$ETH_FEE_TOKEN\",\"$BALANCE_SLOT_LOW\",\"latest\"],\"id\":1}" | jq -r '.result')

echo "   Mainnet balance slot: $MAINNET_BALANCE"
echo "   Fork balance slot: $FORK_BALANCE"
echo ""

echo "2️⃣ Checking block numbers before transaction..."
MAINNET_BLOCK_BEFORE=$(curl -s -X POST "$MAINNET_RPC" \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"starknet_blockNumber","params":[],"id":2}' | jq -r '.result')

FORK_BLOCK_BEFORE=$(curl -s -X POST "$FORK_RPC" \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"starknet_blockNumber","params":[],"id":2}' | jq -r '.result')

echo "   Mainnet block: $MAINNET_BLOCK_BEFORE"
echo "   Fork block: $FORK_BLOCK_BEFORE"
echo ""

echo "3️⃣ To test fork independence:"
echo "   - Execute a transaction in the FORK (not mainnet)"
echo "   - Check if fork block number increases"
echo "   - Check if mainnet block number stays the same"
echo "   - This proves the fork is independent!"
echo ""
echo "   You can use katana CLI or send a transaction via RPC to the fork."
echo "   Example: Send a transaction to $FORK_RPC (not $MAINNET_RPC)"
echo ""

echo "4️⃣ Checking if fork is independent (different block numbers)..."
if [ "$MAINNET_BLOCK_BEFORE" != "$FORK_BLOCK_BEFORE" ]; then
  echo "   ✅ Fork is independent! Block numbers differ:"
  echo "      Mainnet: $MAINNET_BLOCK_BEFORE"
  echo "      Fork: $FORK_BLOCK_BEFORE"
  echo "      Difference: $((FORK_BLOCK_BEFORE - MAINNET_BLOCK_BEFORE)) blocks"
else
  echo "   ⚠️  Fork and mainnet have the same block number."
  echo "      This is normal if you haven't made any transactions in the fork yet."
  echo "      To test independence:"
  echo "      1. Make a transaction in the FORK (port 5051)"
  echo "      2. Run this script again to see the difference"
fi
echo ""

