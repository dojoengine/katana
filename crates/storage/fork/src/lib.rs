use std::collections::hash_map::Entry;
use std::collections::{HashMap, VecDeque};
use std::fmt::Debug;
use std::pin::Pin;
use std::sync::mpsc::{
    channel as oneshot, Receiver as OneshotReceiver, RecvError, Sender as OneshotSender,
};
use std::sync::Arc;
use std::task::{Context, Poll};
use std::{io, thread};

use anyhow::anyhow;
use futures::channel::mpsc::{channel as async_channel, Receiver, SendError, Sender};
use futures::future::BoxFuture;
use futures::stream::Stream;
use futures::{Future, FutureExt};
use katana_primitives::block::{BlockHashOrNumber, BlockIdOrTag};
use katana_primitives::class::{
    ClassHash, CompiledClassHash, ComputeClassHashError, ContractClass,
    ContractClassCompilationError,
};
use katana_primitives::contract::{ContractAddress, Nonce, StorageKey, StorageValue};
use katana_rpc_client::starknet::{
    Client as StarknetClient, Error as StarknetClientError, StarknetApiError,
};
use katana_rpc_types::class::Class;
use parking_lot::Mutex;
use tracing::{error, trace};

const LOG_TARGET: &str = "forking::backend";

type BackendResult<T> = Result<T, BackendError>;

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
    Nonce(BackendResult<Nonce>),
    Storage(BackendResult<StorageValue>),
    ClassHashAt(BackendResult<ClassHash>),
    ClassAt(BackendResult<Class>),
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

/// The types of request that can be sent to [`Backend`].
///
/// Each request consists of a payload and the sender half of a oneshot channel that will be used
/// to send the result back to the backend handle.
enum BackendRequest {
    Nonce(Request<ContractAddress>),
    Class(Request<ClassHash>),
    ClassHash(Request<ContractAddress>),
    Storage(Request<(ContractAddress, StorageKey)>),
    // Test-only request kind for requesting the backend stats
    #[cfg(test)]
    Stats(OneshotSender<usize>),
}

impl BackendRequest {
    /// Create a new request for fetching the nonce of a contract.
    fn nonce(address: ContractAddress) -> (BackendRequest, OneshotReceiver<BackendResponse>) {
        let (sender, receiver) = oneshot();
        (BackendRequest::Nonce(Request { payload: address, sender }), receiver)
    }

    /// Create a new request for fetching the class definitions of a contract.
    fn class(hash: ClassHash) -> (BackendRequest, OneshotReceiver<BackendResponse>) {
        let (sender, receiver) = oneshot();
        (BackendRequest::Class(Request { payload: hash, sender }), receiver)
    }

    /// Create a new request for fetching the class hash of a contract.
    fn class_hash(address: ContractAddress) -> (BackendRequest, OneshotReceiver<BackendResponse>) {
        let (sender, receiver) = oneshot();
        (BackendRequest::ClassHash(Request { payload: address, sender }), receiver)
    }

    /// Create a new request for fetching the storage value of a contract.
    fn storage(
        address: ContractAddress,
        key: StorageKey,
    ) -> (BackendRequest, OneshotReceiver<BackendResponse>) {
        let (sender, receiver) = oneshot();
        (BackendRequest::Storage(Request { payload: (address, key), sender }), receiver)
    }

    #[cfg(test)]
    fn stats() -> (BackendRequest, OneshotReceiver<usize>) {
        let (sender, receiver) = oneshot();
        (BackendRequest::Stats(sender), receiver)
    }
}

type BackendRequestFuture = BoxFuture<'static, BackendResponse>;

// Identifier for pending requests.
// This is used for request deduplication.
#[derive(Eq, Hash, PartialEq, Clone, Copy, Debug)]
enum BackendRequestIdentifier {
    Nonce(ContractAddress),
    Class(ClassHash),
    ClassHash(ContractAddress),
    Storage((ContractAddress, StorageKey)),
}

