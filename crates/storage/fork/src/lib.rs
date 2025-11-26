use std::collections::hash_map::Entry;
use std::collections::{HashMap, VecDeque};
use std::fmt::Debug;
use std::io;
use std::pin::Pin;
#[cfg(test)]
use std::sync::atomic::Ordering;
use std::sync::mpsc::{
    channel as oneshot, Receiver as OneshotReceiver, RecvError, Sender as OneshotSender,
};
use std::sync::Arc;
use std::task::{Context, Poll};

use anyhow::anyhow;
use futures::channel::mpsc::{channel as async_channel, Receiver, SendError, Sender};
use futures::future::BoxFuture;
use futures::stream::Stream;
use futures::{Future, FutureExt};
use katana_metrics::metrics::Gauge;
use katana_metrics::{metrics, Metrics};
use katana_primitives::block::{BlockHashOrNumber, BlockIdOrTag, BlockNumber};
use katana_primitives::class::{
    ClassHash, CompiledClassHash, ComputeClassHashError, ContractClass,
    ContractClassCompilationError,
};
use katana_primitives::contract::{ContractAddress, Nonce, StorageKey, StorageValue};
use katana_primitives::transaction::TxHash;
use katana_primitives::Felt;
use katana_rpc_client::starknet::{
    Client as StarknetClient, Error as StarknetClientError, StarknetApiError,
};
use katana_rpc_types::class::Class;
use katana_rpc_types::{
    ConfirmedBlockIdOrTag, ContractStorageKeys, GetBlockWithReceiptsResponse, GetStorageProofResponse, StateUpdate, TraceBlockTransactionsResponse, TxReceiptWithBlockInfo
};
use parking_lot::Mutex;
use tracing::{error, trace};

/// Default maximum number of concurrent requests that can be processed.
/// This is a reasonable default to prevent overwhelming the remote RPC provider.
const DEFAULT_WORKER_MAX_CONCURRENT_REQUESTS: usize = 50;

type BackendResult<T> = Result<T, BackendError>;

pub struct Backend {
    sender: Mutex<Sender<BackendRequest>>,
    metrics: BackendMetrics,
    #[cfg(test)]
    stats: BackendStats,
}

impl std::fmt::Debug for Backend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        #[cfg(not(test))]
        {
            f.debug_struct("BackendClient")
                .field("sender", &self.sender)
                .field("metrics", &"<metrics>")
                .finish()
        }

        #[cfg(test)]
        {
            f.debug_struct("BackendClient")
                .field("sender", &self.sender)
                .field("metrics", &"<metrics>")
                .field("stats", &self.stats)
                .finish()
        }
    }
}

impl Clone for Backend {
    fn clone(&self) -> Self {
        Self {
            sender: Mutex::new(self.sender.lock().clone()),
            metrics: self.metrics.clone(),
            #[cfg(test)]
            stats: self.stats.clone(),
        }
    }
}

/////////////////////////////////////////////////////////////////
// Backend implementation
/////////////////////////////////////////////////////////////////

impl Backend {
    pub fn new(provider: StarknetClient) -> Result<Self, BackendError> {
        Self::new_with_config(provider, DEFAULT_WORKER_MAX_CONCURRENT_REQUESTS)
    }

    pub fn new_with_config(
        provider: StarknetClient,
        max_concurrent_requests: usize,
    ) -> Result<Self, BackendError> {
        let (request_tx, request_rx) = async_channel(100);
        let metrics = BackendMetrics::default();

        #[cfg(test)]
        let stats = BackendStats::default();

        let worker = BackendWorker {
            incoming: request_rx,
            metrics: metrics.clone(),
            starknet_client: Arc::new(provider),
            pending_requests: Vec::new(),
            request_dedup_map: HashMap::new(),
            queued_requests: VecDeque::new(),
            max_concurrent_requests,
            #[cfg(test)]
            stats: stats.clone(),
        };

        std::thread::Builder::new()
            .name("forking-backend-worker".into())
            .spawn(move || {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("failed to create tokio runtime")
                    .block_on(worker);
            })
            .map_err(|e| BackendError::BackendThreadInit(Arc::new(e)))?;

        trace!("Forking backend worker started.");

        Ok(Backend {
            metrics,
            sender: Mutex::new(request_tx),
            #[cfg(test)]
            stats,
        })
    }

    pub fn get_block(
        &self,
        block_id: BlockHashOrNumber,
    ) -> Result<Option<GetBlockWithReceiptsResponse>, BackendClientError> {
        trace!(%block_id, "Requesting block.");
        let (req, rx) = BackendRequest::block(block_id);
        self.request(req)?;
        match rx.recv()? {
            BackendResponse::Block(res) => handle_not_found_err(res),
            response => Err(BackendClientError::UnexpectedResponse(anyhow!("{response:?}"))),
        }
    }

    pub fn get_state_update(
        &self,
        block_id: BlockHashOrNumber,
    ) -> Result<Option<StateUpdate>, BackendClientError> {
        trace!(%block_id, "Requesting state update.");
        let (req, rx) = BackendRequest::state_update(block_id);
        self.request(req)?;
        match rx.recv()? {
            BackendResponse::StateUpdate(res) => handle_not_found_err(res),
            response => Err(BackendClientError::UnexpectedResponse(anyhow!("{response:?}"))),
        }
    }

    pub fn get_receipt(
        &self,
        tx_hash: TxHash,
    ) -> Result<Option<TxReceiptWithBlockInfo>, BackendClientError> {
        trace!(%tx_hash, "Requesting block.");
        let (req, rx) = BackendRequest::receipt(tx_hash);
        self.request(req)?;
        match rx.recv()? {
            BackendResponse::Receipt(res) => handle_not_found_err(res),
            response => Err(BackendClientError::UnexpectedResponse(anyhow!("{response:?}"))),
        }
    }

    pub fn get_nonce(
        &self,
        address: ContractAddress,
        block_id: BlockNumber,
    ) -> Result<Option<Nonce>, BackendClientError> {
        trace!(%address, "Requesting contract nonce.");
        let (req, rx) = BackendRequest::nonce(address, block_id);
        self.request(req)?;
        match rx.recv()? {
            BackendResponse::Nonce(res) => handle_not_found_err(res),
            response => Err(BackendClientError::UnexpectedResponse(anyhow!("{response:?}"))),
        }
    }

    pub fn get_storage(
        &self,
        address: ContractAddress,
        key: StorageKey,
        block_id: BlockNumber,
    ) -> Result<Option<StorageValue>, BackendClientError> {
        trace!(%address, key = %format!("{key:#x}"), "Requesting contract storage.");
        let (req, rx) = BackendRequest::storage(address, key, block_id);
        self.request(req)?;
        match rx.recv()? {
            BackendResponse::Storage(res) => handle_not_found_err(res),
            response => Err(BackendClientError::UnexpectedResponse(anyhow!("{response:?}"))),
        }
    }

    pub fn get_class_hash_at(
        &self,
        address: ContractAddress,
        block_id: BlockNumber,
    ) -> Result<Option<ClassHash>, BackendClientError> {
        trace!(%address, "Requesting contract class hash.");
        let (req, rx) = BackendRequest::class_hash(address, block_id);
        self.request(req)?;
        match rx.recv()? {
            BackendResponse::ClassHashAt(res) => handle_not_found_err(res),
            response => Err(BackendClientError::UnexpectedResponse(anyhow!("{response:?}"))),
        }
    }

