//! Starknet service handler implementation.

use katana_pool::TransactionPool;
use katana_primitives::Felt;
use katana_provider::{ProviderFactory, ProviderRO};
use katana_rpc_api::starknet::RPC_SPEC_VERSION;
use katana_rpc_server::starknet::{PendingBlockProvider, StarknetApi};
use katana_rpc_types::{BroadcastedTxWithChainId, FunctionCall};
use tonic::{Request, Response, Status};

use crate::conversion::{block_id_from_proto, ProtoFeltVecExt};
use crate::error::IntoGrpcResult;
use crate::protos::starknet::starknet_server::Starknet;
use crate::protos::starknet::starknet_trace_server::StarknetTrace;
use crate::protos::starknet::starknet_write_server::StarknetWrite;
use crate::protos::starknet::{
    AddDeclareTransactionRequest, AddDeclareTransactionResponse,
    AddDeployAccountTransactionRequest, AddDeployAccountTransactionResponse,
    AddInvokeTransactionRequest, AddInvokeTransactionResponse, BlockHashAndNumberRequest,
    BlockHashAndNumberResponse, BlockNumberRequest, BlockNumberResponse, CallRequest, CallResponse,
    ChainIdRequest, ChainIdResponse, EstimateFeeRequest, EstimateFeeResponse,
    EstimateMessageFeeRequest, GetBlockRequest, GetBlockTransactionCountResponse,
    GetBlockWithReceiptsResponse, GetBlockWithTxHashesResponse, GetBlockWithTxsResponse,
    GetClassAtRequest, GetClassAtResponse, GetClassHashAtRequest, GetClassHashAtResponse,
    GetClassRequest, GetClassResponse, GetCompiledCasmRequest, GetCompiledCasmResponse,
    GetEventsRequest, GetEventsResponse, GetNonceRequest, GetNonceResponse, GetStateUpdateResponse,
    GetStorageAtRequest, GetStorageAtResponse, GetStorageProofRequest, GetStorageProofResponse,
    GetTransactionByBlockIdAndIndexRequest, GetTransactionByBlockIdAndIndexResponse,
    GetTransactionByHashRequest, GetTransactionByHashResponse, GetTransactionReceiptRequest,
    GetTransactionReceiptResponse, GetTransactionStatusRequest, GetTransactionStatusResponse,
    SimulateTransactionsRequest, SimulateTransactionsResponse, SpecVersionRequest,
    SpecVersionResponse, SyncingRequest, SyncingResponse, TraceBlockTransactionsRequest,
    TraceBlockTransactionsResponse, TraceTransactionRequest, TraceTransactionResponse,
};
use crate::protos::types::{Transaction as ProtoTx, TransactionReceipt as ProtoTransactionReceipt};

/// The main handler for Starknet gRPC services.
///
/// This struct wraps `StarknetApi` from `katana-rpc-server` and implements the gRPC
/// service traits by delegating to the underlying API.
pub struct StarknetService<Pool, PP, PF>
where
    Pool: TransactionPool,
    PP: PendingBlockProvider,
    PF: ProviderFactory,
{
    pub(crate) api: StarknetApi<Pool, PP, PF>,
}

impl<Pool, PP, PF> StarknetService<Pool, PP, PF>
where
    Pool: TransactionPool,
    PP: PendingBlockProvider,
    PF: ProviderFactory,
{
    pub fn new(api: StarknetApi<Pool, PP, PF>) -> Self {
        Self { api }
    }
}

impl<Pool, PP, PF> std::fmt::Debug for StarknetService<Pool, PP, PF>
where
    Pool: TransactionPool,
    PP: PendingBlockProvider,
    PF: ProviderFactory,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StarknetHandler").finish_non_exhaustive()
    }
}

impl<Pool, PP, PF> Clone for StarknetService<Pool, PP, PF>
where
    Pool: TransactionPool,
    PP: PendingBlockProvider,
    PF: ProviderFactory,
{
    fn clone(&self) -> Self {
        Self { api: self.api.clone() }
    }
}

/////////////////////////////////////////////////////////////////////////
/// Starknet Read Service Implementation
/////////////////////////////////////////////////////////////////////////

