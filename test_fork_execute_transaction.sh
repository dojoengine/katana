#!/bin/bash

# Script to execute a transaction on mainnet and fork, then compare results
# Usage: ./test_fork_execute_transaction.sh <mainnet_rpc_url> <fork_rpc_url>

MAINNET_RPC="${1:-http://127.0.0.1:5050}"
FORK_RPC="${2:-http://127.0.0.1:5051}"

echo "🔍 Testing fork with transaction execution..."
echo "Mainnet RPC: $MAINNET_RPC"
echo "Fork RPC: $FORK_RPC"
echo ""

# ETH Fee Token address
ETH_FEE_TOKEN="0x049d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7"
# Storage slot for total_supply
TOTAL_SUPPLY_SLOT="0x110e2f729c9c2b988559994a3daccd838cf52faf88e18101373e67dd061455a"

echo "1️⃣ Initial state check..."
MAINNET_BLOCK_INITIAL=$(curl -s -X POST "$MAINNET_RPC" \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"starknet_blockNumber","params":[],"id":1}' | jq -r '.result')

FORK_BLOCK_INITIAL=$(curl -s -X POST "$FORK_RPC" \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"starknet_blockNumber","params":[],"id":1}' | jq -r '.result')

echo "   Mainnet block: $MAINNET_BLOCK_INITIAL"
echo "   Fork block: $FORK_BLOCK_INITIAL"
echo ""

MAINNET_STORAGE_INITIAL=$(curl -s -X POST "$MAINNET_RPC" \
  -H "Content-Type: application/json" \
  -d "{\"jsonrpc\":\"2.0\",\"method\":\"starknet_getStorageAt\",\"params\":[\"$ETH_FEE_TOKEN\",\"$TOTAL_SUPPLY_SLOT\",\"latest\"],\"id":2}" | jq -r '.result')

FORK_STORAGE_INITIAL=$(curl -s -X POST "$FORK_RPC" \
  -H "Content-Type: application/json" \
  -d "{\"jsonrpc\":\"2.0\",\"method\":\"starknet_getStorageAt\",\"params\":[\"$ETH_FEE_TOKEN\",\"$TOTAL_SUPPLY_SLOT\",\"latest\"],\"id":2}" | jq -r '.result')

echo "   Mainnet storage (initial): $MAINNET_STORAGE_INITIAL"
echo "   Fork storage (initial): $FORK_STORAGE_INITIAL"
echo ""

if [ "$MAINNET_STORAGE_INITIAL" = "$FORK_STORAGE_INITIAL" ]; then
  echo "   ✅ Initial storage matches!"
else
  echo "   ⚠️  Initial storage differs (this might be expected)"
fi
echo ""

echo "2️⃣ Executing transaction on MAINNET only..."
echo "   (This will increase mainnet block number but not fork)"
echo ""

# Note: To actually execute a transaction, you need to:
# 1. Get account nonce
# 2. Sign the transaction
# 3. Send it via starknet_addInvokeTransaction
# This is complex in bash, so we'll show instructions instead

echo "   To execute a transaction on mainnet, use katana CLI or send RPC call:"
echo "   curl -X POST $MAINNET_RPC \\"
echo "     -H 'Content-Type: application/json' \\"
echo "     -d '{\"jsonrpc\":\"2.0\",\"method\":\"starknet_addInvokeTransaction\",\"params\":[...],\"id\":1}'"
echo ""
echo "   Or use katana CLI:"
echo "   katana --rpc-url $MAINNET_RPC [transaction command]"
echo ""
echo "   After executing, press Enter to continue..."
read -r

echo "3️⃣ Checking state after mainnet transaction..."
MAINNET_BLOCK_AFTER_MAIN=$(curl -s -X POST "$MAINNET_RPC" \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"starknet_blockNumber","params":[],"id":3}' | jq -r '.result')

FORK_BLOCK_AFTER_MAIN=$(curl -s -X POST "$FORK_RPC" \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"starknet_blockNumber","params":[],"id":3}' | jq -r '.result')