    pub fn get_class_at(
        &self,
        class_hash: ClassHash,
        block_id: BlockNumber,
    ) -> Result<Option<ContractClass>, BackendClientError> {
        trace!(class_hash = %format!("{class_hash:#x}"), "Requesting class.");
        let (req, rx) = BackendRequest::class(class_hash, block_id);
        self.request(req)?;
        match rx.recv()? {
            BackendResponse::ClassAt(res) => {
                if let Some(class) = handle_not_found_err(res)? {
                    Ok(Some(ContractClass::try_from(class)?))
                } else {
                    Ok(None)
                }
            }
            response => Err(BackendClientError::UnexpectedResponse(anyhow!("{response:?}"))),
        }
    }

    pub fn get_compiled_class_hash(
        &self,
        class_hash: ClassHash,
        block_id: BlockNumber,
    ) -> Result<Option<CompiledClassHash>, BackendClientError> {
        trace!(class_hash = %format!("{class_hash:#x}"), "Requesting compiled class hash.");
        if let Some(class) = self.get_class_at(class_hash, block_id)? {
            let class = class.compile()?;
            Ok(Some(class.class_hash()?))
        } else {
            Ok(None)
        }
    }

    pub fn get_classes_proofs(
        &self,
        classes: Vec<ClassHash>,
        block_id: BlockNumber,
    ) -> Result<Option<GetStorageProofResponse>, BackendClientError> {
        trace!(classes = %classes.len(), block = %block_id, "Requesting classes proofs.");
        let (req, rx) = BackendRequest::classes_proof(classes, block_id);
        self.request(req)?;
        match rx.recv()? {
            BackendResponse::Proofs(res) => {
                if let Some(proofs) = handle_not_found_err(res)? {
                    Ok(Some(proofs))
                } else {
                    Ok(None)
                }
            }
            response => Err(BackendClientError::UnexpectedResponse(anyhow!("{response:?}"))),
        }
    }

    pub fn get_contracts_proofs(
        &self,
        contracts: Vec<ContractAddress>,
        block_id: BlockNumber,
    ) -> Result<Option<GetStorageProofResponse>, BackendClientError> {
        trace!(contracts = %contracts.len(), block = %block_id, "Requesting contracts proofs.");
        let (req, rx) = BackendRequest::contracts_proof(contracts, block_id);
        self.request(req)?;
        match rx.recv()? {
            BackendResponse::Proofs(res) => {
                if let Some(proofs) = handle_not_found_err(res)? {
                    Ok(Some(proofs))
                } else {
                    Ok(None)
                }
            }
            response => Err(BackendClientError::UnexpectedResponse(anyhow!("{response:?}"))),
        }
    }

    pub fn get_storages_proofs(
        &self,
        storages: Vec<ContractStorageKeys>,
        block_id: BlockNumber,
    ) -> Result<Option<GetStorageProofResponse>, BackendClientError> {
        trace!(contracts = %storages.len(), block = %block_id, "Requesting storages proofs.");
        let (req, rx) = BackendRequest::storages_proof(storages, block_id);
        self.request(req)?;
        match rx.recv()? {
            BackendResponse::Proofs(res) => {
                if let Some(proofs) = handle_not_found_err(res)? {
                    Ok(Some(proofs))
                } else {
                    Ok(None)
                }
            }
            response => Err(BackendClientError::UnexpectedResponse(anyhow!("{response:?}"))),
        }
    }

    pub fn get_global_roots(
        &self,
        block_id: BlockNumber,
    ) -> Result<Option<GetStorageProofResponse>, BackendClientError> {
        trace!(block = %block_id, "Requesting state roots.");
        let (req, rx) = BackendRequest::global_roots(block_id);
        self.request(req)?;
        match rx.recv()? {
            BackendResponse::GlobalRoots(res) => {
                if let Some(roots) = handle_not_found_err(res)? {
                    Ok(Some(roots))
                } else {
                    Ok(None)
                }
            }
            response => Err(BackendClientError::UnexpectedResponse(anyhow!("{response:?}"))),
        }
    }

    pub fn get_storage_root(
        &self,
        contract: ContractAddress,
        block_id: BlockNumber,
    ) -> Result<Option<Felt>, BackendClientError> {
        trace!(%contract, block = %block_id, "Requesting storage root.");
        let (req, rx) = BackendRequest::storage_root(contract, block_id);
        self.request(req)?;
        match rx.recv()? {
            BackendResponse::StorageRoot(res) => {
                if let Some(root) = handle_not_found_err(res)? {
                    Ok(Some(root))
                } else {
                    Ok(None)
                }
            }
            response => Err(BackendClientError::UnexpectedResponse(anyhow!("{response:?}"))),
        }
    }

    pub fn get_block_transactions_traces(
        &self,
        block_id: BlockHashOrNumber,
    ) -> Result<Option<TraceBlockTransactionsResponse>, BackendClientError> {
        trace!( block = %block_id, "Requesting trace block.");
        let (req, rx) = BackendRequest::trace(block_id);
        self.request(req)?;
        match rx.recv()? {
            BackendResponse::TraceBlockTransactions(res) => {
                 if let Some(res) = handle_not_found_err(res)? {
                    Ok(Some(res))
                } else {
                    Ok(None)
                }
}
            response => Err(BackendClientError::UnexpectedResponse(anyhow!("{response:?}"))),
        }
    }
    /// Send a request to the backend thread.
    fn request(&self, req: BackendRequest) -> Result<(), BackendClientError> {
        self.sender.lock().try_send(req).map_err(|e| e.into_send_error())?;
        Ok(())
    }

    /// Get the current number of pending requests.
    /// This is a test-only method that reads the atomic counter.
    #[cfg(test)]
    fn stats(&self) -> &BackendStats {
        &self.stats
    }
}

/// The types of response from [`Backend`].
///
/// This enum implements `Clone` because responses often need to be sent to multiple senders
/// when requests are deduplicated. In the request deduplication logic, when multiple clients
/// request the same data (e.g., the same contract's storage at the same key), only one actual
/// RPC request is made to the remote provider. When that request completes, the same response
/// needs to be distributed to all waiting senders, which requires cloning the response for each
/// sender in the deduplication vector.
#[derive(Debug, Clone)]
enum BackendResponse {
    Receipt(BackendResult<TxReceiptWithBlockInfo>),
    Block(BackendResult<GetBlockWithReceiptsResponse>),
    StateUpdate(BackendResult<StateUpdate>),
    Nonce(BackendResult<Nonce>),
    Storage(BackendResult<StorageValue>),
    ClassHashAt(BackendResult<ClassHash>),
    ClassAt(BackendResult<Class>),
    StorageRoot(BackendResult<Felt>),
    GlobalRoots(BackendResult<GetStorageProofResponse>),
    Proofs(BackendResult<GetStorageProofResponse>),
    TraceBlockTransactions(BackendResult<TraceBlockTransactionsResponse>),
}

