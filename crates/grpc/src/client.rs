//! gRPC client implementation.

use std::time::Duration;

use tonic::transport::{Channel, Endpoint, Uri};
use tonic::{Request, Response, Status};

use crate::protos::starknet::starknet_client::StarknetClient;
use crate::protos::starknet::starknet_trace_client::StarknetTraceClient;
use crate::protos::starknet::starknet_write_client::StarknetWriteClient;
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

/// The default request timeout.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(20);

/// The default connection timeout.
pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// Error type for gRPC client operations.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Transport error from tonic.
    #[error(transparent)]
    Transport(#[from] tonic::transport::Error),

    /// Invalid URI.
    #[error("Invalid URI: {0}")]
    InvalidUri(#[from] http::uri::InvalidUri),
}

/// Builder for creating a gRPC client.
///
/// # Example
///
/// ```ignore
/// use std::time::Duration;
/// use katana_grpc::GrpcClient;
///
/// let client = GrpcClient::builder("http://localhost:5051")
///     .timeout(Duration::from_secs(30))
///     .connect_timeout(Duration::from_secs(10))
///     .connect()
///     .await?;
/// ```
#[derive(Debug, Clone)]
pub struct GrpcClientBuilder {
    endpoint: String,
    timeout: Duration,
    connect_timeout: Duration,
}

impl GrpcClientBuilder {
    /// Creates a new client builder for the specified endpoint.
    fn new(endpoint: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
            timeout: DEFAULT_TIMEOUT,
            connect_timeout: DEFAULT_CONNECT_TIMEOUT,
        }
    }

    /// Sets the request timeout. Default is 20 seconds.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Sets the connection timeout. Default is 5 seconds.
    pub fn connect_timeout(mut self, timeout: Duration) -> Self {
        self.connect_timeout = timeout;
        self
    }

    /// Connects to the gRPC server and returns a client.
    pub async fn connect(self) -> Result<GrpcClient, Error> {
        let uri: Uri = self.endpoint.parse()?;

        let channel = Endpoint::from(uri)
            .timeout(self.timeout)
            .connect_timeout(self.connect_timeout)
            .connect()
            .await?;

        Ok(GrpcClient::from_channel(channel))
    }
}

/// A client for interacting with Katana's gRPC endpoints.
#[derive(Debug, Clone)]
pub struct GrpcClient {
    starknet: StarknetClient<Channel>,
    starknet_write: StarknetWriteClient<Channel>,
    starknet_trace: StarknetTraceClient<Channel>,
}

impl GrpcClient {
    /// Creates a new client builder for the specified endpoint.
    ///
    /// # Arguments
    ///
    /// * `endpoint` - The URI of the gRPC server (e.g., "http://localhost:5051")
    pub fn builder(endpoint: impl Into<String>) -> GrpcClientBuilder {
        GrpcClientBuilder::new(endpoint)
    }

    /// Connects to the gRPC server with default configuration.
    ///
    /// This is a convenience method equivalent to `GrpcClient::builder(endpoint).connect()`.
    ///
    /// # Arguments
    ///
    /// * `endpoint` - The URI of the gRPC server (e.g., "http://localhost:5051")
    pub async fn connect(endpoint: impl Into<String>) -> Result<Self, Error> {
        Self::builder(endpoint).connect().await
    }

    /// Creates a new gRPC client from an existing channel.
    ///
    /// This is useful when you want to share a channel between multiple clients
    /// or have custom channel configuration.
    pub fn from_channel(channel: Channel) -> Self {
        Self {
            starknet: StarknetClient::new(channel.clone()),
            starknet_write: StarknetWriteClient::new(channel.clone()),
            starknet_trace: StarknetTraceClient::new(channel),
        }
    }

    // ============================================================
    // Read Methods (Starknet service)
    // ============================================================

    /// Returns the version of the Starknet JSON-RPC specification being used.
    pub async fn spec_version(
        &mut self,
        request: impl Into<Request<SpecVersionRequest>>,
    ) -> Result<Response<SpecVersionResponse>, Status> {
        self.starknet.spec_version(request.into()).await
    }

    /// Get block information with transaction hashes given the block id.
    pub async fn get_block_with_tx_hashes(
        &mut self,
        request: impl Into<Request<GetBlockRequest>>,
    ) -> Result<Response<GetBlockWithTxHashesResponse>, Status> {
        self.starknet.get_block_with_tx_hashes(request.into()).await
    }

