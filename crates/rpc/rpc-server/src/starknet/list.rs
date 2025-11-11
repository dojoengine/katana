//! Implementation of list endpoints for the Starknet API.

use jsonrpsee::core::{async_trait, RpcResult};
use katana_pool::TransactionPool;
use katana_primitives::transaction::TxNumber;
use katana_rpc_api::starknet_ext::StarknetApiExtServer;
use katana_rpc_types::list::{
    GetBlocksRequest, GetBlocksResponse, GetTransactionsRequest, GetTransactionsResponse,
};

use super::StarknetApi;
use crate::starknet::pending::PendingBlockProvider;

#[async_trait]
impl<Pool, PP> StarknetApiExtServer for StarknetApi<Pool, PP>
where
    Pool: TransactionPool + 'static,
    PP: PendingBlockProvider,
{
    async fn get_blocks(&self, request: GetBlocksRequest) -> RpcResult<GetBlocksResponse> {
        Ok(self.blocks(request).await?)
    }

    async fn get_transactions(
        &self,
        request: GetTransactionsRequest,
    ) -> RpcResult<GetTransactionsResponse> {
        Ok(self.transactions(request).await?)
    }

    async fn transaction_number(&self) -> RpcResult<TxNumber> {
        Ok(self.total_transactions().await?)
    }
}
