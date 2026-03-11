use jsonrpsee::core::{async_trait, RpcResult};
use katana_pool::TransactionPool;
use katana_primitives::block::{BlockIdOrTag, ConfirmedBlockIdOrTag};
use katana_primitives::transaction::TxHash;
use katana_provider::{ProviderFactory, ProviderRO};
use katana_rpc_api::starknet::StarknetTraceApiServer;
use katana_rpc_types::broadcasted::BroadcastedTx;
use katana_rpc_types::trace::{
    SimulatedTransactionsResponse, TraceBlockTransactionsResponse, TxTrace,
};
use katana_rpc_types::{BroadcastedTxWithChainId, SimulationFlag};

use super::StarknetApi;
use crate::starknet::pending::PendingBlockProvider;

#[async_trait]
impl<Pool, PoolTx, Pending, PF> StarknetTraceApiServer for StarknetApi<Pool, Pending, PF>
where
    Pool: TransactionPool<Transaction = PoolTx> + Send + Sync + 'static,
    PoolTx: From<BroadcastedTxWithChainId>,
    Pending: PendingBlockProvider,
    PF: ProviderFactory,
    <PF as ProviderFactory>::Provider: ProviderRO,
{
    async fn trace_transaction(&self, transaction_hash: TxHash) -> RpcResult<TxTrace> {
        Ok(self.trace(transaction_hash).await?)
    }

    async fn simulate_transactions(
        &self,
        block_id: BlockIdOrTag,
        transactions: Vec<BroadcastedTx>,
        simulation_flags: Vec<SimulationFlag>,
    ) -> RpcResult<SimulatedTransactionsResponse> {
        let transactions = self.simulate_txs(block_id, transactions, simulation_flags).await?;
        Ok(SimulatedTransactionsResponse { transactions })
    }

    async fn trace_block_transactions(
        &self,
        block_id: ConfirmedBlockIdOrTag,
    ) -> RpcResult<TraceBlockTransactionsResponse> {
        let traces = self.block_traces(block_id).await?;
        Ok(TraceBlockTransactionsResponse { traces })
    }
}
