//! Implementation of list endpoints for the Starknet API.

use std::ops::Range;

use jsonrpsee::core::{async_trait, RpcResult};
use katana_primitives::transaction::TxNumber;
use katana_provider::traits::block::{BlockNumberProvider, BlockProvider};
use katana_provider::traits::transaction::TransactionProvider;
use katana_rpc_api::error::starknet::StarknetApiError;
use katana_rpc_api::starknet_ext::StarknetApiExtServer;
use katana_rpc_types::list::{
    GetBlocksRequest, GetBlocksResponse, GetTransactionsRequest, GetTransactionsResponse,
};
use katana_rpc_types::transaction::Tx;

use super::{StarknetApi, StarknetApiResult};

#[async_trait]
impl<EF> StarknetApiExtServer for StarknetApi<EF>
where
    EF: katana_executor::ExecutorFactory,
{
    async fn get_blocks(&self, request: GetBlocksRequest) -> RpcResult<GetBlocksResponse> {
        Ok(self.blocks(request).await?)
    }

    async fn get_transactions(
        &self,
        request: GetTransactionsRequest,
    ) -> RpcResult<GetTransactionsResponse> {
        Ok(self.get_transactions(request).await?)
    }

    async fn transaction_number(&self) -> RpcResult<TxNumber> {
        Ok(self.total_transactions().await?)
    }
}