/// The backend for the forked provider.
///
/// It is responsible for processing [requests](BackendRequest) to fetch data from the remote
/// provider.
pub struct Backend {
    /// The Starknet RPC provider that will be used to fetch data from.
    provider: Arc<StarknetClient>,
    // HashMap that keep track of current requests, for dedup purposes.
    request_dedup_map: HashMap<BackendRequestIdentifier, Vec<OneshotSender<BackendResponse>>>,
    /// Requests that are currently being poll.
    pending_requests: Vec<(BackendRequestIdentifier, BackendRequestFuture)>,
    /// Requests that are queued to be polled.
    queued_requests: VecDeque<BackendRequest>,
    /// A channel for receiving requests from the [BackendHandle]s.
    incoming: Receiver<BackendRequest>,
    /// Pinned block id for all requests.
    block_id: BlockHashOrNumber,
}

/////////////////////////////////////////////////////////////////
// Backend implementation
/////////////////////////////////////////////////////////////////

impl Backend {
    // TODO(kariy): create a `.start()` method start running the backend logic and let the users
    // choose which thread to running it on instead of spawning the thread ourselves.
    /// Create a new [Backend] with the given provider and block id, and returns a handle to it. The
    /// backend will start processing requests immediately upon creation.
    #[allow(clippy::new_ret_no_self)]
    pub fn new(
        provider: StarknetClient,
        block_id: BlockHashOrNumber,
    ) -> Result<BackendClient, BackendError> {
        let (handle, backend) = Self::new_inner(provider, block_id);

        thread::Builder::new()
            .name("forking-backend".into())
            .spawn(move || {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("failed to create tokio runtime")
                    .block_on(backend);
            })
            .map_err(|e| BackendError::BackendThreadInit(Arc::new(e)))?;

        trace!(target: LOG_TARGET, "Forking backend started.");

        Ok(handle)
    }

    fn new_inner(
        provider: StarknetClient,
        block_id: BlockHashOrNumber,
    ) -> (BackendClient, Backend) {
        // Create async channel to receive requests from the handle.
        let (tx, rx) = async_channel(100);
        let backend = Backend {
            block_id,
            incoming: rx,
            provider: Arc::new(provider),
            request_dedup_map: HashMap::new(),
            pending_requests: Vec::new(),
            queued_requests: VecDeque::new(),
        };

        (BackendClient(Mutex::new(tx)), backend)
    }

    /// This method is responsible for transforming the incoming request
    /// sent from a [BackendHandle] into a RPC request to the remote network.
    fn handle_requests(&mut self, request: BackendRequest) {
        let block_id = BlockIdOrTag::from(self.block_id);
        let provider = self.provider.clone();

        // Check if there are similar requests in the queue before sending the request
        match request {
            BackendRequest::Nonce(Request { payload, sender }) => {
                let req_key = BackendRequestIdentifier::Nonce(payload);

                self.dedup_request(
                    req_key,
                    sender,
                    Box::pin(async move {
                        let res = provider
                            .get_nonce(block_id, payload)
                            .await
                            .map_err(|e| BackendError::StarknetProvider(Arc::new(e)));

                        BackendResponse::Nonce(res)
                    }),
                );
            }

            BackendRequest::Storage(Request { payload: (addr, key), sender }) => {
                let req_key = BackendRequestIdentifier::Storage((addr, key));

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

            BackendRequest::ClassHash(Request { payload, sender }) => {
                let req_key = BackendRequestIdentifier::ClassHash(payload);

                self.dedup_request(
                    req_key,
                    sender,
                    Box::pin(async move {
                        let res = provider
                            .get_class_hash_at(block_id, payload)
                            .await
                            .map_err(|e| BackendError::StarknetProvider(Arc::new(e)));

                        BackendResponse::ClassHashAt(res)
                    }),
                );
            }

            BackendRequest::Class(Request { payload, sender }) => {
                let req_key = BackendRequestIdentifier::Class(payload);

                self.dedup_request(
                    req_key,
                    sender,
                    Box::pin(async move {
                        let res = provider
                            .get_class(block_id, payload)
                            .await
                            .map_err(|e| BackendError::StarknetProvider(Arc::new(e)));

                        BackendResponse::ClassAt(res)
                    }),
                );
            }

            #[cfg(test)]
            BackendRequest::Stats(sender) => {
                let total_ongoing_request = self.pending_requests.len();
                sender.send(total_ongoing_request).expect("failed to send backend stats");
            }
        }
    }

    fn dedup_request(
        &mut self,
        req_key: BackendRequestIdentifier,
        sender: OneshotSender<BackendResponse>,
        rpc_call_future: BoxFuture<'static, BackendResponse>,
    ) {
        if let Entry::Vacant(e) = self.request_dedup_map.entry(req_key) {
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
                    error!(target: LOG_TARGET, "failed to get current request dedup vector");
                }
            }
        }
    }
}