/// Errors that can occur when interacting with the backend.
#[derive(Debug, thiserror::Error, Clone)]
pub enum BackendError {
    #[error("failed to spawn backend thread: {0}")]
    BackendThreadInit(#[from] Arc<io::Error>),
    #[error("rpc provider error: {0}")]
    StarknetProvider(#[from] Arc<katana_rpc_client::starknet::Error>),
    #[error("unexpected received result: {0}")]
    UnexpectedReceiveResult(Arc<anyhow::Error>),
}

struct Request<P> {
    payload: P,
    sender: OneshotSender<BackendResponse>,
}

#[derive(Eq, Hash, PartialEq, Clone, Debug)]
enum ProofsType {
    Classes(Vec<ClassHash>),
    Contracts(Vec<ContractAddress>),
    Storages(Vec<ContractStorageKeys>),
}

/// The types of request that can be sent to [`Backend`].
///
/// Each request consists of a payload and the sender half of a oneshot channel that will be used
/// to send the result back to the backend handle.
enum BackendRequest {
    Receipt(Request<TxHash>),
    GlobalRoots(Request<BlockNumber>),
    StorageRoot(Request<(ContractAddress, BlockNumber)>),
    Block(Request<BlockHashOrNumber>),
    StateUpdate(Request<BlockHashOrNumber>),
    Class(Request<(ClassHash, BlockNumber)>),
    Proofs(Request<(ProofsType, BlockNumber)>),
    Nonce(Request<(ContractAddress, BlockNumber)>),
    ClassHash(Request<(ContractAddress, BlockNumber)>),
    Storage(Request<((ContractAddress, StorageKey), BlockNumber)>),
    Trace(Request<BlockHashOrNumber>),
}

impl BackendRequest {
    fn receipt(tx_hash: TxHash) -> (BackendRequest, OneshotReceiver<BackendResponse>) {
        let (sender, receiver) = oneshot();
        (BackendRequest::Receipt(Request { payload: tx_hash, sender }), receiver)
    }

    /// Create a new request for fetching the nonce of a contract.
    fn block(block_id: BlockHashOrNumber) -> (BackendRequest, OneshotReceiver<BackendResponse>) {
        let (sender, receiver) = oneshot();
        (BackendRequest::Block(Request { payload: block_id, sender }), receiver)
    }

    fn state_update(
        block_id: BlockHashOrNumber,
    ) -> (BackendRequest, OneshotReceiver<BackendResponse>) {
        let (sender, receiver) = oneshot();
        (BackendRequest::StateUpdate(Request { payload: block_id, sender }), receiver)
    }

    /// Create a new request for fetching the nonce of a contract.
    fn nonce(
        address: ContractAddress,
        block_id: BlockNumber,
    ) -> (BackendRequest, OneshotReceiver<BackendResponse>) {
        let (sender, receiver) = oneshot();
        (BackendRequest::Nonce(Request { payload: (address, block_id), sender }), receiver)
    }

    /// Create a new request for fetching the class definitions of a contract.
    fn class(
        hash: ClassHash,
        block_id: BlockNumber,
    ) -> (BackendRequest, OneshotReceiver<BackendResponse>) {
        let (sender, receiver) = oneshot();
        (BackendRequest::Class(Request { payload: (hash, block_id), sender }), receiver)
    }

    /// Create a new request for fetching the class hash of a contract.
    fn class_hash(
        address: ContractAddress,
        block_id: BlockNumber,
    ) -> (BackendRequest, OneshotReceiver<BackendResponse>) {
        let (sender, receiver) = oneshot();
        (BackendRequest::ClassHash(Request { payload: (address, block_id), sender }), receiver)
    }

    /// Create a new request for fetching the storage value of a contract.
    fn storage(
        address: ContractAddress,
        key: StorageKey,
        block_id: BlockNumber,
    ) -> (BackendRequest, OneshotReceiver<BackendResponse>) {
        let (sender, receiver) = oneshot();
        (BackendRequest::Storage(Request { payload: ((address, key), block_id), sender }), receiver)
    }

    fn classes_proof(
        classes: Vec<ClassHash>,
        block_id: BlockNumber,
    ) -> (BackendRequest, OneshotReceiver<BackendResponse>) {
        let (sender, receiver) = oneshot();
        let payload = (ProofsType::Classes(classes), block_id);
        (BackendRequest::Proofs(Request { payload, sender }), receiver)
    }

    fn contracts_proof(
        contracts: Vec<ContractAddress>,
        block_id: BlockNumber,
    ) -> (BackendRequest, OneshotReceiver<BackendResponse>) {
        let (sender, receiver) = oneshot();
        let payload = (ProofsType::Contracts(contracts), block_id);
        (BackendRequest::Proofs(Request { payload, sender }), receiver)
    }

    fn storages_proof(
        storages: Vec<ContractStorageKeys>,
        block_id: BlockNumber,
    ) -> (BackendRequest, OneshotReceiver<BackendResponse>) {
        let (sender, receiver) = oneshot();
        let payload = (ProofsType::Storages(storages), block_id);
        (BackendRequest::Proofs(Request { payload, sender }), receiver)
    }

    fn global_roots(block_id: BlockNumber) -> (BackendRequest, OneshotReceiver<BackendResponse>) {
        let (sender, receiver) = oneshot();
        (BackendRequest::GlobalRoots(Request { payload: block_id, sender }), receiver)
    }

    fn storage_root(
        contract: ContractAddress,
        block_id: BlockNumber,
    ) -> (BackendRequest, OneshotReceiver<BackendResponse>) {
        let (sender, receiver) = oneshot();
        (BackendRequest::StorageRoot(Request { payload: (contract, block_id), sender }), receiver)
    }

    fn trace(block_id: BlockHashOrNumber) -> (BackendRequest, OneshotReceiver<BackendResponse>) {
        let (sender, receiver) = oneshot();
        (BackendRequest::Trace(Request { payload: block_id, sender }), receiver)
    }
}

type BackendRequestFuture = BoxFuture<'static, BackendResponse>;

// Identifier for pending requests.
// This is used for request deduplication.
#[derive(Eq, Hash, PartialEq, Clone, Debug)]
enum BackendRequestIdentifier {
    Receipt(TxHash),
    GlobalRoots(BlockNumber),
    StorageRoot(ContractAddress, BlockNumber),
    Block(BlockHashOrNumber),
    Proofs(ProofsType, BlockNumber),
    StateUpdate(BlockHashOrNumber),
    Nonce(ContractAddress, BlockNumber),
    Class(ClassHash, BlockNumber),
    ClassHash(ContractAddress, BlockNumber),
    Storage((ContractAddress, StorageKey), BlockNumber),
    Trace(BlockHashOrNumber),
}

/// Metrics for the forking backend.
#[derive(Metrics, Clone)]
#[metrics(scope = "forking.backend")]
pub struct BackendMetrics {
    /// Number of requests currently being processed (pending)
    pub pending_requests: Gauge,
    /// Number of requests queued and waiting to be processed
    pub queued_requests: Gauge,
}

/////////////////////////////////////////////////////////////////
// BackendWorker
/////////////////////////////////////////////////////////////////

/// The backend for the forked provider.
///
/// It is responsible for processing [requests](BackendRequest) to fetch data from the remote
/// provider.
struct BackendWorker {
    /// The Starknet RPC provider that will be used to fetch data from.
    starknet_client: Arc<StarknetClient>,
    // HashMap that keep track of current requests, for dedup purposes.
    request_dedup_map: HashMap<BackendRequestIdentifier, Vec<OneshotSender<BackendResponse>>>,
    /// Requests that are currently being poll.
    pending_requests: Vec<(BackendRequestIdentifier, BackendRequestFuture)>,
    /// Requests that are queued to be polled.
    queued_requests: VecDeque<BackendRequest>,
    /// A channel for receiving requests from the [Backend]s.
    incoming: Receiver<BackendRequest>,
    /// Maximum number of concurrent requests that can be processed.
    max_concurrent_requests: usize,
    /// Metrics for the backend.
    metrics: BackendMetrics,
    #[cfg(test)]
    stats: BackendStats,
}

/////////////////////////////////////////////////////////////////
// BackendWorker implementation
/////////////////////////////////////////////////////////////////

impl BackendWorker {
    /// This method is responsible for transforming the incoming request
    /// sent from a [Backend] into a RPC request to the remote network.
    fn handle_requests(&mut self, request: BackendRequest) {
        let provider = self.starknet_client.clone();

        // Check if there are similar requests in the queue before sending the request
        match request {
            BackendRequest::Receipt(Request { payload: tx_hash, sender }) => {
                let req_key = BackendRequestIdentifier::Receipt(tx_hash);

                self.dedup_request(
                    req_key,
                    sender,
                    Box::pin(async move {
                        let res = provider
                            .get_transaction_receipt(tx_hash)
                            .await
                            .map_err(|e| BackendError::StarknetProvider(Arc::new(e)));

                        BackendResponse::Receipt(res)
                    }),
                );
            }

            BackendRequest::Block(Request { payload: block_id, sender }) => {
                let req_key = BackendRequestIdentifier::Block(block_id);
                let block_id = BlockIdOrTag::from(block_id);

                self.dedup_request(
                    req_key,
                    sender,
                    Box::pin(async move {
                        let res = provider
                            .get_block_with_receipts(block_id)
                            .await
                            .map_err(|e| BackendError::StarknetProvider(Arc::new(e)));

                        BackendResponse::Block(res)
                    }),
                );
            }

            BackendRequest::StateUpdate(Request { payload: block_id, sender }) => {
                let req_key = BackendRequestIdentifier::StateUpdate(block_id);
                let block_id = BlockIdOrTag::from(block_id);

                self.dedup_request(
                    req_key,
                    sender,
                    Box::pin(async move {
                        let res = provider
                            .get_state_update(block_id)
                            .await
                            .map_err(|e| BackendError::StarknetProvider(Arc::new(e)));

                        BackendResponse::StateUpdate(res)
                    }),
                );
            }

            BackendRequest::Nonce(Request { payload: (address, block_id), sender }) => {
                let req_key = BackendRequestIdentifier::Nonce(address, block_id);
                let block_id = BlockIdOrTag::from(block_id);

                self.dedup_request(
                    req_key,
                    sender,
                    Box::pin(async move {
                        let res = provider
                            .get_nonce(block_id, address)
                            .await
                            .map_err(|e| BackendError::StarknetProvider(Arc::new(e)));

                        BackendResponse::Nonce(res)
                    }),
                );
            }

            BackendRequest::Storage(Request { payload: ((addr, key), block_id), sender }) => {
                let req_key = BackendRequestIdentifier::Storage((addr, key), block_id);
                let block_id = BlockIdOrTag::from(block_id);

                self.dedup_request(
                    req_key,
                    sender,
                    Box::pin(async move {
                        let res = provider
                            .get_storage_at(addr, key, block_id)
                            .await
                            .map_err(|e| BackendError::StarknetProvider(Arc::new(e)));

                        BackendResponse::Storage(res)
                    }),
                );
            }

            BackendRequest::ClassHash(Request { payload: (address, block_id), sender }) => {
                let req_key = BackendRequestIdentifier::ClassHash(address, block_id);
                let block_id = BlockIdOrTag::from(block_id);

                self.dedup_request(
                    req_key,
                    sender,
                    Box::pin(async move {
                        let res = provider
                            .get_class_hash_at(block_id, address)
                            .await
                            .map_err(|e| BackendError::StarknetProvider(Arc::new(e)));

                        BackendResponse::ClassHashAt(res)
                    }),
                );
            }

            BackendRequest::Class(Request { payload: (hash, block_id), sender }) => {
                let req_key = BackendRequestIdentifier::Class(hash, block_id);
                let block_id = BlockIdOrTag::from(block_id);

                self.dedup_request(
                    req_key,
                    sender,
                    Box::pin(async move {
                        let res = provider
                            .get_class(block_id, hash)
                            .await
                            .map_err(|e| BackendError::StarknetProvider(Arc::new(e)));

                        BackendResponse::ClassAt(res)
                    }),
                );
            }

            BackendRequest::Proofs(Request { payload: (r#type, block_id), sender }) => {
                let req_key = BackendRequestIdentifier::Proofs(r#type.clone(), block_id);
                let block_id = BlockIdOrTag::from(block_id);

                let request: BoxFuture<'static, BackendResponse> = match r#type {
                    ProofsType::Classes(classes) => Box::pin(async move {
                        let res = provider
                            .get_storage_proof(block_id, Some(classes), None, None)
                            .await
                            .map_err(|e| BackendError::StarknetProvider(Arc::new(e)));

                        BackendResponse::Proofs(res)
                    }),

                    ProofsType::Contracts(contracts) => Box::pin(async move {
                        let res = provider
                            .get_storage_proof(block_id, None, Some(contracts), None)
                            .await
                            .map_err(|e| BackendError::StarknetProvider(Arc::new(e)));

                        BackendResponse::Proofs(res)
                    }),

                    ProofsType::Storages(storage_keys) => Box::pin(async move {
                        let res = provider
                            .get_storage_proof(block_id, None, None, Some(storage_keys))
                            .await
                            .map_err(|e| BackendError::StarknetProvider(Arc::new(e)));

                        BackendResponse::Proofs(res)
                    }),
                };

                self.dedup_request(req_key, sender, request);
            }

            BackendRequest::GlobalRoots(Request { payload: block_id, sender }) => {
                let req_key = BackendRequestIdentifier::GlobalRoots(block_id);
                let block_id = BlockIdOrTag::from(block_id);

                self.dedup_request(
                    req_key,
                    sender,
                    Box::pin(async move {
                        let res = provider
                            .get_storage_proof(block_id, None, None, None)
                            .await
                            .map_err(|e| BackendError::StarknetProvider(Arc::new(e)));

                        BackendResponse::GlobalRoots(res)
                    }),
                );
            }

            BackendRequest::StorageRoot(Request { payload: (contract, block_id), sender }) => {
                let req_key = BackendRequestIdentifier::StorageRoot(contract, block_id);
                let block_id = BlockIdOrTag::from(block_id);

                self.dedup_request(
                    req_key,
                    sender,
                    Box::pin(async move {
                        let res = provider
                            .get_storage_proof(block_id, None, Some(vec![contract]), None)
                            .await
                            .map(|mut proof| {
                                proof
                                    .contracts_proof
                                    .contract_leaves_data
                                    .pop()
                                    .expect("must exist")
                                    .storage_root
                            })
                            .map_err(|e| BackendError::StarknetProvider(Arc::new(e)));

                        BackendResponse::StorageRoot(res)
                    }),
                );
            }
            BackendRequest::Trace(Request { payload: block_id, sender }) => {
                let req_key = BackendRequestIdentifier::Trace(block_id);
                let block_id = match block_id {
                    BlockHashOrNumber::Hash(h) => ConfirmedBlockIdOrTag::Hash(h),
                    BlockHashOrNumber::Num(n) => ConfirmedBlockIdOrTag::Number(n),
                };
                
                self.dedup_request(
                    req_key,
                    sender,
                    Box::pin(async move {
                        let result = provider
                            .trace_block_transactions(block_id)
                            .await
                            .map_err(|e| BackendError::StarknetProvider(Arc::new(e)));
                        BackendResponse::TraceBlockTransactions(result)
                    }),
                );
            }
        }
    }

    fn dedup_request(
        &mut self,
        req_key: BackendRequestIdentifier,
        sender: OneshotSender<BackendResponse>,
        rpc_call_future: BoxFuture<'static, BackendResponse>,
    ) {
        if let Entry::Vacant(e) = self.request_dedup_map.entry(req_key.clone()) {
            self.pending_requests.push((req_key, rpc_call_future));
            e.insert(vec![sender]);
        } else {
            match self.request_dedup_map.get_mut(&req_key) {
                Some(sender_vec) => {
                    sender_vec.push(sender);
                }
                None => {
                    // Log this and do nothing here, as this should never happen.
                    // If this does happen it is an unexpected bug.
                    error!("failed to get current request dedup vector");
                }
            }
        }
    }
}

impl Future for BackendWorker {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let pin = self.get_mut();
        loop {
            // convert queued requests into futures to be polled, respecting the concurrent limit
            while !pin.queued_requests.is_empty()
                && pin.pending_requests.len() < pin.max_concurrent_requests
            {
                if let Some(req) = pin.queued_requests.pop_front() {
                    pin.handle_requests(req);
                }
            }

            // Update metrics and atomic counters
            let pending_count = pin.pending_requests.len();
            let queued_count = pin.queued_requests.len();
            pin.metrics.pending_requests.set(pending_count as f64);
            pin.metrics.queued_requests.set(queued_count as f64);

            #[cfg(test)]
            {
                pin.stats.pending_requests_count.store(pending_count, Ordering::Relaxed);
                pin.stats.queued_requests_count.store(queued_count, Ordering::Relaxed);
            }

            loop {
                match Pin::new(&mut pin.incoming).poll_next(cx) {
                    Poll::Ready(Some(req)) => {
                        pin.queued_requests.push_back(req);
                    }
                    // Resolve if stream is exhausted.
                    Poll::Ready(None) => {
                        return Poll::Ready(());
                    }
                    Poll::Pending => {
                        break;
                    }
                }
            }

            // poll all pending requests
            for n in (0..pin.pending_requests.len()).rev() {
                let (fut_key, mut fut) = pin.pending_requests.swap_remove(n);
                // poll the future and if the future is still pending, push it back to the
                // pending requests so that it will be polled again
                match fut.poll_unpin(cx) {
                    Poll::Pending => {
                        pin.pending_requests.push((fut_key, fut));
                    }
                    Poll::Ready(res) => {
                        let sender_vec = pin
                            .request_dedup_map
                            .get(&fut_key)
                            .expect("failed to get sender vector");

                        // Send the response to all the senders waiting on the same request
                        sender_vec.iter().for_each(|sender| {
                            sender.send(res.clone()).unwrap_or_else(
                                |error| error!(key = ?fut_key, %error, "Failed to send result."),
                            );
                        });

                        pin.request_dedup_map.remove(&fut_key);
                    }
                }
            }

            // Update metrics and atomic counters after processing
            let pending_count = pin.pending_requests.len();
            let queued_count = pin.queued_requests.len();
            pin.metrics.pending_requests.set(pending_count as f64);
            pin.metrics.queued_requests.set(queued_count as f64);

            #[cfg(test)]
            {
                pin.stats.pending_requests_count.store(pending_count, Ordering::Relaxed);
                pin.stats.queued_requests_count.store(queued_count, Ordering::Relaxed);
            }

            // if no queued requests, then yield
            if pin.queued_requests.is_empty() {
                return Poll::Pending;
            }
        }
    }
}

impl std::fmt::Debug for BackendWorker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Backend")
            .field("provider", &self.starknet_client)
            .field("request_dedup_map", &self.request_dedup_map)
            .field("pending_requests", &self.pending_requests.len())
            .field("queued_requests", &self.queued_requests.len())
            .field("incoming", &self.incoming)
            .field("max_concurrent_requests", &self.max_concurrent_requests)
            .finish()
    }
}

#[cfg(test)]
#[derive(Debug, Clone, Default)]
struct BackendStats {
    pending_requests_count: Arc<std::sync::atomic::AtomicUsize>,
    queued_requests_count: Arc<std::sync::atomic::AtomicUsize>,
}

#[cfg(test)]
impl BackendStats {
    fn pending_requests_count(&self) -> usize {
        self.pending_requests_count.load(Ordering::Relaxed)
    }