    /// Get block information with full transactions given the block id.
    pub async fn get_block_with_txs(
        &mut self,
        request: impl Into<Request<GetBlockRequest>>,
    ) -> Result<Response<GetBlockWithTxsResponse>, Status> {
        self.starknet.get_block_with_txs(request.into()).await
    }

    /// Get block information with full transactions and receipts given the block id.
    pub async fn get_block_with_receipts(
        &mut self,
        request: impl Into<Request<GetBlockRequest>>,
    ) -> Result<Response<GetBlockWithReceiptsResponse>, Status> {
        self.starknet.get_block_with_receipts(request.into()).await
    }

    /// Get the information about the result of executing the requested block.
    pub async fn get_state_update(
        &mut self,
        request: impl Into<Request<GetBlockRequest>>,
    ) -> Result<Response<GetStateUpdateResponse>, Status> {
        self.starknet.get_state_update(request.into()).await
    }

    /// Get the value of the storage at the given address and key.
    pub async fn get_storage_at(
        &mut self,
        request: impl Into<Request<GetStorageAtRequest>>,
    ) -> Result<Response<GetStorageAtResponse>, Status> {
        self.starknet.get_storage_at(request.into()).await
    }

    /// Gets the transaction status.
    pub async fn get_transaction_status(
        &mut self,
        request: impl Into<Request<GetTransactionStatusRequest>>,
    ) -> Result<Response<GetTransactionStatusResponse>, Status> {
        self.starknet.get_transaction_status(request.into()).await
    }

    /// Get the details and status of a submitted transaction.
    pub async fn get_transaction_by_hash(
        &mut self,
        request: impl Into<Request<GetTransactionByHashRequest>>,
    ) -> Result<Response<GetTransactionByHashResponse>, Status> {
        self.starknet.get_transaction_by_hash(request.into()).await
    }

    /// Get the details of a transaction by a given block id and index.
    pub async fn get_transaction_by_block_id_and_index(
        &mut self,
        request: impl Into<Request<GetTransactionByBlockIdAndIndexRequest>>,
    ) -> Result<Response<GetTransactionByBlockIdAndIndexResponse>, Status> {
        self.starknet.get_transaction_by_block_id_and_index(request.into()).await
    }

    /// Get the transaction receipt by the transaction hash.
    pub async fn get_transaction_receipt(
        &mut self,
        request: impl Into<Request<GetTransactionReceiptRequest>>,
    ) -> Result<Response<GetTransactionReceiptResponse>, Status> {
        self.starknet.get_transaction_receipt(request.into()).await
    }

    /// Get the contract class definition in the given block associated with the given hash.
    pub async fn get_class(
        &mut self,
        request: impl Into<Request<GetClassRequest>>,
    ) -> Result<Response<GetClassResponse>, Status> {
        self.starknet.get_class(request.into()).await
    }

    /// Get the contract class hash in the given block for the contract deployed at the given
    /// address.
    pub async fn get_class_hash_at(
        &mut self,
        request: impl Into<Request<GetClassHashAtRequest>>,
    ) -> Result<Response<GetClassHashAtResponse>, Status> {
        self.starknet.get_class_hash_at(request.into()).await
    }

    /// Get the contract class definition in the given block at the given address.
    pub async fn get_class_at(
        &mut self,
        request: impl Into<Request<GetClassAtRequest>>,
    ) -> Result<Response<GetClassAtResponse>, Status> {
        self.starknet.get_class_at(request.into()).await
    }

    /// Get the number of transactions in a block given a block id.
    pub async fn get_block_transaction_count(
        &mut self,
        request: impl Into<Request<GetBlockRequest>>,
    ) -> Result<Response<GetBlockTransactionCountResponse>, Status> {
        self.starknet.get_block_transaction_count(request.into()).await
    }

    /// Call a starknet function without creating a Starknet transaction.
    pub async fn call(
        &mut self,
        request: impl Into<Request<CallRequest>>,
    ) -> Result<Response<CallResponse>, Status> {
        self.starknet.call(request.into()).await
    }

    /// Estimate the fee for Starknet transactions.
    pub async fn estimate_fee(
        &mut self,
        request: impl Into<Request<EstimateFeeRequest>>,
    ) -> Result<Response<EstimateFeeResponse>, Status> {
        self.starknet.estimate_fee(request.into()).await
    }

    /// Estimate the L2 fee of a message sent on L1.
    pub async fn estimate_message_fee(
        &mut self,
        request: impl Into<Request<EstimateMessageFeeRequest>>,
    ) -> Result<Response<EstimateFeeResponse>, Status> {
        self.starknet.estimate_message_fee(request.into()).await
    }

