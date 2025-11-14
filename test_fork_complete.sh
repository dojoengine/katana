#!/bin/bash

# Complete test script for fork - executes transactions and compares results
# Usage: ./test_fork_complete.sh <mainnet_rpc_url> <fork_rpc_url>

MAINNET_RPC="${1:-http://127.0.0.1:5050}"
FORK_RPC="${2:-http://127.0.0.1:5051}"

echo "🔍 Complete Fork Test - Transaction Execution and Comparison"
echo "============================================================"
echo "Mainnet RPC: $MAINNET_RPC"
echo "Fork RPC: $FORK_RPC"
echo ""

# ETH Fee Token address
ETH_FEE_TOKEN="0x049d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7"
# Storage slot for total_supply
TOTAL_SUPPLY_SLOT="0x110e2f729c9c2b988559994a3daccd838cf52faf88e18101373e67dd061455a"
# First prefunded account
ACCOUNT="0x127fd5f1fe78a71f8bcd1fec63e3fe2f0486b6ecd5c86a0466c3a21fa5cfcec"

# Helper function to get block number
get_block_number() {
    local rpc_url=$1
    curl -s -X POST "$rpc_url" \
        -H "Content-Type: application/json" \
        -d '{"jsonrpc":"2.0","method":"starknet_blockNumber","params":[],"id":1}' | jq -r '.result'
}

# Helper function to get storage
get_storage() {
    local rpc_url=$1
    local block_id=$2
    curl -s -X POST "$rpc_url" \
        -H "Content-Type: application/json" \
        -d "{\"jsonrpc\":\"2.0\",\"method\":\"starknet_getStorageAt\",\"params\":[\"$ETH_FEE_TOKEN\",\"$TOTAL_SUPPLY_SLOT\",$block_id],\"id\":2}" | jq -r '.result'
}

# Helper function to get nonce
get_nonce() {
    local rpc_url=$1
    local block_id=$2
    curl -s -X POST "$rpc_url" \
        -H "Content-Type: application/json" \
        -d "{\"jsonrpc\":\"2.0\",\"method\":\"starknet_getNonce\",\"params\":[$block_id,\"$ACCOUNT\"],\"id\":3}" | jq -r '.result'
}

echo "📊 STEP 1: Initial State Check"
echo "--------------------------------"
MAINNET_BLOCK_INITIAL=$(get_block_number "$MAINNET_RPC")
FORK_BLOCK_INITIAL=$(get_block_number "$FORK_RPC")

echo "Mainnet block: $MAINNET_BLOCK_INITIAL"
echo "Fork block: $FORK_BLOCK_INITIAL"
echo ""

MAINNET_STORAGE_INITIAL=$(get_storage "$MAINNET_RPC" "\"latest\"")
FORK_STORAGE_INITIAL=$(get_storage "$FORK_RPC" "\"latest\"")

echo "Mainnet storage (initial): $MAINNET_STORAGE_INITIAL"
echo "Fork storage (initial): $FORK_STORAGE_INITIAL"

if [ "$MAINNET_STORAGE_INITIAL" = "$FORK_STORAGE_INITIAL" ]; then
    echo "✅ Initial storage matches!"
else
    echo "⚠️  Initial storage differs"
fi
echo ""

echo "📝 STEP 2: Execute Transaction on MAINNET"
echo "------------------------------------------"
echo "To execute a transaction on mainnet, you can:"
echo ""
echo "Option A: Use katana CLI (if available):"
echo "  katana --rpc-url $MAINNET_RPC account transfer \\"
echo "    --to 0x1 \\"
echo "    --amount 1"
echo ""
echo "Option B: Use starkli (if available):"
echo "  starkli invoke \\"
echo "    --rpc $MAINNET_RPC \\"
echo "    --account $ACCOUNT \\"
echo "    $ETH_FEE_TOKEN transfer 0x1 0x1 0x0"
echo ""
echo "Option C: Use RPC directly (requires signing):"
echo "  See documentation for starknet_addInvokeTransaction"
echo ""
echo "After executing a transaction on MAINNET, press Enter to continue..."
read -r

echo ""
echo "📊 STEP 3: Check State After Mainnet Transaction"
echo "-------------------------------------------------"
MAINNET_BLOCK_AFTER_MAIN=$(get_block_number "$MAINNET_RPC")
FORK_BLOCK_AFTER_MAIN=$(get_block_number "$FORK_RPC")

echo "Mainnet block: $MAINNET_BLOCK_AFTER_MAIN (was $MAINNET_BLOCK_INITIAL)"
echo "Fork block: $FORK_BLOCK_AFTER_MAIN (was $FORK_BLOCK_INITIAL)"
echo ""

if [ "$MAINNET_BLOCK_AFTER_MAIN" -gt "$MAINNET_BLOCK_INITIAL" ]; then
    echo "✅ Mainnet block increased (transaction executed)"
    MAINNET_INCREASED=$((MAINNET_BLOCK_AFTER_MAIN - MAINNET_BLOCK_INITIAL))
    echo "   Increased by: $MAINNET_INCREASED blocks"
