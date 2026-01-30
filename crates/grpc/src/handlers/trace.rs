//! Starknet trace service handler implementation.

use katana_primitives::Felt;
use tonic::{Request, Response, Status};

use crate::convert::block_id_from_proto;
use crate::error::IntoGrpcResult;
use crate::handlers::StarknetHandler;
use crate::protos::starknet::starknet_trace_server::StarknetTrace;
use crate::protos::starknet::{
    SimulateTransactionsRequest, SimulateTransactionsResponse, TraceBlockTransactionsRequest,
    TraceBlockTransactionsResponse, TraceTransactionRequest, TraceTransactionResponse,
};

/// Trait for the inner handler that provides Starknet Trace API functionality.
#[tonic::async_trait]
pub trait StarknetTraceApiProvider: Clone + Send + Sync + 'static {
    /// Returns the trace for a transaction.
    async fn trace_transaction(
        &self,
        transaction_hash: katana_primitives::Felt,
    ) -> Result<katana_rpc_types::trace::TxTrace, katana_rpc_api::error::starknet::StarknetApiError>;

    /// Simulates transactions and returns traces.
    async fn simulate_transactions(
        &self,
        block_id: katana_primitives::block::BlockIdOrTag,
        transactions: Vec<katana_rpc_types::broadcasted::BroadcastedTx>,
        simulation_flags: Vec<katana_rpc_types::SimulationFlag>,
    ) -> Result<
        katana_rpc_types::trace::SimulatedTransactionsResponse,
        katana_rpc_api::error::starknet::StarknetApiError,
    >;

    /// Returns traces for all transactions in a block.
    async fn trace_block_transactions(
        &self,
        block_id: katana_primitives::block::ConfirmedBlockIdOrTag,
    ) -> Result<
        katana_rpc_types::trace::TraceBlockTransactionsResponse,
        katana_rpc_api::error::starknet::StarknetApiError,
    >;
}

#[tonic::async_trait]
impl<T: StarknetTraceApiProvider> StarknetTrace for StarknetHandler<T> {
    async fn trace_transaction(
        &self,
        request: Request<TraceTransactionRequest>,
    ) -> Result<Response<TraceTransactionResponse>, Status> {
        let tx_hash = Felt::try_from(
            request
                .into_inner()
                .transaction_hash
                .as_ref()
                .ok_or_else(|| Status::invalid_argument("Missing transaction_hash"))?,
        )?;

        let trace = self.inner.trace_transaction(tx_hash).await.into_grpc_result()?;

        Ok(Response::new(trace.into()))
    }

    async fn simulate_transactions(
        &self,
        _request: Request<SimulateTransactionsRequest>,
    ) -> Result<Response<SimulateTransactionsResponse>, Status> {
        // Would need to convert proto transactions to RPC transactions
        Err(Status::unimplemented(
            "simulate_transactions requires full transaction conversion from proto",
        ))
    }

    async fn trace_block_transactions(
        &self,
        request: Request<TraceBlockTransactionsRequest>,
    ) -> Result<Response<TraceBlockTransactionsResponse>, Status> {
        let block_id = block_id_from_proto(request.into_inner().block_id.as_ref())?;

        // Convert BlockIdOrTag to ConfirmedBlockIdOrTag
        let confirmed_block_id = match block_id {
            katana_primitives::block::BlockIdOrTag::Number(n) => {
                katana_primitives::block::ConfirmedBlockIdOrTag::Number(n)
            }
            katana_primitives::block::BlockIdOrTag::Hash(h) => {
                katana_primitives::block::ConfirmedBlockIdOrTag::Hash(h)
            }
            katana_primitives::block::BlockIdOrTag::Latest => {
                katana_primitives::block::ConfirmedBlockIdOrTag::Latest
            }
            katana_primitives::block::BlockIdOrTag::PreConfirmed => {
                return Err(Status::invalid_argument("Pending block does not have traces"));
            }
            katana_primitives::block::BlockIdOrTag::L1Accepted => {
                katana_primitives::block::ConfirmedBlockIdOrTag::L1Accepted
            }
        };

        let response =
            self.inner.trace_block_transactions(confirmed_block_id).await.into_grpc_result()?;

        Ok(Response::new(response.into()))
    }
}
