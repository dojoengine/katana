//! Starknet read service handler implementation.

use katana_primitives::Felt;
use tonic::{Request, Response, Status};

use crate::convert::{block_id_from_proto, FeltVecExt, ProtoFeltVecExt};
use crate::error::IntoGrpcResult;
use crate::protos::starknet::starknet_server::Starknet;
use crate::protos::starknet::{
    BlockHashAndNumberRequest, BlockHashAndNumberResponse, BlockNumberRequest, BlockNumberResponse,
    CallRequest, CallResponse, ChainIdRequest, ChainIdResponse, EstimateFeeRequest,
    EstimateFeeResponse, EstimateMessageFeeRequest, GetBlockRequest,
    GetBlockTransactionCountResponse, GetBlockWithReceiptsResponse, GetBlockWithTxHashesResponse,
    GetBlockWithTxsResponse, GetClassAtRequest, GetClassAtResponse, GetClassHashAtRequest,
    GetClassHashAtResponse, GetClassRequest, GetClassResponse, GetCompiledCasmRequest,
    GetCompiledCasmResponse, GetEventsRequest, GetEventsResponse, GetNonceRequest,
    GetNonceResponse, GetStateUpdateResponse, GetStorageAtRequest, GetStorageAtResponse,
    GetStorageProofRequest, GetStorageProofResponse, GetTransactionByBlockIdAndIndexRequest,
    GetTransactionByBlockIdAndIndexResponse, GetTransactionByHashRequest,
    GetTransactionByHashResponse, GetTransactionReceiptRequest, GetTransactionReceiptResponse,
    GetTransactionStatusRequest, GetTransactionStatusResponse, SpecVersionRequest,
    SpecVersionResponse, SyncingRequest, SyncingResponse,
};
use crate::protos::types::{Transaction as ProtoTx, TransactionReceipt as ProtoTransactionReceipt};

/// The main handler for Starknet gRPC services.
///
/// This struct wraps an inner handler type that provides the actual business logic.
/// The inner type is expected to be or behave like `StarknetApi` from `katana-rpc-server`.
#[derive(Debug, Clone)]
pub struct StarknetHandler<T> {
    inner: T,
}

impl<T> StarknetHandler<T> {
    /// Creates a new handler with the given inner implementation.
    pub fn new(inner: T) -> Self {
        Self { inner }
    }

    /// Returns a reference to the inner handler.
    pub fn inner(&self) -> &T {
        &self.inner
    }
}