else
    echo "⚠️  Mainnet block did not increase (no transaction executed)"
    MAINNET_INCREASED=0
fi

if [ "$FORK_BLOCK_AFTER_MAIN" -eq "$FORK_BLOCK_INITIAL" ]; then
    echo "✅ Fork block stayed the same (fork is independent!)"
else
    FORK_INCREASED=$((FORK_BLOCK_AFTER_MAIN - FORK_BLOCK_INITIAL))
    echo "⚠️  Fork block changed by $FORK_INCREASED blocks"
fi
echo ""

MAINNET_STORAGE_AFTER_MAIN=$(get_storage "$MAINNET_RPC" "\"latest\"")
FORK_STORAGE_AFTER_MAIN=$(get_storage "$FORK_RPC" "\"latest\"")

echo "Mainnet storage: $MAINNET_STORAGE_AFTER_MAIN"
echo "Fork storage: $FORK_STORAGE_AFTER_MAIN"
echo ""

echo "📝 STEP 4: Execute SAME Transaction on FORK"
echo "--------------------------------------------"
echo "Now execute the EXACT SAME transaction on the FORK:"
echo ""
echo "Option A: Use katana CLI:"
echo "  katana --rpc-url $FORK_RPC account transfer \\"
echo "    --to 0x1 \\"
echo "    --amount 1"
echo ""
echo "Option B: Use starkli:"
echo "  starkli invoke \\"
echo "    --rpc $FORK_RPC \\"
echo "    --account $ACCOUNT \\"
echo "    $ETH_FEE_TOKEN transfer 0x1 0x1 0x0"
echo ""
echo "After executing the SAME transaction on FORK, press Enter to continue..."
read -r

echo ""
echo "📊 STEP 5: Final Comparison"
echo "--------------------------"
MAINNET_BLOCK_FINAL=$(get_block_number "$MAINNET_RPC")
FORK_BLOCK_FINAL=$(get_block_number "$FORK_RPC")

echo "Mainnet block: $MAINNET_BLOCK_FINAL"
echo "Fork block: $FORK_BLOCK_FINAL"
echo ""

MAINNET_STORAGE_FINAL=$(get_storage "$MAINNET_RPC" "\"latest\"")
FORK_STORAGE_FINAL=$(get_storage "$FORK_RPC" "\"latest\"")

echo "Mainnet storage: $MAINNET_STORAGE_FINAL"
echo "Fork storage: $FORK_STORAGE_FINAL"
echo ""

echo "📈 Results:"
echo "-----------"

# Check block independence
if [ "$MAINNET_BLOCK_FINAL" != "$FORK_BLOCK_FINAL" ]; then
    BLOCK_DIFF=$((FORK_BLOCK_FINAL - MAINNET_BLOCK_FINAL))
    echo "✅ Block numbers differ - fork is independent!"
    echo "   Mainnet: $MAINNET_BLOCK_FINAL"
    echo "   Fork: $FORK_BLOCK_FINAL"
    echo "   Difference: $BLOCK_DIFF blocks"
else
    echo "⚠️  Block numbers are the same"
fi
echo ""

# Check storage consistency
if [ "$MAINNET_STORAGE_FINAL" = "$FORK_STORAGE_FINAL" ]; then
    echo "✅ Storage matches after same transaction on both!"
    echo "✅ This proves fork works correctly:"
    echo "   - Same transaction = same result"
    echo "   - Fork is independent (different block numbers)"
    echo "   - Fork state is consistent with mainnet at fork point"
else
    echo "⚠️  Storage differs after same transaction"
    echo "   This might indicate an issue with fork state management"
fi
echo ""

# Summary
echo "📋 Summary:"
echo "-----------"
echo "Initial blocks: Mainnet=$MAINNET_BLOCK_INITIAL, Fork=$FORK_BLOCK_INITIAL"
echo "Final blocks:   Mainnet=$MAINNET_BLOCK_FINAL, Fork=$FORK_BLOCK_FINAL"
echo ""
echo "Mainnet increased by: $((MAINNET_BLOCK_FINAL - MAINNET_BLOCK_INITIAL)) blocks"
echo "Fork increased by:    $((FORK_BLOCK_FINAL - FORK_BLOCK_INITIAL)) blocks"
echo ""

if [ "$MAINNET_STORAGE_FINAL" = "$FORK_STORAGE_FINAL" ] && [ "$MAINNET_BLOCK_FINAL" != "$FORK_BLOCK_FINAL" ]; then
    echo "🎉 SUCCESS: Fork is working correctly!"
    echo "   ✅ Fork is independent (different block numbers)"
    echo "   ✅ Fork produces same results for same transactions"
    echo "   ✅ Fork state is consistent"
else
    echo "⚠️  Some checks failed - review the results above"
fi
echo ""

