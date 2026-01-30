//! Starknet read service handler implementation.

use katana_pool::TransactionPool;
use katana_primitives::Felt;
use katana_provider::{ProviderFactory, ProviderRO};
use katana_rpc_api::starknet::RPC_SPEC_VERSION;
use katana_rpc_server::starknet::{PendingBlockProvider, StarknetApi};
use tonic::{Request, Response, Status};

use crate::conversion::{block_id_from_proto, ProtoFeltVecExt};
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
/// This struct wraps `StarknetApi` from `katana-rpc-server` and implements the gRPC
/// service traits by delegating to the underlying API.
pub struct StarknetService<Pool, PP, PF>
where
    Pool: TransactionPool,
    PP: PendingBlockProvider,
    PF: ProviderFactory,
{
    inner: StarknetApi<Pool, PP, PF>,
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
        Self { inner: self.inner.clone() }
    }
}

impl<Pool, PP, PF> StarknetService<Pool, PP, PF>
where
    Pool: TransactionPool,
    PP: PendingBlockProvider,
    PF: ProviderFactory,
{
    /// Creates a new handler wrapping the given `StarknetApi`.
    pub fn new(inner: StarknetApi<Pool, PP, PF>) -> Self {
        Self { inner }
    }

    /// Returns a reference to the inner `StarknetApi`.
    #[allow(dead_code)]
    pub fn inner(&self) -> &StarknetApi<Pool, PP, PF> {
        &self.inner
    }
}

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
        let result = self.inner.block_with_tx_hashes(block_id).await.into_grpc_result()?;
        Ok(Response::new(result.into()))
    }

    async fn get_block_with_txs(
        &self,
        request: Request<GetBlockRequest>,
    ) -> Result<Response<GetBlockWithTxsResponse>, Status> {
        let block_id = block_id_from_proto(request.into_inner().block_id.as_ref())?;
        let result = self.inner.block_with_txs(block_id).await.into_grpc_result()?;
        Ok(Response::new(result.into()))
    }

    async fn get_block_with_receipts(
        &self,
        request: Request<GetBlockRequest>,
    ) -> Result<Response<GetBlockWithReceiptsResponse>, Status> {
        let block_id = block_id_from_proto(request.into_inner().block_id.as_ref())?;

        // Use the storage provider to build the block with receipts
        let result = self
            .inner
            .on_io_blocking_task(move |api| {
                use katana_primitives::block::BlockIdOrTag;
                use katana_provider::api::block::BlockIdReader;

                let provider = api.storage().provider();

                // Note: Pending block support requires access to private StarknetApi internals.
                // For now, pending blocks are not supported via gRPC.
                if BlockIdOrTag::PreConfirmed == block_id {
                    return Err(katana_rpc_api::error::starknet::StarknetApiError::BlockNotFound);
                }

                if let Some(num) = provider.convert_block_id(block_id)? {
                    let block = katana_rpc_types_builder::BlockBuilder::new(num.into(), provider)
                        .build_with_receipts()?
                        .map(katana_rpc_types::block::GetBlockWithReceiptsResponse::Block);

                    if let Some(block) = block {
                        Ok(block)
                    } else {
                        Err(katana_rpc_api::error::starknet::StarknetApiError::BlockNotFound)
                    }
                } else {
                    Err(katana_rpc_api::error::starknet::StarknetApiError::BlockNotFound)
                }
            })
            .await
            .into_grpc_result()?
            .map_err(crate::error::to_status)?;

        Ok(Response::new(result.into()))
    }

    async fn get_state_update(
        &self,
        request: Request<GetBlockRequest>,
    ) -> Result<Response<GetStateUpdateResponse>, Status> {
        let block_id = block_id_from_proto(request.into_inner().block_id.as_ref())?;
        let result = self.inner.state_update(block_id).await.into_grpc_result()?;
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
            .on_io_blocking_task(move |api| api.storage_at(contract_address.into(), key, block_id))
            .await
            .into_grpc_result()?
            .map_err(crate::error::to_status)?;

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

        let status = self
            .inner
            .on_io_blocking_task(move |api| {
                #[allow(unused_imports)]
                use katana_pool::TransactionPool;
                use katana_primitives::block::FinalityStatus;
                use katana_provider::api::transaction::{
                    ReceiptProvider, TransactionStatusProvider,
                };

                let provider = api.storage().provider();
                let status = provider.transaction_status(tx_hash)?;

                if let Some(status) = status {
                    let Some(receipt) = provider.receipt_by_hash(tx_hash)? else {
                        return Err(katana_rpc_api::error::starknet::StarknetApiError::unexpected(
                            "Transaction hash exist, but the receipt is missing",
                        ));
                    };

                    let exec_status = if let Some(reason) = receipt.revert_reason() {
                        katana_rpc_types::ExecutionResult::Reverted { reason: reason.to_string() }
                    } else {
                        katana_rpc_types::ExecutionResult::Succeeded
                    };

                    let status = match status {
                        FinalityStatus::AcceptedOnL1 => {
                            katana_rpc_types::TxStatus::AcceptedOnL1(exec_status)
                        }
                        FinalityStatus::AcceptedOnL2 => {
                            katana_rpc_types::TxStatus::AcceptedOnL2(exec_status)
                        }
                        FinalityStatus::PreConfirmed => {
                            katana_rpc_types::TxStatus::PreConfirmed(exec_status)
                        }
                    };

                    return Ok(status);
                }

                // Check if it's in the pool
                let _ = api
                    .pool()
                    .get(tx_hash)
                    .ok_or(katana_rpc_api::error::starknet::StarknetApiError::TxnHashNotFound)?;
                Ok(katana_rpc_types::TxStatus::Received)
            })
            .await
            .into_grpc_result()?
            .map_err(crate::error::to_status)?;

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

        let tx = self
            .inner
            .on_io_blocking_task(move |api| {
                use katana_provider::api::transaction::TransactionProvider;

                let tx = api
                    .storage()
                    .provider()
                    .transaction_by_hash(tx_hash)?
                    .map(katana_rpc_types::transaction::RpcTxWithHash::from);

                tx.ok_or(katana_rpc_api::error::starknet::StarknetApiError::TxnHashNotFound)
            })
            .await
            .into_grpc_result()?
            .map_err(crate::error::to_status)?;

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
            .on_io_blocking_task(move |api| {
                use katana_primitives::block::{BlockHashOrNumber, BlockIdOrTag};
                use katana_provider::api::block::BlockIdReader;
                use katana_provider::api::transaction::TransactionProvider;

                // Note: Pending block support requires access to private StarknetApi internals.
                if BlockIdOrTag::PreConfirmed == block_id {
                    return Err(katana_rpc_api::error::starknet::StarknetApiError::BlockNotFound);
                }

                let provider = api.storage().provider();
                let block_num = provider
                    .convert_block_id(block_id)?
                    .map(BlockHashOrNumber::Num)
                    .ok_or(katana_rpc_api::error::starknet::StarknetApiError::BlockNotFound)?;

                let tx = provider
                    .transaction_by_block_and_idx(block_num, index)?
                    .map(katana_rpc_types::transaction::RpcTxWithHash::from);

                tx.ok_or(katana_rpc_api::error::starknet::StarknetApiError::InvalidTxnIndex)
            })
            .await
            .into_grpc_result()?
            .map_err(crate::error::to_status)?;

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

        let receipt = self
            .inner
            .on_io_blocking_task(move |api| {
                let provider = api.storage().provider();
                let receipt =
                    katana_rpc_types_builder::ReceiptBuilder::new(tx_hash, provider).build()?;

                receipt.ok_or(katana_rpc_api::error::starknet::StarknetApiError::TxnHashNotFound)
            })
            .await
            .into_grpc_result()?
            .map_err(crate::error::to_status)?;

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

        let class = self.inner.class_at_hash(block_id, class_hash).await.into_grpc_result()?;

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
            .inner
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
        let count = self.inner.block_tx_count(block_id).await.into_grpc_result()?;
        Ok(Response::new(GetBlockTransactionCountResponse { count }))
    }

    async fn call(&self, request: Request<CallRequest>) -> Result<Response<CallResponse>, Status> {
        let req = request.into_inner();
        let _block_id = block_id_from_proto(req.block_id.as_ref())?;

        let function_call =
            req.request.ok_or_else(|| Status::invalid_argument("Missing request"))?;

        let _contract_address = Felt::try_from(
            function_call
                .contract_address
                .as_ref()
                .ok_or_else(|| Status::invalid_argument("Missing contract_address"))?,
        )?;

        let _entry_point_selector = Felt::try_from(
            function_call
                .entry_point_selector
                .as_ref()
                .ok_or_else(|| Status::invalid_argument("Missing entry_point_selector"))?,
        )?;

        let _calldata = function_call.calldata.to_felts()?;

        // Call requires access to the executor - not yet implemented
        Err(Status::unimplemented("call not yet implemented for gRPC"))
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
            .inner
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
        _request: Request<BlockHashAndNumberRequest>,
    ) -> Result<Response<BlockHashAndNumberResponse>, Status> {
        let result = self
            .inner
            .on_io_blocking_task(move |api| {
                use katana_provider::api::block::{BlockHashProvider, BlockNumberProvider};
                let provider = api.storage().provider();
                let hash = provider.latest_hash()?;
                let number = provider.latest_number()?;
                Ok(katana_rpc_types::block::BlockHashAndNumberResponse::new(hash, number))
            })
            .await
            .into_grpc_result()?
            .map_err(crate::error::to_status)?;

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
            self.inner.nonce_at(block_id, contract_address.into()).await.into_grpc_result()?;

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

fn execution_result_to_string(exec: &katana_rpc_types::ExecutionResult) -> String {
    match exec {
        katana_rpc_types::ExecutionResult::Succeeded => "SUCCEEDED".to_string(),
        katana_rpc_types::ExecutionResult::Reverted { .. } => "REVERTED".to_string(),
    }
}