impl Future for Backend {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let pin = self.get_mut();
        loop {
            // convert all queued requests into futures to be polled
            while let Some(req) = pin.queued_requests.pop_front() {
                pin.handle_requests(req);
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
                            sender.send(res.clone()).unwrap_or_else(|error| {
                            	error!(target: LOG_TARGET, key = ?fut_key, %error, "Failed to send result.")
                            });
                        });

                        pin.request_dedup_map.remove(&fut_key);
                    }
                }
            }

            // if no queued requests, then yield
            if pin.queued_requests.is_empty() {
                return Poll::Pending;
            }
        }
    }
}

impl Debug for Backend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Backend")
            .field("provider", &self.provider)
            .field("request_dedup_map", &self.request_dedup_map)
            .field("pending_requests", &self.pending_requests.len())
            .field("queued_requests", &self.queued_requests.len())
            .field("incoming", &self.incoming)
            .field("block", &self.block_id)
            .finish()
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

/// A thread safe handler to [`Backend`].
///
/// This is the primary interface for sending request to the backend to fetch data from the remote
/// network.
#[derive(Debug)]
pub struct BackendClient(Mutex<Sender<BackendRequest>>);

impl Clone for BackendClient {
    fn clone(&self) -> Self {
        Self(Mutex::new(self.0.lock().clone()))
    }
}

/////////////////////////////////////////////////////////////////
// BackendHandle implementation
/////////////////////////////////////////////////////////////////

impl BackendClient {
    pub fn get_nonce(&self, address: ContractAddress) -> Result<Option<Nonce>, BackendClientError> {
        trace!(target: LOG_TARGET, %address, "Requesting contract nonce.");
        let (req, rx) = BackendRequest::nonce(address);
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
    ) -> Result<Option<StorageValue>, BackendClientError> {
        trace!(target: LOG_TARGET, %address, key = %format!("{key:#x}"), "Requesting contract storage.");
        let (req, rx) = BackendRequest::storage(address, key);
        self.request(req)?;
        match rx.recv()? {
            BackendResponse::Storage(res) => handle_not_found_err(res),
            response => Err(BackendClientError::UnexpectedResponse(anyhow!("{response:?}"))),
        }
    }

    pub fn get_class_hash_at(
        &self,
        address: ContractAddress,
    ) -> Result<Option<ClassHash>, BackendClientError> {
        trace!(target: LOG_TARGET, %address, "Requesting contract class hash.");
        let (req, rx) = BackendRequest::class_hash(address);
        self.request(req)?;
        match rx.recv()? {
            BackendResponse::ClassHashAt(res) => handle_not_found_err(res),
            response => Err(BackendClientError::UnexpectedResponse(anyhow!("{response:?}"))),
        }
    }

    pub fn get_class_at(
        &self,
        class_hash: ClassHash,
    ) -> Result<Option<ContractClass>, BackendClientError> {
        trace!(target: LOG_TARGET, class_hash = %format!("{class_hash:#x}"), "Requesting class.");
        let (req, rx) = BackendRequest::class(class_hash);
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
    ) -> Result<Option<CompiledClassHash>, BackendClientError> {
        trace!(target: LOG_TARGET, class_hash = %format!("{class_hash:#x}"), "Requesting compiled class hash.");
        if let Some(class) = self.get_class_at(class_hash)? {
            let class = class.compile()?;
            Ok(Some(class.class_hash()?))
        } else {
            Ok(None)
        }
    }

    /// Send a request to the backend thread.
    fn request(&self, req: BackendRequest) -> Result<(), BackendClientError> {
        self.0.lock().try_send(req).map_err(|e| e.into_send_error())?;
        Ok(())
    }

    #[cfg(test)]
    fn stats(&self) -> Result<usize, BackendClientError> {
        let (req, rx) = BackendRequest::stats();
        self.request(req)?;
        Ok(rx.recv()?)
    }
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

    use katana_primitives::block::BlockNumber;
    use katana_rpc_client::HttpClientBuilder;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use url::Url;