/// Trait for the inner handler that provides Starknet API functionality.
///
/// This trait abstracts the `StarknetApi` from `katana-rpc-server` to allow
/// for easier testing and flexibility.
#[tonic::async_trait]
pub trait StarknetApiProvider: Clone + Send + Sync + 'static {
    /// Returns the spec version.
    fn spec_version(&self) -> &'static str;

    /// Returns the chain ID.
    fn chain_id(&self) -> katana_primitives::Felt;

    /// Returns the latest block number.
    async fn block_number(
        &self,
    ) -> Result<
        katana_rpc_types::block::BlockNumberResponse,
        katana_rpc_api::error::starknet::StarknetApiError,
    >;

    /// Returns the latest block hash and number.
    async fn block_hash_and_number(
        &self,
    ) -> Result<
        katana_rpc_types::block::BlockHashAndNumberResponse,
        katana_rpc_api::error::starknet::StarknetApiError,
    >;

    /// Returns a block with transaction hashes.
    async fn get_block_with_tx_hashes(
        &self,
        block_id: katana_primitives::block::BlockIdOrTag,
    ) -> Result<
        katana_rpc_types::block::GetBlockWithTxHashesResponse,
        katana_rpc_api::error::starknet::StarknetApiError,
    >;

    /// Returns a block with full transactions.
    async fn get_block_with_txs(
        &self,
        block_id: katana_primitives::block::BlockIdOrTag,
    ) -> Result<
        katana_rpc_types::block::MaybePreConfirmedBlock,
        katana_rpc_api::error::starknet::StarknetApiError,
    >;

    /// Returns a block with transactions and receipts.
    async fn get_block_with_receipts(
        &self,
        block_id: katana_primitives::block::BlockIdOrTag,
    ) -> Result<
        katana_rpc_types::block::GetBlockWithReceiptsResponse,
        katana_rpc_api::error::starknet::StarknetApiError,
    >;

    /// Returns the state update for a block.
    async fn get_state_update(
        &self,
        block_id: katana_primitives::block::BlockIdOrTag,
    ) -> Result<
        katana_rpc_types::state_update::StateUpdate,
        katana_rpc_api::error::starknet::StarknetApiError,
    >;

    /// Returns the storage value at a given address and key.
    async fn get_storage_at(
        &self,
        contract_address: katana_primitives::ContractAddress,
        key: katana_primitives::Felt,
        block_id: katana_primitives::block::BlockIdOrTag,
    ) -> Result<katana_primitives::Felt, katana_rpc_api::error::starknet::StarknetApiError>;

    /// Returns the transaction status.
    async fn get_transaction_status(
        &self,
        transaction_hash: katana_primitives::Felt,
    ) -> Result<katana_rpc_types::TxStatus, katana_rpc_api::error::starknet::StarknetApiError>;

    /// Returns a transaction by hash.
    async fn get_transaction_by_hash(
        &self,
        transaction_hash: katana_primitives::Felt,
    ) -> Result<
        katana_rpc_types::transaction::RpcTxWithHash,
        katana_rpc_api::error::starknet::StarknetApiError,
    >;

    /// Returns a transaction by block ID and index.
    async fn get_transaction_by_block_id_and_index(
        &self,
        block_id: katana_primitives::block::BlockIdOrTag,
        index: u64,
    ) -> Result<
        katana_rpc_types::transaction::RpcTxWithHash,
        katana_rpc_api::error::starknet::StarknetApiError,
    >;

    /// Returns a transaction receipt.
    async fn get_transaction_receipt(
        &self,
        transaction_hash: katana_primitives::Felt,
    ) -> Result<
        katana_rpc_types::receipt::TxReceiptWithBlockInfo,
        katana_rpc_api::error::starknet::StarknetApiError,
    >;

    /// Returns a contract class by hash.
    async fn get_class(
        &self,
        block_id: katana_primitives::block::BlockIdOrTag,
        class_hash: katana_primitives::Felt,
    ) -> Result<katana_rpc_types::class::Class, katana_rpc_api::error::starknet::StarknetApiError>;

    /// Returns the class hash at a given address.
    async fn get_class_hash_at(
        &self,
        block_id: katana_primitives::block::BlockIdOrTag,
        contract_address: katana_primitives::ContractAddress,
    ) -> Result<katana_primitives::Felt, katana_rpc_api::error::starknet::StarknetApiError>;

    /// Returns the contract class at a given address.
    async fn get_class_at(
        &self,
        block_id: katana_primitives::block::BlockIdOrTag,
        contract_address: katana_primitives::ContractAddress,
    ) -> Result<katana_rpc_types::class::Class, katana_rpc_api::error::starknet::StarknetApiError>;

    /// Returns the transaction count for a block.
    async fn get_block_transaction_count(
        &self,
        block_id: katana_primitives::block::BlockIdOrTag,
    ) -> Result<u64, katana_rpc_api::error::starknet::StarknetApiError>;

    /// Calls a contract function.
    async fn call(
        &self,
        request: katana_rpc_types::FunctionCall,
        block_id: katana_primitives::block::BlockIdOrTag,
    ) -> Result<katana_rpc_types::CallResponse, katana_rpc_api::error::starknet::StarknetApiError>;

    /// Estimates the fee for transactions.
    async fn estimate_fee(
        &self,
        request: Vec<katana_rpc_types::broadcasted::BroadcastedTx>,
        simulation_flags: Vec<katana_rpc_types::EstimateFeeSimulationFlag>,
        block_id: katana_primitives::block::BlockIdOrTag,
    ) -> Result<Vec<katana_rpc_types::FeeEstimate>, katana_rpc_api::error::starknet::StarknetApiError>;

    /// Estimates the fee for an L1 message.
    async fn estimate_message_fee(
        &self,
        message: katana_rpc_types::message::MsgFromL1,
        block_id: katana_primitives::block::BlockIdOrTag,
    ) -> Result<katana_rpc_types::FeeEstimate, katana_rpc_api::error::starknet::StarknetApiError>;

    /// Returns events matching the filter.
    async fn get_events(
        &self,
        filter: katana_rpc_types::event::EventFilterWithPage,
    ) -> Result<
        katana_rpc_types::event::GetEventsResponse,
        katana_rpc_api::error::starknet::StarknetApiError,
    >;

    /// Returns the nonce at a given address.
    async fn get_nonce(
        &self,
        block_id: katana_primitives::block::BlockIdOrTag,
        contract_address: katana_primitives::ContractAddress,
    ) -> Result<katana_primitives::Felt, katana_rpc_api::error::starknet::StarknetApiError>;

    /// Returns the sync status.
    fn syncing(&self) -> bool;
}

