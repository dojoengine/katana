//! Extension to the Starknet JSON-RPC API for list endpoints. These endpoints shouldn't be relied upon as they may change or be removed in the future.

use jsonrpsee::core::RpcResult;
use jsonrpsee::proc_macros::rpc;
use katana_primitives::transaction::TxNumber;
use katana_rpc_types::list::{
    GetBlocksRequest, GetBlocksResponse, GetTransactionsRequest, GetTransactionsResponse,
};

/// Extension API for retrieving lists of blocks and transactions.
///
/// These endpoints are primarily intended for stateless blockchain explorers
/// to display lists of blocks and transactions with range-based queries.
#[cfg_attr(not(feature = "client"), rpc(server, namespace = "starknet"))]
#[cfg_attr(feature = "client", rpc(client, server, namespace = "starknet"))]
pub trait StarknetApiExt {
    /// Returns a list of blocks within the specified range.
    ///
    /// This endpoint accepts a range of block numbers and returns blocks
    /// within that range. Set `descending: true` to get results in
    /// descending order (newest first). Use `limit` to control the
    /// maximum number of blocks returned.
    #[method(name = "getBlocks")]
    async fn get_blocks(&self, request: GetBlocksRequest) -> RpcResult<GetBlocksResponse>;

    /// Returns a list of transactions within the specified range.
    ///
    /// This endpoint accepts a range of transaction numbers and returns
    /// transactions within that range. Set `descending: true` to get
    /// results in descending order (newest first). Use `limit` to control
    /// the maximum number of transactions returned.
    #[method(name = "getTransactions")]
    async fn get_transactions(
        &self,
        request: GetTransactionsRequest,
    ) -> RpcResult<GetTransactionsResponse>;

    /// Get the most recent accepted transaction number.
    ///
    /// Similar to `starknet_blockNumber` but for transaction.
    #[method(name = "transactionNumber")]
    async fn transaction_number(&self) -> RpcResult<TxNumber>;
}