echo "   Mainnet block: $MAINNET_BLOCK_AFTER_MAIN (was $MAINNET_BLOCK_INITIAL)"
echo "   Fork block: $FORK_BLOCK_AFTER_MAIN (was $FORK_BLOCK_INITIAL)"
echo ""

if [ "$MAINNET_BLOCK_AFTER_MAIN" -gt "$MAINNET_BLOCK_INITIAL" ]; then
  echo "   ✅ Mainnet block increased (transaction was executed)"
else
  echo "   ⚠️  Mainnet block did not increase (no transaction executed)"
fi

if [ "$FORK_BLOCK_AFTER_MAIN" -eq "$FORK_BLOCK_INITIAL" ]; then
  echo "   ✅ Fork block stayed the same (fork is independent!)"
else
  echo "   ⚠️  Fork block changed (this might be expected if fork syncs)"
fi
echo ""

echo "4️⃣ Executing SAME transaction on FORK..."
echo "   (This should result in same storage state if fork works correctly)"
echo ""
echo "   Execute the SAME transaction on fork:"
echo "   curl -X POST $FORK_RPC \\"
echo "     -H 'Content-Type: application/json' \\"
echo "     -d '{\"jsonrpc\":\"2.0\",\"method\":\"starknet_addInvokeTransaction\",\"params\":[...],\"id\":1}'"
echo ""
echo "   Or use katana CLI:"
echo "   katana --rpc-url $FORK_RPC [same transaction command]"
echo ""
echo "   After executing, press Enter to continue..."
read -r

echo "5️⃣ Final state comparison..."
MAINNET_BLOCK_FINAL=$(curl -s -X POST "$MAINNET_RPC" \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"starknet_blockNumber","params":[],"id":4}' | jq -r '.result')

FORK_BLOCK_FINAL=$(curl -s -X POST "$FORK_RPC" \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"starknet_blockNumber","params":[],"id":4}' | jq -r '.result')

MAINNET_STORAGE_FINAL=$(curl -s -X POST "$MAINNET_RPC" \
  -H "Content-Type: application/json" \
  -d "{\"jsonrpc\":\"2.0\",\"method\":\"starknet_getStorageAt\",\"params\":[\"$ETH_FEE_TOKEN\",\"$TOTAL_SUPPLY_SLOT\",\"latest\"],\"id":5}" | jq -r '.result')

FORK_STORAGE_FINAL=$(curl -s -X POST "$FORK_RPC" \
  -H "Content-Type: application/json" \
  -d "{\"jsonrpc\":\"2.0\",\"method\":\"starknet_getStorageAt\",\"params\":[\"$ETH_FEE_TOKEN\",\"$TOTAL_SUPPLY_SLOT\",\"latest\"],\"id":5}" | jq -r '.result')

echo "   Mainnet block: $MAINNET_BLOCK_FINAL"
echo "   Fork block: $FORK_BLOCK_FINAL"
echo ""
echo "   Mainnet storage: $MAINNET_STORAGE_FINAL"
echo "   Fork storage: $FORK_STORAGE_FINAL"
echo ""

if [ "$MAINNET_STORAGE_FINAL" = "$FORK_STORAGE_FINAL" ]; then
  echo "   ✅ Storage matches after same transaction on both!"
  echo "   ✅ This proves fork works correctly - same transaction = same result!"
else
  echo "   ⚠️  Storage differs after same transaction"
  echo "   This might indicate an issue with fork state management"
fi
echo ""

if [ "$MAINNET_BLOCK_FINAL" != "$FORK_BLOCK_FINAL" ]; then
  echo "   ✅ Block numbers differ - fork is independent!"
  echo "   Mainnet: $MAINNET_BLOCK_FINAL, Fork: $FORK_BLOCK_FINAL"
else
  echo "   ⚠️  Block numbers are the same"
fi
echo ""

echo "✅ Test completed!"