#[tonic::async_trait]
impl<T: StarknetApiProvider> Starknet for StarknetHandler<T> {
    async fn spec_version(
        &self,
        _request: Request<SpecVersionRequest>,
    ) -> Result<Response<SpecVersionResponse>, Status> {
        Ok(Response::new(SpecVersionResponse { version: self.inner.spec_version().to_string() }))
    }

    async fn get_block_with_tx_hashes(
        &self,
        request: Request<GetBlockRequest>,
    ) -> Result<Response<GetBlockWithTxHashesResponse>, Status> {
        let block_id = block_id_from_proto(request.into_inner().block_id.as_ref())?;
        let result = self.inner.get_block_with_tx_hashes(block_id).await.into_grpc_result()?;
        Ok(Response::new(result.into()))
    }

    async fn get_block_with_txs(
        &self,
        request: Request<GetBlockRequest>,
    ) -> Result<Response<GetBlockWithTxsResponse>, Status> {
        let block_id = block_id_from_proto(request.into_inner().block_id.as_ref())?;
        let result = self.inner.get_block_with_txs(block_id).await.into_grpc_result()?;
        Ok(Response::new(result.into()))
    }

    async fn get_block_with_receipts(
        &self,
        request: Request<GetBlockRequest>,
    ) -> Result<Response<GetBlockWithReceiptsResponse>, Status> {
        let block_id = block_id_from_proto(request.into_inner().block_id.as_ref())?;
        let result = self.inner.get_block_with_receipts(block_id).await.into_grpc_result()?;
        Ok(Response::new(result.into()))
    }