#[tonic::async_trait]
impl<Pool, PP, PF> Starknet for StarknetService<Pool, PP, PF>
where
    Pool: TransactionPool + 'static,
    PP: PendingBlockProvider,
    PF: ProviderFactory,
    <PF as ProviderFactory>::Provider: ProviderRO,
{
    async fn spec_version(
        &self,
        _request: Request<SpecVersionRequest>,
    ) -> Result<Response<SpecVersionResponse>, Status> {
        Ok(Response::new(SpecVersionResponse { version: RPC_SPEC_VERSION.to_string() }))
    }

    async fn get_block_with_tx_hashes(
        &self,
        request: Request<GetBlockRequest>,
    ) -> Result<Response<GetBlockWithTxHashesResponse>, Status> {
        let block_id = block_id_from_proto(request.into_inner().block_id.as_ref())?;
        let result = self.api.block_with_tx_hashes(block_id).await.into_grpc_result()?;
        Ok(Response::new(result.into()))
    }

    async fn get_block_with_txs(
        &self,
        request: Request<GetBlockRequest>,
    ) -> Result<Response<GetBlockWithTxsResponse>, Status> {
        let block_id = block_id_from_proto(request.into_inner().block_id.as_ref())?;
        let result = self.api.block_with_txs(block_id).await.into_grpc_result()?;
        Ok(Response::new(result.into()))
    }

    async fn get_block_with_receipts(
        &self,
        request: Request<GetBlockRequest>,
    ) -> Result<Response<GetBlockWithReceiptsResponse>, Status> {
        let block_id = block_id_from_proto(request.into_inner().block_id.as_ref())?;
        let result = self.api.block_with_receipts(block_id).await.into_grpc_result()?;
        Ok(Response::new(result.into()))
    }

    async fn get_state_update(
        &self,
        request: Request<GetBlockRequest>,
    ) -> Result<Response<GetStateUpdateResponse>, Status> {
        let block_id = block_id_from_proto(request.into_inner().block_id.as_ref())?;
        let result = self.api.state_update(block_id).await.into_grpc_result()?;
        Ok(Response::new(result.into()))
    }

    async fn get_storage_at(
        &self,
        request: Request<GetStorageAtRequest>,
    ) -> Result<Response<GetStorageAtResponse>, Status> {
        let req = request.into_inner();
        let block_id = block_id_from_proto(req.block_id.as_ref())?;
        let contract_address = Felt::try_from(
            req.contract_address
                .as_ref()
                .ok_or_else(|| Status::invalid_argument("Missing contract_address"))?,
        )?;
        let key = Felt::try_from(
            req.key.as_ref().ok_or_else(|| Status::invalid_argument("Missing key"))?,
        )?;

        let result =
            self.api.storage_at(contract_address.into(), key, block_id).await.into_grpc_result()?;

        Ok(Response::new(GetStorageAtResponse { value: Some(result.into()) }))
    }

    async fn get_transaction_status(
        &self,
        request: Request<GetTransactionStatusRequest>,
    ) -> Result<Response<GetTransactionStatusResponse>, Status> {
        let tx_hash = Felt::try_from(
            request
                .into_inner()
                .transaction_hash
                .as_ref()
                .ok_or_else(|| Status::invalid_argument("Missing transaction_hash"))?,
        )?;

        let status = self.api.transaction_status(tx_hash).await.into_grpc_result()?;

        let (finality_status, execution_status) = match status {
            katana_rpc_types::TxStatus::Received => ("RECEIVED".to_string(), String::new()),
            katana_rpc_types::TxStatus::Candidate => ("CANDIDATE".to_string(), String::new()),
            katana_rpc_types::TxStatus::PreConfirmed(exec) => {
                ("PRE_CONFIRMED".to_string(), execution_result_to_string(&exec))
            }
            katana_rpc_types::TxStatus::AcceptedOnL2(exec) => {
                ("ACCEPTED_ON_L2".to_string(), execution_result_to_string(&exec))
            }
            katana_rpc_types::TxStatus::AcceptedOnL1(exec) => {
                ("ACCEPTED_ON_L1".to_string(), execution_result_to_string(&exec))
            }
        };

        Ok(Response::new(GetTransactionStatusResponse { finality_status, execution_status }))
    }

    async fn get_transaction_by_hash(
        &self,
        request: Request<GetTransactionByHashRequest>,
    ) -> Result<Response<GetTransactionByHashResponse>, Status> {
        let tx_hash = Felt::try_from(
            request
                .into_inner()
                .transaction_hash
                .as_ref()
                .ok_or_else(|| Status::invalid_argument("Missing transaction_hash"))?,
        )?;

        let tx = self.api.transaction(tx_hash).await.into_grpc_result()?;

        Ok(Response::new(GetTransactionByHashResponse { transaction: Some(ProtoTx::from(tx)) }))
    }

    async fn get_transaction_by_block_id_and_index(
        &self,
        request: Request<GetTransactionByBlockIdAndIndexRequest>,
    ) -> Result<Response<GetTransactionByBlockIdAndIndexResponse>, Status> {
        let req = request.into_inner();
        let block_id = block_id_from_proto(req.block_id.as_ref())?;
        let index = req.index;

        let tx =
            self.api.transaction_by_block_id_and_index(block_id, index).await.into_grpc_result()?;

        Ok(Response::new(GetTransactionByBlockIdAndIndexResponse { transaction: Some(tx.into()) }))
    }

    async fn get_transaction_receipt(
        &self,
        request: Request<GetTransactionReceiptRequest>,
    ) -> Result<Response<GetTransactionReceiptResponse>, Status> {
        let tx_hash = Felt::try_from(
            request
                .into_inner()
                .transaction_hash
                .as_ref()
                .ok_or_else(|| Status::invalid_argument("Missing transaction_hash"))?,
        )?;

        let receipt = self.api.receipt(tx_hash).await.into_grpc_result()?;

        Ok(Response::new(GetTransactionReceiptResponse {
            receipt: Some(ProtoTransactionReceipt::from(&receipt)),
        }))
    }

    async fn get_class(
        &self,
        request: Request<GetClassRequest>,
    ) -> Result<Response<GetClassResponse>, Status> {
        let req = request.into_inner();
        let block_id = block_id_from_proto(req.block_id.as_ref())?;
        let class_hash = Felt::try_from(
            req.class_hash
                .as_ref()
                .ok_or_else(|| Status::invalid_argument("Missing class_hash"))?,
        )?;

        let class = self.api.class_at_hash(block_id, class_hash).await.into_grpc_result()?;

        // Convert class to proto - simplified for now
        Ok(Response::new(GetClassResponse {
            result: Some(crate::protos::starknet::get_class_response::Result::ContractClass(
                crate::protos::types::ContractClass {
                    sierra_program: Vec::new(), // Would need full conversion
                    contract_class_version: String::new(),
                    entry_points_by_type: None,
                    abi: serde_json::to_string(&class).unwrap_or_default(),
                },
            )),
        }))
    }

    async fn get_class_hash_at(
        &self,
        request: Request<GetClassHashAtRequest>,
    ) -> Result<Response<GetClassHashAtResponse>, Status> {
        let req = request.into_inner();
        let block_id = block_id_from_proto(req.block_id.as_ref())?;
        let contract_address = Felt::try_from(
            req.contract_address
                .as_ref()
                .ok_or_else(|| Status::invalid_argument("Missing contract_address"))?,
        )?;

        let class_hash = self
            .api
            .class_hash_at_address(block_id, contract_address.into())
            .await
            .into_grpc_result()?;

        Ok(Response::new(GetClassHashAtResponse { class_hash: Some(class_hash.into()) }))
    }

    async fn get_class_at(
        &self,
        request: Request<GetClassAtRequest>,
    ) -> Result<Response<GetClassAtResponse>, Status> {
        let req = request.into_inner();
        let block_id = block_id_from_proto(req.block_id.as_ref())?;
        let contract_address = Felt::try_from(
            req.contract_address
                .as_ref()
                .ok_or_else(|| Status::invalid_argument("Missing contract_address"))?,
        )?;

        let class = self
            .api
            .class_at_address(block_id, contract_address.into())
            .await
            .into_grpc_result()?;

        // Convert class to proto - simplified for now
        Ok(Response::new(GetClassAtResponse {
            result: Some(crate::protos::starknet::get_class_at_response::Result::ContractClass(
                crate::protos::types::ContractClass {
                    sierra_program: Vec::new(),
                    contract_class_version: String::new(),
                    entry_points_by_type: None,
                    abi: serde_json::to_string(&class).unwrap_or_default(),
                },
            )),
        }))
    }

    async fn get_block_transaction_count(
        &self,
        request: Request<GetBlockRequest>,
    ) -> Result<Response<GetBlockTransactionCountResponse>, Status> {
        let block_id = block_id_from_proto(request.into_inner().block_id.as_ref())?;
        let count = self.api.block_tx_count(block_id).await.into_grpc_result()?;
        Ok(Response::new(GetBlockTransactionCountResponse { count }))
    }

    async fn call(&self, request: Request<CallRequest>) -> Result<Response<CallResponse>, Status> {
        let req = request.into_inner();
        let block_id = block_id_from_proto(req.block_id.as_ref())?;

        let function_call =
            req.request.ok_or_else(|| Status::invalid_argument("Missing request"))?;

        let contract_address = Felt::try_from(
            function_call
                .contract_address
                .as_ref()
                .ok_or_else(|| Status::invalid_argument("Missing contract_address"))?,
        )?;

        let entry_point_selector = Felt::try_from(
            function_call
                .entry_point_selector
                .as_ref()
                .ok_or_else(|| Status::invalid_argument("Missing entry_point_selector"))?,
        )?;

        let calldata = function_call.calldata.to_felts()?;

        let response = self
            .api
            .call_contract(
                FunctionCall {
                    calldata,
                    entry_point_selector,
                    contract_address: contract_address.into(),
                },
                block_id,
            )
            .await
            .into_grpc_result()?;

        Ok(Response::new(CallResponse {
            result: response.result.into_iter().map(Into::into).collect(),
        }))
    }

    async fn estimate_fee(
        &self,
        _request: Request<EstimateFeeRequest>,
    ) -> Result<Response<EstimateFeeResponse>, Status> {
        Err(Status::unimplemented("estimate_fee requires full transaction conversion"))
    }

    async fn estimate_message_fee(
        &self,
        _request: Request<EstimateMessageFeeRequest>,
    ) -> Result<Response<EstimateFeeResponse>, Status> {
        Err(Status::unimplemented("estimate_message_fee requires full message conversion"))
    }

    async fn block_number(
        &self,
        _request: Request<BlockNumberRequest>,
    ) -> Result<Response<BlockNumberResponse>, Status> {
        let result = self
            .api
            .on_io_blocking_task(move |api| {
                use katana_provider::api::block::BlockNumberProvider;
                let block_number = api.storage().provider().latest_number()?;
                Ok(katana_rpc_types::block::BlockNumberResponse { block_number })
            })
            .await
            .into_grpc_result()?
            .map_err(crate::error::to_status)?;

        Ok(Response::new(BlockNumberResponse { block_number: result.block_number }))
    }

    async fn block_hash_and_number(
        &self,
        request: Request<BlockHashAndNumberRequest>,
    ) -> Result<Response<BlockHashAndNumberResponse>, Status> {
        let _ = request;

        let result = self.api.block_hash_and_number().await.into_grpc_result()?;

        Ok(Response::new(BlockHashAndNumberResponse {
            block_hash: Some(result.block_hash.into()),
            block_number: result.block_number,
        }))
    }

    async fn chain_id(
        &self,
        _request: Request<ChainIdRequest>,
    ) -> Result<Response<ChainIdResponse>, Status> {
        let chain_id = self.api.chain_id();
        Ok(Response::new(ChainIdResponse { chain_id: format!("{chain_id:#x}") }))
    }

    async fn syncing(
        &self,
        _request: Request<SyncingRequest>,
    ) -> Result<Response<SyncingResponse>, Status> {
        // Katana doesn't support syncing status yet
        Ok(Response::new(SyncingResponse {
            result: Some(crate::protos::starknet::syncing_response::Result::NotSyncing(true)),
        }))
    }

    async fn get_events(
        &self,
        _request: Request<GetEventsRequest>,
    ) -> Result<Response<GetEventsResponse>, Status> {
        Err(Status::unimplemented("get_events requires full filter conversion"))
    }

    async fn get_nonce(
        &self,
        request: Request<GetNonceRequest>,
    ) -> Result<Response<GetNonceResponse>, Status> {
        let req = request.into_inner();
        let block_id = block_id_from_proto(req.block_id.as_ref())?;
        let contract_address = Felt::try_from(
            req.contract_address
                .as_ref()
                .ok_or_else(|| Status::invalid_argument("Missing contract_address"))?,
        )?;

        let nonce =
            self.api.nonce_at(block_id, contract_address.into()).await.into_grpc_result()?;

        Ok(Response::new(GetNonceResponse { nonce: Some(nonce.into()) }))
    }

    async fn get_compiled_casm(
        &self,
        _request: Request<GetCompiledCasmRequest>,
    ) -> Result<Response<GetCompiledCasmResponse>, Status> {
        Err(Status::unimplemented("get_compiled_casm requires CASM conversion"))
    }

    async fn get_storage_proof(
        &self,
        _request: Request<GetStorageProofRequest>,
    ) -> Result<Response<GetStorageProofResponse>, Status> {
        Err(Status::unimplemented("get_storage_proof requires proof conversion"))
    }
}