    use super::*;

    pub fn create_forked_backend(rpc_url: &str, block_num: BlockNumber) -> BackendClient {
        let url = Url::parse(rpc_url).expect("valid url");
        let provider = StarknetClient::new(HttpClientBuilder::new().build(url).unwrap());
        Backend::new(provider, block_num.into()).unwrap()
    }

    // Starts a TCP server that never close the connection.
    pub fn start_tcp_server(addr: String) {
        use tokio::runtime::Builder;

        let (tx, rx) = sync_channel::<()>(1);
        thread::spawn(move || {
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
    pub fn start_mock_rpc_server(addr: String, response: String) -> SyncSender<()> {
        use tokio::runtime::Builder;
        let (tx, rx) = sync_channel::<()>(1);

        thread::spawn(move || {
            Builder::new_current_thread().enable_all().build().unwrap().block_on(async move {
                let listener = TcpListener::bind(addr).await.unwrap();

                loop {
                    let (mut socket, _) = listener.accept().await.unwrap();

                    // Read the request, so hyper would not close the connection
                    let mut buffer = [0; 1024];
                    let n = socket.read(&mut buffer).await.unwrap();

                    // Parse the HTTP request to extract the JSON-RPC body
                    let request_str = String::from_utf8_lossy(&buffer[..n]);
                    let request_id = if let Some(body_start) = request_str.rfind("\r\n\r\n") {
                        let body = &request_str[body_start + 4..];

                        // Parse the JSON-RPC request using serde_json
                        if let Ok(json_request) = serde_json::from_str::<serde_json::Value>(body) {
                            // Extract the ID from the parsed JSON
                            json_request
                                .get("id")
                                .and_then(|id| {
                                    if id.is_string() {
                                        id.as_str().map(|s| format!("\"{}\"", s))
                                    } else {
                                        Some(id.to_string())
                                    }
                                })
                                .unwrap_or_else(|| "1".to_string())
                        } else {
                            "1".to_string()
                        }
                    } else {
                        "1".to_string()
                    };

                    // Wait for a signal to return the response.
                    rx.recv().unwrap();

                    // Parse the response template and update with correct ID
                    let json_response = if let Ok(mut response_json) =
                        serde_json::from_str::<serde_json::Value>(&response)
                    {
                        // Update the ID in the response
                        if let Ok(id_value) = serde_json::from_str::<serde_json::Value>(&request_id)
                        {
                            response_json["id"] = id_value;
                        }
                        serde_json::to_string(&response_json).unwrap_or(response.clone())
                    } else {
                        response.clone()
                    };

                    // After reading, we send the response with correct ID
                    let http_response = format!(
                        "HTTP/1.1 200 OK\r\ncontent-length: {}\r\ncontent-type: \
                         application/json\r\n\r\n{}",
                        json_response.len(),
                        json_response
                    );

                    socket.write_all(http_response.as_bytes()).await.unwrap();
                    socket.flush().await.unwrap();
                }
            });
        });

        // Returning the sender to allow controlling the response timing.
        tx
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
    const ERROR_STATS: &str = "Failed to get stats";

    #[test]
    fn handle_incoming_requests() {
        // start a mock remote network
        start_tcp_server("127.0.0.1:8080".to_string());

        let handle = create_forked_backend("http://127.0.0.1:8080", 1);

        // check no pending requests
        let stats = handle.stats().expect(ERROR_STATS);
        assert_eq!(stats, 0, "Backend should not have any ongoing requests.");

        // send requests to the backend
        let h1 = handle.clone();
        thread::spawn(move || {
            h1.get_nonce(felt!("0x1").into()).expect(ERROR_SEND_REQUEST);
        });
        let h2 = handle.clone();
        thread::spawn(move || {
            h2.get_class_at(felt!("0x1")).expect(ERROR_SEND_REQUEST);
        });
        let h3 = handle.clone();
        thread::spawn(move || {
            h3.get_compiled_class_hash(felt!("0x2")).expect(ERROR_SEND_REQUEST);
        });
        let h4 = handle.clone();
        thread::spawn(move || {
            h4.get_class_hash_at(felt!("0x1").into()).expect(ERROR_SEND_REQUEST);
        });
        let h5 = handle.clone();
        thread::spawn(move || {
            h5.get_storage(felt!("0x1").into(), felt!("0x1")).expect(ERROR_SEND_REQUEST);
        });

        // wait for the requests to be handled
        thread::sleep(Duration::from_secs(1));

        // check request are handled
        let stats = handle.stats().expect(ERROR_STATS);
        assert_eq!(stats, 5, "Backend should have 5 ongoing requests.")
    }

    #[test]
    fn get_nonce_request_should_be_deduplicated() {
        // start a mock remote network
        start_tcp_server("127.0.0.1:8081".to_string());

        let handle = create_forked_backend("http://127.0.0.1:8081", 1);

        // check no pending requests
        let stats = handle.stats().expect(ERROR_STATS);
        assert_eq!(stats, 0, "Backend should not have any ongoing requests.");

        // send requests to the backend
        let h1 = handle.clone();
        thread::spawn(move || {
            h1.get_nonce(felt!("0x1").into()).expect(ERROR_SEND_REQUEST);
        });
        let h2 = handle.clone();
        thread::spawn(move || {
            h2.get_nonce(felt!("0x1").into()).expect(ERROR_SEND_REQUEST);
        });

        // wait for the requests to be handled
        thread::sleep(Duration::from_secs(1));

        // check current request count
        let stats = handle.stats().expect(ERROR_STATS);
        assert_eq!(stats, 1, "Backend should have 1 ongoing requests.");

        // Different request, should be counted
        let h3 = handle.clone();
        thread::spawn(move || {
            h3.get_nonce(felt!("0x2").into()).expect(ERROR_SEND_REQUEST);
        });

        // wait for the requests to be handled
        thread::sleep(Duration::from_secs(1));

        // check request are handled
        let stats = handle.stats().expect(ERROR_STATS);
        assert_eq!(stats, 2, "Backend should only have 2 ongoing requests.")
    }

    #[test]
    fn get_class_at_request_should_be_deduplicated() {
        // start a mock remote network
        start_tcp_server("127.0.0.1:8082".to_string());

        let handle = create_forked_backend("http://127.0.0.1:8082", 1);

        // check no pending requests
        let stats = handle.stats().expect(ERROR_STATS);
        assert_eq!(stats, 0, "Backend should not have any ongoing requests.");

        // send requests to the backend
        let h1 = handle.clone();
        thread::spawn(move || {
            h1.get_class_at(felt!("0x1")).expect(ERROR_SEND_REQUEST);
        });
        let h2 = handle.clone();
        thread::spawn(move || {
            h2.get_class_at(felt!("0x1")).expect(ERROR_SEND_REQUEST);
        });

        // wait for the requests to be handled
        thread::sleep(Duration::from_secs(1));

        // check current request count
        let stats = handle.stats().expect(ERROR_STATS);
        assert_eq!(stats, 1, "Backend should have 1 ongoing requests.");

        // Different request, should be counted
        let h3 = handle.clone();
        thread::spawn(move || {
            h3.get_class_at(felt!("0x2")).expect(ERROR_SEND_REQUEST);
        });

        // wait for the requests to be handled
        thread::sleep(Duration::from_secs(1));

        // check request are handled
        let stats = handle.stats().expect(ERROR_STATS);
        assert_eq!(stats, 2, "Backend should only have 2 ongoing requests.")
    }

    #[test]
    fn get_compiled_class_hash_request_should_be_deduplicated() {
        // start a mock remote network
        start_tcp_server("127.0.0.1:8083".to_string());

        let handle = create_forked_backend("http://127.0.0.1:8083", 1);

        // check no pending requests
        let stats = handle.stats().expect(ERROR_STATS);
        assert_eq!(stats, 0, "Backend should not have any ongoing requests.");

        // send requests to the backend
        let h1 = handle.clone();
        thread::spawn(move || {
            h1.get_compiled_class_hash(felt!("0x1")).expect(ERROR_SEND_REQUEST);
        });
        let h2 = handle.clone();
        thread::spawn(move || {
            h2.get_compiled_class_hash(felt!("0x1")).expect(ERROR_SEND_REQUEST);
        });

        // wait for the requests to be handled
        thread::sleep(Duration::from_secs(1));

        // check current request count
        let stats = handle.stats().expect(ERROR_STATS);
        assert_eq!(stats, 1, "Backend should have 1 ongoing requests.");

        // Different request, should be counted
        let h3 = handle.clone();
        thread::spawn(move || {
            h3.get_compiled_class_hash(felt!("0x2")).expect(ERROR_SEND_REQUEST);
        });

        // wait for the requests to be handled
        thread::sleep(Duration::from_secs(1));

        // check request are handled
        let stats = handle.stats().expect(ERROR_STATS);
        assert_eq!(stats, 2, "Backend should only have 2 ongoing requests.")
    }

    #[test]
    fn get_class_at_and_get_compiled_class_hash_request_should_be_deduplicated() {
        // start a mock remote network
        start_tcp_server("127.0.0.1:8084".to_string());

        let handle = create_forked_backend("http://127.0.0.1:8084", 1);

        // check no pending requests
        let stats = handle.stats().expect(ERROR_STATS);
        assert_eq!(stats, 0, "Backend should not have any ongoing requests.");

        // send requests to the backend
        let h1 = handle.clone();
        thread::spawn(move || {
            h1.get_class_at(felt!("0x1")).expect(ERROR_SEND_REQUEST);
        });
        // Since this also calls to the same request as the previous one, it should be deduped
        let h2 = handle.clone();
        thread::spawn(move || {
            h2.get_compiled_class_hash(felt!("0x1")).expect(ERROR_SEND_REQUEST);
        });

        // wait for the requests to be handled
        thread::sleep(Duration::from_secs(1));

        // check current request count
        let stats = handle.stats().expect(ERROR_STATS);
        assert_eq!(stats, 1, "Backend should have 1 ongoing requests.");

        // Different request, should be counted
        let h3 = handle.clone();
        thread::spawn(move || {
            h3.get_class_at(felt!("0x2")).expect(ERROR_SEND_REQUEST);
        });

        // wait for the requests to be handled
        thread::sleep(Duration::from_secs(1));

        // check request are handled
        let stats = handle.stats().expect(ERROR_STATS);
        assert_eq!(stats, 2, "Backend should only have 2 ongoing requests.")
    }

    #[test]
    fn get_class_hash_at_request_should_be_deduplicated() {
        // start a mock remote network
        start_tcp_server("127.0.0.1:8085".to_string());

        let handle = create_forked_backend("http://127.0.0.1:8085", 1);

        // check no pending requests
        let stats = handle.stats().expect(ERROR_STATS);
        assert_eq!(stats, 0, "Backend should not have any ongoing requests.");

        // send requests to the backend
        let h1 = handle.clone();
        thread::spawn(move || {
            h1.get_class_hash_at(felt!("0x1").into()).expect(ERROR_SEND_REQUEST);
        });
        let h2 = handle.clone();
        thread::spawn(move || {
            h2.get_class_hash_at(felt!("0x1").into()).expect(ERROR_SEND_REQUEST);
        });

        // wait for the requests to be handled
        thread::sleep(Duration::from_secs(1));

        // check current request count
        let stats = handle.stats().expect(ERROR_STATS);
        assert_eq!(stats, 1, "Backend should have 1 ongoing requests.");

        // Different request, should be counted
        let h3 = handle.clone();
        thread::spawn(move || {
            h3.get_class_hash_at(felt!("0x2").into()).expect(ERROR_SEND_REQUEST);
        });

        // wait for the requests to be handled
        thread::sleep(Duration::from_secs(1));

        // check request are handled
        let stats = handle.stats().expect(ERROR_STATS);
        assert_eq!(stats, 2, "Backend should only have 2 ongoing requests.")
    }

    #[test]
    fn get_storage_request_should_be_deduplicated() {
        // start a mock remote network
        start_tcp_server("127.0.0.1:8086".to_string());

        let handle = create_forked_backend("http://127.0.0.1:8086", 1);

        // check no pending requests
        let stats = handle.stats().expect(ERROR_STATS);
        assert_eq!(stats, 0, "Backend should not have any ongoing requests.");

        // send requests to the backend
        let h1 = handle.clone();
        thread::spawn(move || {
            h1.get_storage(felt!("0x1").into(), felt!("0x1")).expect(ERROR_SEND_REQUEST);
        });
        let h2 = handle.clone();
        thread::spawn(move || {
            h2.get_storage(felt!("0x1").into(), felt!("0x1")).expect(ERROR_SEND_REQUEST);
        });

        // wait for the requests to be handled
        thread::sleep(Duration::from_secs(1));

        // check current request count
        let stats = handle.stats().expect(ERROR_STATS);
        assert_eq!(stats, 1, "Backend should have 1 ongoing requests.");

        // Different request, should be counted
        let h3 = handle.clone();
        thread::spawn(move || {
            h3.get_storage(felt!("0x2").into(), felt!("0x3")).expect(ERROR_SEND_REQUEST);
        });

        // wait for the requests to be handled
        thread::sleep(Duration::from_secs(1));

        // check request are handled
        let stats = handle.stats().expect(ERROR_STATS);
        assert_eq!(stats, 2, "Backend should only have 2 ongoing requests.")
    }

    #[test]
    fn get_storage_request_on_same_address_with_different_key_should_be_deduplicated() {
        // start a mock remote network
        start_tcp_server("127.0.0.1:8087".to_string());

        let handle = create_forked_backend("http://127.0.0.1:8087", 1);

        // check no pending requests
        let stats = handle.stats().expect(ERROR_STATS);
        assert_eq!(stats, 0, "Backend should not have any ongoing requests.");

        // send requests to the backend
        let h1 = handle.clone();
        thread::spawn(move || {
            h1.get_storage(felt!("0x1").into(), felt!("0x1")).expect(ERROR_SEND_REQUEST);
        });
        let h2 = handle.clone();
        thread::spawn(move || {
            h2.get_storage(felt!("0x1").into(), felt!("0x1")).expect(ERROR_SEND_REQUEST);
        });

        // wait for the requests to be handled
        thread::sleep(Duration::from_secs(1));

        // check current request count
        let stats = handle.stats().expect(ERROR_STATS);
        assert_eq!(stats, 1, "Backend should have 1 ongoing requests.");

        // Different request, should be counted
        let h3 = handle.clone();
        thread::spawn(move || {
            h3.get_storage(felt!("0x1").into(), felt!("0x3")).expect(ERROR_SEND_REQUEST);
        });
        // Different request, should be counted
        let h4 = handle.clone();
        thread::spawn(move || {
            h4.get_storage(felt!("0x1").into(), felt!("0x6")).expect(ERROR_SEND_REQUEST);
        });

        // wait for the requests to be handled
        thread::sleep(Duration::from_secs(1));

        // check current request count
        let stats = handle.stats().expect(ERROR_STATS);
        assert_eq!(stats, 3, "Backend should have 3 ongoing requests.");

        // Same request as the last one, shouldn't be counted
        let h5 = handle.clone();
        thread::spawn(move || {
            h5.get_storage(felt!("0x1").into(), felt!("0x6")).expect(ERROR_SEND_REQUEST);
        });

        // wait for the requests to be handled
        thread::sleep(Duration::from_secs(1));

        // check request are handled
        let stats = handle.stats().expect(ERROR_STATS);
        assert_eq!(stats, 3, "Backend should only have 3 ongoing requests.")
    }

    #[test]
    fn test_deduplicated_request_should_return_similar_results() {
        // Start mock server with a predefined nonce response
        let response = r#"{"jsonrpc":"2.0","result":"0x123","id":1}"#;
        let sender = start_mock_rpc_server("127.0.0.1:8090".to_string(), response.to_string());

        let handle = create_forked_backend("http://127.0.0.1:8090", 1);
        let addr = ContractAddress(felt!("0x1"));

        // Collect results from multiple identical nonce requests
        let results: Arc<Mutex<Vec<_>>> = Arc::new(Mutex::new(Vec::new()));

        let handles: Vec<_> = (0..5)
            .map(|_| {
                let h = handle.clone();
                let results = results.clone();
                thread::spawn(move || {
                    let res = h.get_nonce(addr);
                    results.lock().unwrap().push(res);
                })
            })
            .collect();

        // wait for the requests to be sent to the rpc server
        thread::sleep(Duration::from_secs(1));

        // Check that there's only one request, meaning it is deduplicated.
        let stats = handle.stats().expect(ERROR_STATS);
        assert_eq!(stats, 1, "Backend should only have 1 ongoing requests.");

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
}