    /// Get the most recent accepted block number.
    pub async fn block_number(
        &mut self,
        request: impl Into<Request<BlockNumberRequest>>,
    ) -> Result<Response<BlockNumberResponse>, Status> {
        self.starknet.block_number(request.into()).await
    }

    /// Get the most recent accepted block hash and number.
    pub async fn block_hash_and_number(
        &mut self,
        request: impl Into<Request<BlockHashAndNumberRequest>>,
    ) -> Result<Response<BlockHashAndNumberResponse>, Status> {
        self.starknet.block_hash_and_number(request.into()).await
    }

    /// Return the currently configured Starknet chain id.
    pub async fn chain_id(
        &mut self,
        request: impl Into<Request<ChainIdRequest>>,
    ) -> Result<Response<ChainIdResponse>, Status> {
        self.starknet.chain_id(request.into()).await
    }

    /// Returns an object about the sync status, or false if the node is not synching.
    pub async fn syncing(
        &mut self,
        request: impl Into<Request<SyncingRequest>>,
    ) -> Result<Response<SyncingResponse>, Status> {
        self.starknet.syncing(request.into()).await
    }

    /// Returns all events matching the given filter.
    pub async fn get_events(
        &mut self,
        request: impl Into<Request<GetEventsRequest>>,
    ) -> Result<Response<GetEventsResponse>, Status> {
        self.starknet.get_events(request.into()).await
    }

    /// Get the nonce associated with the given address in the given block.
    pub async fn get_nonce(
        &mut self,
        request: impl Into<Request<GetNonceRequest>>,
    ) -> Result<Response<GetNonceResponse>, Status> {
        self.starknet.get_nonce(request.into()).await
    }

    /// Get the compiled CASM for a given class hash.
    pub async fn get_compiled_casm(
        &mut self,
        request: impl Into<Request<GetCompiledCasmRequest>>,
    ) -> Result<Response<GetCompiledCasmResponse>, Status> {
        self.starknet.get_compiled_casm(request.into()).await
    }

    /// Get Merkle paths in the state tries for a set of classes, contracts, and storage keys.
    pub async fn get_storage_proof(
        &mut self,
        request: impl Into<Request<GetStorageProofRequest>>,
    ) -> Result<Response<GetStorageProofResponse>, Status> {
        self.starknet.get_storage_proof(request.into()).await
    }

    // ============================================================
    // Write Methods (StarknetWrite service)
    // ============================================================

    /// Submit a new invoke transaction.
    pub async fn add_invoke_transaction(
        &mut self,
        request: impl Into<Request<AddInvokeTransactionRequest>>,
    ) -> Result<Response<AddInvokeTransactionResponse>, Status> {
        self.starknet_write.add_invoke_transaction(request.into()).await
    }

    /// Submit a new declare transaction.
    pub async fn add_declare_transaction(
        &mut self,
        request: impl Into<Request<AddDeclareTransactionRequest>>,
    ) -> Result<Response<AddDeclareTransactionResponse>, Status> {
        self.starknet_write.add_declare_transaction(request.into()).await
    }

    /// Submit a new deploy account transaction.
    pub async fn add_deploy_account_transaction(
        &mut self,
        request: impl Into<Request<AddDeployAccountTransactionRequest>>,
    ) -> Result<Response<AddDeployAccountTransactionResponse>, Status> {
        self.starknet_write.add_deploy_account_transaction(request.into()).await
    }

    // ============================================================
    // Trace Methods (StarknetTrace service)
    // ============================================================

    /// Get the trace for a specific transaction.
    pub async fn trace_transaction(
        &mut self,
        request: impl Into<Request<TraceTransactionRequest>>,
    ) -> Result<Response<TraceTransactionResponse>, Status> {
        self.starknet_trace.trace_transaction(request.into()).await
    }

    /// Simulate a list of transactions and return their execution traces and fee estimations.
    pub async fn simulate_transactions(
        &mut self,
        request: impl Into<Request<SimulateTransactionsRequest>>,
    ) -> Result<Response<SimulateTransactionsResponse>, Status> {
        self.starknet_trace.simulate_transactions(request.into()).await
    }

    /// Get the traces for all transactions in a block.
    pub async fn trace_block_transactions(
        &mut self,
        request: impl Into<Request<TraceBlockTransactionsRequest>>,
    ) -> Result<Response<TraceBlockTransactionsResponse>, Status> {
        self.starknet_trace.trace_block_transactions(request.into()).await
    }
}