/////////////////////////////////////////////////////////////////////////
/// Starknet Write Service Implementation
/////////////////////////////////////////////////////////////////////////

#[tonic::async_trait]
impl<Pool, PoolTx, PP, PF> StarknetWrite for StarknetService<Pool, PP, PF>
where
    Pool: TransactionPool<Transaction = PoolTx> + Send + Sync + 'static,
    PoolTx: From<BroadcastedTxWithChainId>,
    PP: PendingBlockProvider,
    PF: ProviderFactory,
{
    async fn add_invoke_transaction(
        &self,
        request: Request<AddInvokeTransactionRequest>,
    ) -> Result<Response<AddInvokeTransactionResponse>, Status> {
        let AddInvokeTransactionRequest { transaction } = request.into_inner();

        let tx = transaction.ok_or(Status::invalid_argument("missing transaction"))?;
        let response = self.api.add_invoke_tx(tx.try_into()?).await.into_grpc_result()?;

        Ok(Response::new(AddInvokeTransactionResponse {
            transaction_hash: Some(response.transaction_hash.into()),
        }))
    }

    async fn add_declare_transaction(
        &self,
        request: Request<AddDeclareTransactionRequest>,
    ) -> Result<Response<AddDeclareTransactionResponse>, Status> {
        let AddDeclareTransactionRequest { transaction } = request.into_inner();

        let tx = transaction.ok_or(Status::invalid_argument("missing transaction"))?;
        let response = self.api.add_declare_tx(tx.try_into()?).await.into_grpc_result()?;

        Ok(Response::new(AddDeclareTransactionResponse {
            transaction_hash: Some(response.transaction_hash.into()),
            class_hash: Some(response.class_hash.into()),
        }))
    }

    async fn add_deploy_account_transaction(
        &self,
        request: Request<AddDeployAccountTransactionRequest>,
    ) -> Result<Response<AddDeployAccountTransactionResponse>, Status> {
        let AddDeployAccountTransactionRequest { transaction } = request.into_inner();

        let tx = transaction.ok_or(Status::invalid_argument("missing transaction"))?;
        let response = self.api.add_deploy_account_tx(tx.try_into()?).await.into_grpc_result()?;

        Ok(Response::new(AddDeployAccountTransactionResponse {
            transaction_hash: Some(response.transaction_hash.into()),
            contract_address: Some(Felt::from(response.contract_address).into()),
        }))
    }
}

/////////////////////////////////////////////////////////////////////////
/// Starknet Trace Service Implementation
/////////////////////////////////////////////////////////////////////////

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

fn execution_result_to_string(exec: &katana_rpc_types::ExecutionResult) -> String {
    match exec {
        katana_rpc_types::ExecutionResult::Succeeded => "SUCCEEDED".to_string(),
        katana_rpc_types::ExecutionResult::Reverted { .. } => "REVERTED".to_string(),
    }
}
