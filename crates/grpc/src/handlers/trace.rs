//! Starknet trace service handler implementation.

use katana_pool::TransactionPool;
use katana_primitives::Felt;
use katana_provider::{ProviderFactory, ProviderRO};
use katana_rpc_server::starknet::PendingBlockProvider;
use tonic::{Request, Response, Status};

use crate::conversion::block_id_from_proto;
use crate::handlers::StarknetService;
use crate::protos::starknet::starknet_trace_server::StarknetTrace;
use crate::protos::starknet::{
    SimulateTransactionsRequest, SimulateTransactionsResponse, TraceBlockTransactionsRequest,
    TraceBlockTransactionsResponse, TraceTransactionRequest, TraceTransactionResponse,
};

#[tonic::async_trait]
impl<Pool, PP, PF> StarknetTrace for StarknetService<Pool, PP, PF>
where
    Pool: TransactionPool + 'static,
    PP: PendingBlockProvider,
    PF: ProviderFactory,
    <PF as ProviderFactory>::Provider: ProviderRO,
{
    async fn trace_transaction(
        &self,
        request: Request<TraceTransactionRequest>,
    ) -> Result<Response<TraceTransactionResponse>, Status> {
        let _tx_hash = Felt::try_from(
            request
                .into_inner()
                .transaction_hash
                .as_ref()
                .ok_or_else(|| Status::invalid_argument("Missing transaction_hash"))?,
        )?;

        // Trace requires access to the executor - not yet implemented
        Err(Status::unimplemented("trace_transaction not yet implemented for gRPC"))
    }

    async fn simulate_transactions(
        &self,
        _request: Request<SimulateTransactionsRequest>,
    ) -> Result<Response<SimulateTransactionsResponse>, Status> {
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
        let _confirmed_block_id = match block_id {
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

        // Trace requires access to the executor - not yet implemented
        Err(Status::unimplemented("trace_block_transactions not yet implemented for gRPC"))
    }
}