    async fn get_state_update(
        &self,
        request: Request<GetBlockRequest>,
    ) -> Result<Response<GetStateUpdateResponse>, Status> {
        let block_id = block_id_from_proto(request.into_inner().block_id.as_ref())?;
        let result = self.inner.get_state_update(block_id).await.into_grpc_result()?;
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

        let result = self
            .inner
            .get_storage_at(contract_address.into(), key, block_id)
            .await
            .into_grpc_result()?;

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

        let status = self.inner.get_transaction_status(tx_hash).await.into_grpc_result()?;

        let (finality_status, execution_status) = match status {
            katana_rpc_types::TxStatus::Received => ("RECEIVED".to_string(), String::new()),
            katana_rpc_types::TxStatus::Rejected => ("REJECTED".to_string(), String::new()),
            katana_rpc_types::TxStatus::PreConfirmed(exec) => {
                ("ACCEPTED_ON_L2".to_string(), execution_result_to_string(&exec))
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

        let tx = self.inner.get_transaction_by_hash(tx_hash).await.into_grpc_result()?;

        Ok(Response::new(GetTransactionByHashResponse { transaction: Some(ProtoTx::from(tx)) }))
    }

    async fn get_transaction_by_block_id_and_index(
        &self,
        request: Request<GetTransactionByBlockIdAndIndexRequest>,
    ) -> Result<Response<GetTransactionByBlockIdAndIndexResponse>, Status> {
        let req = request.into_inner();
        let block_id = block_id_from_proto(req.block_id.as_ref())?;
        let index = req.index;

        let tx = self
            .inner
            .get_transaction_by_block_id_and_index(block_id, index)
            .await
            .into_grpc_result()?;

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

        let receipt = self.inner.get_transaction_receipt(tx_hash).await.into_grpc_result()?;

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

        let class = self.inner.get_class(block_id, class_hash).await.into_grpc_result()?;

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
            .inner
            .get_class_hash_at(block_id, contract_address.into())
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

        let class =
            self.inner.get_class_at(block_id, contract_address.into()).await.into_grpc_result()?;

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
        let count = self.inner.get_block_transaction_count(block_id).await.into_grpc_result()?;
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

        let call_request = katana_rpc_types::FunctionCall {
            contract_address: contract_address.into(),
            entry_point_selector,
            calldata,
        };

        let result = self.inner.call(call_request, block_id).await.into_grpc_result()?;

        Ok(Response::new(CallResponse { result: result.result.to_proto_felts() }))
    }

    async fn estimate_fee(
        &self,
        request: Request<EstimateFeeRequest>,
    ) -> Result<Response<EstimateFeeResponse>, Status> {
        // Simplified implementation - would need full transaction conversion
        let _req = request.into_inner();
        Err(Status::unimplemented("estimate_fee requires full transaction conversion"))
    }

    async fn estimate_message_fee(
        &self,
        request: Request<EstimateMessageFeeRequest>,
    ) -> Result<Response<EstimateFeeResponse>, Status> {
        // Simplified implementation - would need full message conversion
        let _req = request.into_inner();
        Err(Status::unimplemented("estimate_message_fee requires full message conversion"))
    }

    async fn block_number(
        &self,
        _request: Request<BlockNumberRequest>,
    ) -> Result<Response<BlockNumberResponse>, Status> {
        let result = self.inner.block_number().await.into_grpc_result()?;
        Ok(Response::new(BlockNumberResponse { block_number: result.block_number }))
    }

    async fn block_hash_and_number(
        &self,
        _request: Request<BlockHashAndNumberRequest>,
    ) -> Result<Response<BlockHashAndNumberResponse>, Status> {
        let result = self.inner.block_hash_and_number().await.into_grpc_result()?;
        Ok(Response::new(BlockHashAndNumberResponse {
            block_hash: Some(result.block_hash.into()),
            block_number: result.block_number,
        }))
    }

    async fn chain_id(
        &self,
        _request: Request<ChainIdRequest>,
    ) -> Result<Response<ChainIdResponse>, Status> {
        let chain_id = self.inner.chain_id();
        Ok(Response::new(ChainIdResponse { chain_id: format!("{:#x}", chain_id) }))
    }

    async fn syncing(
        &self,
        _request: Request<SyncingRequest>,
    ) -> Result<Response<SyncingResponse>, Status> {
        let is_syncing = self.inner.syncing();
        Ok(Response::new(SyncingResponse {
            result: Some(crate::protos::starknet::syncing_response::Result::NotSyncing(
                !is_syncing,
            )),
        }))
    }

    async fn get_events(
        &self,
        request: Request<GetEventsRequest>,
    ) -> Result<Response<GetEventsResponse>, Status> {
        // Simplified implementation - would need full filter conversion
        let _req = request.into_inner();
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
            self.inner.get_nonce(block_id, contract_address.into()).await.into_grpc_result()?;

        Ok(Response::new(GetNonceResponse { nonce: Some(nonce.into()) }))
    }

    async fn get_compiled_casm(
        &self,
        _request: Request<GetCompiledCasmRequest>,
    ) -> Result<Response<GetCompiledCasmResponse>, Status> {
        // Simplified implementation - would need CASM conversion
        Err(Status::unimplemented("get_compiled_casm requires CASM conversion"))
    }

    async fn get_storage_proof(
        &self,
        _request: Request<GetStorageProofRequest>,
    ) -> Result<Response<GetStorageProofResponse>, Status> {
        // Simplified implementation - would need proof conversion
        Err(Status::unimplemented("get_storage_proof requires proof conversion"))
    }
}

fn execution_result_to_string(exec: &katana_rpc_types::ExecutionResult) -> String {
    match exec {
        katana_rpc_types::ExecutionResult::Succeeded => "SUCCEEDED".to_string(),
        katana_rpc_types::ExecutionResult::Reverted { .. } => "REVERTED".to_string(),
    }
}