    fn queued_requests_count(&self) -> usize {
        self.queued_requests_count.load(Ordering::Relaxed)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum BackendClientError {
    #[error("failed to send request to backend: {0}")]
    FailedSendRequest(#[from] SendError),

    #[error("failed to receive result from backend: {0}")]
    FailedReceiveResult(#[from] RecvError),

    #[error(transparent)]
    BackendError(#[from] BackendError),

    #[error("failed to convert class: {0}")]
    ClassConversion(#[from] katana_rpc_types::class::ConversionError),

    #[error("failed to compile class: {0}")]
    ClassCompilation(#[from] ContractClassCompilationError),

    #[error("failed to compute class hash: {0}")]
    ClassHashComputation(#[from] ComputeClassHashError),

    #[error("unexpected response: {0}")]
    UnexpectedResponse(anyhow::Error),
}

/// A helper function to convert a contract/class not found error returned by the RPC provider into
/// a `Option::None`.
///
/// This is to follow the Katana's provider APIs convention where 'not found'/'non-existent' should
/// be represented as `Option::None`.
fn handle_not_found_err<T>(
    result: Result<T, BackendError>,
) -> Result<Option<T>, BackendClientError> {
    match result {
        Ok(value) => Ok(Some(value)),

        Err(BackendError::StarknetProvider(err)) => match err.as_ref() {
            StarknetClientError::Starknet(StarknetApiError::BlockNotFound) => Ok(None),
            StarknetClientError::Starknet(StarknetApiError::TxnHashNotFound) => Ok(None),
            StarknetClientError::Starknet(StarknetApiError::ContractNotFound) => Ok(None),
            StarknetClientError::Starknet(StarknetApiError::ClassHashNotFound) => Ok(None),
            _ => Err(BackendClientError::BackendError(BackendError::StarknetProvider(err))),
        },

        Err(err) => Err(BackendClientError::BackendError(err)),
    }
}

#[cfg(test)]
pub(crate) mod test_utils {

    use std::sync::mpsc::{sync_channel, SyncSender};
    use std::time::Duration;

    use serde_json::{json, Value};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use url::Url;

    use super::*;

    pub fn create_forked_backend(rpc_url: &str) -> Backend {
        let url = Url::parse(rpc_url).expect("valid url");
        let provider = StarknetClient::new(url);
        Backend::new(provider).unwrap()
    }

    pub fn create_forked_backend_with_config(
        rpc_url: &str,
        max_concurrent_requests: usize,
    ) -> Backend {
        let url = Url::parse(rpc_url).expect("valid url");
        let provider = StarknetClient::new(url);
        Backend::new_with_config(provider, max_concurrent_requests).unwrap()
    }

    // Starts a TCP server that never close the connection.
    pub fn start_tcp_server(addr: String) {
        use tokio::runtime::Builder;

        let (tx, rx) = sync_channel::<()>(1);
        std::thread::spawn(move || {
            Builder::new_current_thread().enable_all().build().unwrap().block_on(async move {
                let listener = TcpListener::bind(addr).await.unwrap();
                let mut connections = Vec::new();

                tx.send(()).unwrap();

                loop {
                    let (socket, _) = listener.accept().await.unwrap();
                    connections.push(socket);
                }
            });
        });

        rx.recv().unwrap();
    }

    // Helper function to start a TCP server that returns predefined JSON-RPC responses
    // The `result` parameter should be the value to return in the "result" field of the JSON-RPC
    // response
    pub fn start_mock_rpc_server(addr: String, result: String) -> SyncSender<()> {
        use tokio::runtime::Builder;

        let (resp_signal_tx, resp_signal_rx) = sync_channel::<()>(100);
        let (server_ready_tx, server_ready_rx) = sync_channel::<()>(1);

        std::thread::spawn(move || {
            Builder::new_current_thread().enable_all().build().unwrap().block_on(async move {
                let listener = TcpListener::bind(addr).await.unwrap();
                let pending_requests = Arc::new(Mutex::new(VecDeque::new()));

                server_ready_tx.send(()).unwrap();

                // Spawn a task to accept incoming connections
                let pending_requests_accept = pending_requests.clone();
                let accept_task_handle = tokio::spawn(async move {
                    loop {
                        let (mut socket, _) = listener.accept().await.unwrap();

                        // Read the request
                        let mut buffer = [0; 1024];
                        let n = socket.read(&mut buffer).await.unwrap();

                        // Parse the HTTP request to extract the JSON-RPC body
                        let request_str = String::from_utf8_lossy(&buffer[..n]);
                        let id = if let Some(body_start) = request_str.rfind("\r\n\r\n") {
                            let body = &request_str[body_start + 4..];
                            let request = serde_json::from_str::<Value>(body).unwrap();
                            request.get("id").unwrap().clone()
                        } else {
                            json!(1)
                        };

                        // Store the socket and ID for later processing (the ID needs to be equal in
                        // both request and response as per JSON-RPC spec)
                        pending_requests_accept.lock().push_back((socket, id));
                    }
                });

                // Process requests in FIFO order based on signals
                loop {
                    // Wait for a signal to process the next request triggered by consumer
                    if resp_signal_rx.recv().is_err() {
                        break;
                    }

                    // Get the next pending request
                    let (mut socket, id) = {
                        let mut queue = pending_requests.lock();
                        // Wait until there's a request to process
                        while queue.is_empty() {
                            drop(queue);
                            tokio::time::sleep(Duration::from_millis(10)).await;
                            queue = pending_requests.lock();
                        }

                        queue.pop_front().unwrap()
                    };

                    let json_response = serde_json::to_string(&json!({
                        "id": id,
                        "jsonrpc": "2.0",
                        "result": result
                    }))
                    .unwrap();

                    let http_response = format!(
                        "HTTP/1.1 200 OK\r\ncontent-length: {}\r\ncontent-type: \
                         application/json\r\n\r\n{}",
                        json_response.len(),
                        json_response
                    );

                    socket.write_all(http_response.as_bytes()).await.unwrap();
                    socket.flush().await.unwrap();
                }

                accept_task_handle.abort();
            });
        });

        // Wait for the server to be ready
        server_ready_rx.recv().unwrap();

        // Returning the sender to allow controlling the response timing.
        resp_signal_tx
    }
}

impl From<BackendClientError> for katana_provider_api::ProviderError {
    fn from(value: BackendClientError) -> Self {
        Self::Other(value.to_string())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::time::Duration;

    use starknet::macros::felt;

    use super::test_utils::*;
    use super::*;

    const ERROR_SEND_REQUEST: &str = "Failed to send request to backend";

    #[test]
    fn handle_incoming_requests() {
        // start a mock remote network
        start_tcp_server("127.0.0.1:8080".to_string());

        let handle = create_forked_backend("http://127.0.0.1:8080");
        let block_id = 1;

        // check no pending requests
        let pending_requests_count = handle.stats().pending_requests_count();
        assert_eq!(pending_requests_count, 0, "Backend should not have any ongoing requests.");

        // send requests to the backend
        let h1 = handle.clone();
        std::thread::spawn(move || {
            h1.get_nonce(felt!("0x1").into(), block_id).expect(ERROR_SEND_REQUEST);
        });
        let h2 = handle.clone();
        std::thread::spawn(move || {
            h2.get_class_at(felt!("0x1"), block_id).expect(ERROR_SEND_REQUEST);
        });
        let h3 = handle.clone();
        std::thread::spawn(move || {
            h3.get_compiled_class_hash(felt!("0x2"), block_id).expect(ERROR_SEND_REQUEST);
        });
        let h4 = handle.clone();
        std::thread::spawn(move || {
            h4.get_class_hash_at(felt!("0x1").into(), block_id).expect(ERROR_SEND_REQUEST);
        });
        let h5 = handle.clone();
        std::thread::spawn(move || {
            h5.get_storage(felt!("0x1").into(), felt!("0x1"), block_id).expect(ERROR_SEND_REQUEST);
        });

        // wait for the requests to be handled
        std::thread::sleep(Duration::from_secs(1));

        // check request are handled
        let pending_requests_count = handle.stats().pending_requests_count();
        assert_eq!(pending_requests_count, 5, "Backend should have 5 ongoing requests.")
    }

    #[test]
    fn get_nonce_request_should_be_deduplicated() {
        // start a mock remote network
        start_tcp_server("127.0.0.1:8081".to_string());

        let handle = create_forked_backend("http://127.0.0.1:8081");
        let block_id = 1;

        // check no pending requests
        let pending_requests_count = handle.stats().pending_requests_count();
        assert_eq!(pending_requests_count, 0, "Backend should not have any ongoing requests.");

        // send requests to the backend
        let h1 = handle.clone();
        std::thread::spawn(move || {
            h1.get_nonce(felt!("0x1").into(), block_id).expect(ERROR_SEND_REQUEST);
        });
        let h2 = handle.clone();
        std::thread::spawn(move || {
            h2.get_nonce(felt!("0x1").into(), block_id).expect(ERROR_SEND_REQUEST);
        });

        // wait for the requests to be handled
        std::thread::sleep(Duration::from_secs(1));

        // check current request count
        let pending_requests_count = handle.stats().pending_requests_count();
        assert_eq!(pending_requests_count, 1, "Backend should have 1 ongoing requests.");

        // Different request, should be counted
        let h3 = handle.clone();
        std::thread::spawn(move || {
            h3.get_nonce(felt!("0x2").into(), block_id).expect(ERROR_SEND_REQUEST);
        });

        // wait for the requests to be handled
        std::thread::sleep(Duration::from_secs(1));

        // check request are handled
        let pending_requests_count = handle.stats().pending_requests_count();
        assert_eq!(pending_requests_count, 2, "Backend should only have 2 ongoing requests.")
    }

    #[test]
    fn get_class_at_request_should_be_deduplicated() {
        // start a mock remote network
        start_tcp_server("127.0.0.1:8082".to_string());

        let handle = create_forked_backend("http://127.0.0.1:8082");
        let block_id = 1;

        // check no pending requests
        let pending_requests_count = handle.stats().pending_requests_count();
        assert_eq!(pending_requests_count, 0, "Backend should not have any ongoing requests.");

        // send requests to the backend
        let h1 = handle.clone();
        std::thread::spawn(move || {
            h1.get_class_at(felt!("0x1"), block_id).expect(ERROR_SEND_REQUEST);
        });
        let h2 = handle.clone();
        std::thread::spawn(move || {
            h2.get_class_at(felt!("0x1"), block_id).expect(ERROR_SEND_REQUEST);
        });

        // wait for the requests to be handled
        std::thread::sleep(Duration::from_secs(1));

        // check current request count
        let pending_requests_count = handle.stats().pending_requests_count();
        assert_eq!(pending_requests_count, 1, "Backend should have 1 ongoing requests.");

        // Different request, should be counted
        let h3 = handle.clone();
        std::thread::spawn(move || {
            h3.get_class_at(felt!("0x2"), block_id).expect(ERROR_SEND_REQUEST);
        });

        // wait for the requests to be handled
        std::thread::sleep(Duration::from_secs(1));

        // check request are handled
        let pending_requests_count = handle.stats().pending_requests_count();
        assert_eq!(pending_requests_count, 2, "Backend should only have 2 ongoing requests.")
    }

    #[test]
    fn get_compiled_class_hash_request_should_be_deduplicated() {
        // start a mock remote network
        start_tcp_server("127.0.0.1:8083".to_string());

        let handle = create_forked_backend("http://127.0.0.1:8083");
        let block_id = 1;

        // check no pending requests
        let pending_requests_count = handle.stats().pending_requests_count();
        assert_eq!(pending_requests_count, 0, "Backend should not have any ongoing requests.");

        // send requests to the backend
        let h1 = handle.clone();
        std::thread::spawn(move || {
            h1.get_compiled_class_hash(felt!("0x1"), block_id).expect(ERROR_SEND_REQUEST);
        });
        let h2 = handle.clone();
        std::thread::spawn(move || {
            h2.get_compiled_class_hash(felt!("0x1"), block_id).expect(ERROR_SEND_REQUEST);
        });

        // wait for the requests to be handled
        std::thread::sleep(Duration::from_secs(1));

        // check current request count
        let pending_requests_count = handle.stats().pending_requests_count();
        assert_eq!(pending_requests_count, 1, "Backend should have 1 ongoing requests.");

        // Different request, should be counted
        let h3 = handle.clone();
        std::thread::spawn(move || {
            h3.get_compiled_class_hash(felt!("0x2"), block_id).expect(ERROR_SEND_REQUEST);
        });

        // wait for the requests to be handled
        std::thread::sleep(Duration::from_secs(1));

        // check request are handled
        let pending_requests_count = handle.stats().pending_requests_count();
        assert_eq!(pending_requests_count, 2, "Backend should only have 2 ongoing requests.")
    }

    #[test]
    fn get_class_at_and_get_compiled_class_hash_request_should_be_deduplicated() {
        // start a mock remote network
        start_tcp_server("127.0.0.1:8084".to_string());

        let handle = create_forked_backend("http://127.0.0.1:8084");
        let block_id = 1;

        // check no pending requests
        let pending_requests_count = handle.stats().pending_requests_count();
        assert_eq!(pending_requests_count, 0, "Backend should not have any ongoing requests.");

        // send requests to the backend
        let h1 = handle.clone();
        std::thread::spawn(move || {
            h1.get_class_at(felt!("0x1"), block_id).expect(ERROR_SEND_REQUEST);
        });
        // Since this also calls to the same request as the previous one, it should be deduped
        let h2 = handle.clone();
        std::thread::spawn(move || {
            h2.get_compiled_class_hash(felt!("0x1"), block_id).expect(ERROR_SEND_REQUEST);
        });

        // wait for the requests to be handled
        std::thread::sleep(Duration::from_secs(1));

        // check current request count
        let pending_requests_count = handle.stats().pending_requests_count();
        assert_eq!(pending_requests_count, 1, "Backend should have 1 ongoing requests.");

        // Different request, should be counted
        let h3 = handle.clone();
        std::thread::spawn(move || {
            h3.get_class_at(felt!("0x2"), block_id).expect(ERROR_SEND_REQUEST);
        });

        // wait for the requests to be handled
        std::thread::sleep(Duration::from_secs(1));

        // check request are handled
        let pending_requests_count = handle.stats().pending_requests_count();
        assert_eq!(pending_requests_count, 2, "Backend should only have 2 ongoing requests.")
    }

    #[test]
    fn get_class_hash_at_request_should_be_deduplicated() {
        // start a mock remote network
        start_tcp_server("127.0.0.1:8085".to_string());

        let handle = create_forked_backend("http://127.0.0.1:8085");
        let block_id = 1;

        // check no pending requests
        let pending_requests_count = handle.stats().pending_requests_count();
        assert_eq!(pending_requests_count, 0, "Backend should not have any ongoing requests.");

        // send requests to the backend
        let h1 = handle.clone();
        std::thread::spawn(move || {
            h1.get_class_hash_at(felt!("0x1").into(), block_id).expect(ERROR_SEND_REQUEST);
        });
        let h2 = handle.clone();
        std::thread::spawn(move || {
            h2.get_class_hash_at(felt!("0x1").into(), block_id).expect(ERROR_SEND_REQUEST);
        });

        // wait for the requests to be handled
        std::thread::sleep(Duration::from_secs(1));

        // check current request count
        let pending_requests_count = handle.stats().pending_requests_count();
        assert_eq!(pending_requests_count, 1, "Backend should have 1 ongoing requests.");

        // Different request, should be counted
        let h3 = handle.clone();
        std::thread::spawn(move || {
            h3.get_class_hash_at(felt!("0x2").into(), block_id).expect(ERROR_SEND_REQUEST);
        });

        // wait for the requests to be handled
        std::thread::sleep(Duration::from_secs(1));

        // check request are handled
        let pending_requests_count = handle.stats().pending_requests_count();
        assert_eq!(pending_requests_count, 2, "Backend should only have 2 ongoing requests.")
    }

    #[test]
    fn get_storage_request_should_be_deduplicated() {
        // start a mock remote network
        start_tcp_server("127.0.0.1:8086".to_string());

        let handle = create_forked_backend("http://127.0.0.1:8086");
        let block_id = 1;

        // check no pending requests
        let pending_requests_count = handle.stats().pending_requests_count();
        assert_eq!(pending_requests_count, 0, "Backend should not have any ongoing requests.");

        // send requests to the backend
        let h1 = handle.clone();
        std::thread::spawn(move || {
            h1.get_storage(felt!("0x1").into(), felt!("0x1"), block_id).expect(ERROR_SEND_REQUEST);
        });
        let h2 = handle.clone();
        std::thread::spawn(move || {
            h2.get_storage(felt!("0x1").into(), felt!("0x1"), block_id).expect(ERROR_SEND_REQUEST);
        });

        // wait for the requests to be handled
        std::thread::sleep(Duration::from_secs(1));

        // check current request count
        let pending_requests_count = handle.stats().pending_requests_count();
        assert_eq!(pending_requests_count, 1, "Backend should have 1 ongoing requests.");

        // Different request, should be counted
        let h3 = handle.clone();
        std::thread::spawn(move || {
            h3.get_storage(felt!("0x2").into(), felt!("0x3"), block_id).expect(ERROR_SEND_REQUEST);
        });

        // wait for the requests to be handled
        std::thread::sleep(Duration::from_secs(1));

        // check request are handled
        let pending_requests_count = handle.stats().pending_requests_count();
        assert_eq!(pending_requests_count, 2, "Backend should only have 2 ongoing requests.")
    }

    #[test]
    fn get_storage_request_on_same_address_with_different_key_should_be_deduplicated() {
        // start a mock remote network
        start_tcp_server("127.0.0.1:8087".to_string());

        let handle = create_forked_backend("http://127.0.0.1:8087");
        let block_id = 1;

        // check no pending requests
        let pending_requests_count = handle.stats().pending_requests_count();
        assert_eq!(pending_requests_count, 0, "Backend should not have any ongoing requests.");

        // send requests to the backend
        let h1 = handle.clone();
        std::thread::spawn(move || {
            h1.get_storage(felt!("0x1").into(), felt!("0x1"), block_id).expect(ERROR_SEND_REQUEST);
        });
        let h2 = handle.clone();
        std::thread::spawn(move || {
            h2.get_storage(felt!("0x1").into(), felt!("0x1"), block_id).expect(ERROR_SEND_REQUEST);
        });

        // wait for the requests to be handled
        std::thread::sleep(Duration::from_secs(1));

        // check current request count
        let pending_requests_count = handle.stats().pending_requests_count();
        assert_eq!(pending_requests_count, 1, "Backend should have 1 ongoing requests.");

        // Different request, should be counted
        let h3 = handle.clone();
        std::thread::spawn(move || {
            h3.get_storage(felt!("0x1").into(), felt!("0x3"), block_id).expect(ERROR_SEND_REQUEST);
        });
        // Different request, should be counted
        let h4 = handle.clone();
        std::thread::spawn(move || {
            h4.get_storage(felt!("0x1").into(), felt!("0x6"), block_id).expect(ERROR_SEND_REQUEST);
        });

        // wait for the requests to be handled
        std::thread::sleep(Duration::from_secs(1));

        // check current request count
        let pending_requests_count = handle.stats().pending_requests_count();
        assert_eq!(pending_requests_count, 3, "Backend should have 3 ongoing requests.");

        // Same request as the last one, shouldn't be counted
        let h5 = handle.clone();
        std::thread::spawn(move || {
            h5.get_storage(felt!("0x1").into(), felt!("0x6"), block_id).expect(ERROR_SEND_REQUEST);
        });

        // wait for the requests to be handled
        std::thread::sleep(Duration::from_secs(1));

        // check request are handled
        let pending_requests_count = handle.stats().pending_requests_count();
        assert_eq!(pending_requests_count, 3, "Backend should only have 3 ongoing requests.")
    }

    #[test]
    fn test_deduplicated_request_should_return_similar_results() {
        // Start mock server with a predefined nonce result value
        let result = "0x123";
        let sender = start_mock_rpc_server("127.0.0.1:8090".to_string(), result.to_string());

        let handle = create_forked_backend("http://127.0.0.1:8090");
        let block_id = 1;
        let addr = ContractAddress(felt!("0x1"));

        // Collect results from multiple identical nonce requests
        let results: Arc<Mutex<Vec<_>>> = Arc::new(Mutex::new(Vec::new()));

        let handles: Vec<_> = (0..5)
            .map(|_| {
                let h = handle.clone();
                let results = results.clone();
                std::thread::spawn(move || {
                    let res = h.get_nonce(addr, block_id);
                    results.lock().unwrap().push(res);
                })
            })
            .collect();

        // wait for the requests to be sent to the rpc server
        std::thread::sleep(Duration::from_secs(1));

        // Check that there's only one request, meaning it is deduplicated.
        let pending_requests_count = handle.stats().pending_requests_count();
        assert_eq!(pending_requests_count, 1, "Backend should only have 1 ongoing requests.");

        // Send the signal to tell the mock rpc server to return the response
        sender.send(()).unwrap();

        // Join all request threads
        handles.into_iter().for_each(|h| h.join().unwrap());

        // Verify all results are identical
        let results = results.lock().unwrap();
        for result in results.iter() {
            assert_eq!(
                &Some(felt!("0x123")),
                result.as_ref().unwrap(),
                "All deduplicated nonce requests should return the same result"
            );
        }
    }

    #[test]
    fn test_concurrent_request_limit() {
        // Start a mock remote network that never responds
        start_tcp_server("127.0.0.1:8091".to_string());

        // Create a backend with a low concurrent request limit
        let max_concurrent = 3;
        let handle = create_forked_backend_with_config("http://127.0.0.1:8091", max_concurrent);
        let block_id = 1;

        // Check no pending requests initially
        let pending_requests_count = handle.stats().pending_requests_count();
        assert_eq!(pending_requests_count, 0, "Backend should not have any ongoing requests.");

        // Send more requests than the limit (each with different parameters to avoid deduplication)
        let addresses = [
            felt!("0x1"),
            felt!("0x2"),
            felt!("0x3"),
            felt!("0x4"),
            felt!("0x5"),
            felt!("0x6"),
            felt!("0x7"),
            felt!("0x8"),
            felt!("0x9"),
            felt!("0xa"),
        ];

        for address in addresses {
            let h = handle.clone();
            std::thread::spawn(move || {
                let _ = h.get_nonce(address.into(), block_id);
            });
        }

        // Wait for requests to be processed
        std::thread::sleep(Duration::from_secs(1));

        // Verify that the number of pending requests does not exceed the limit
        let pending_requests_count = handle.stats().pending_requests_count();
        assert_eq!(
            pending_requests_count, max_concurrent,
            "Backend should respect the max concurrent requests limit"
        );

        let ongoing_requests_count = handle.stats().queued_requests_count();
        assert_eq!(ongoing_requests_count, addresses.len() - max_concurrent,);
    }

    #[test]
    fn test_requests_are_processed_after_limit_freed() {
        // Start mock server with a predefined result
        let result = "0x456";
        let sender = start_mock_rpc_server("127.0.0.1:8092".to_string(), result.to_string());

        // Create a backend with a limit of 2 concurrent requests
        let max_concurrent = 2;
        let handle = create_forked_backend_with_config("http://127.0.0.1:8092", max_concurrent);
        let block_id = 1;

        // Send 4 different requests (more than the limit)
        let addresses = [felt!("0x1"), felt!("0x2"), felt!("0x3"), felt!("0x4")];
        for address in addresses {
            let h = handle.clone();
            std::thread::spawn(move || {
                let _ = h.get_nonce(address.into(), block_id);
            });
        }

        // Wait for requests to be queued
        std::thread::sleep(Duration::from_secs(1));

        // Initially should only have max_concurrent pending
        let pending_requests_count = handle.stats().pending_requests_count();
        assert_eq!(
            pending_requests_count, max_concurrent,
            "Backend should have max_concurrent ongoing requests"
        );

        // Send signals to complete the first batch of requests
        sender.send(()).unwrap();
        sender.send(()).unwrap();

        // Wait for the first batch to complete and next batch to start
        std::thread::sleep(Duration::from_secs(1));

        // Now the remaining requests should be processing
        let pending_requests_count = handle.stats().pending_requests_count();
        assert_eq!(
            pending_requests_count, max_concurrent,
            "Backend should process remaining queued requests up to the limit"
        );

        // Complete the remaining requests
        sender.send(()).unwrap();
        sender.send(()).unwrap();
    }
}
