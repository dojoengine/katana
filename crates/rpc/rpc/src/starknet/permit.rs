use std::sync::Arc;

use tokio::sync::Semaphore;

use crate::starknet::StarknetApiError;

/// `Permit` provides rate limiting functionality for compute-intensive RPC methods.
///
/// This type is specifically designed to limit concurrent executions of the
/// `starknet_estimateFee` RPC method, which is computationally expensive relative
/// to other methods. By limiting concurrent executions, we prevent potential
/// Denial of Service (DoS) attacks where an attacker could overwhelm the node
/// with many simultaneous fee estimation requests.
///
/// The implementation uses a tokio semaphore to track available permits. When
/// the semaphore is exhausted (all permits are in use), new requests will wait
/// and yield to the async runtime rather than blocking, allowing other RPC
/// methods to continue processing.
#[derive(Debug, Clone)]
pub struct Permit {
    /// The underlying semaphore that tracks available permits
    semaphore: Arc<Semaphore>,
}

impl Permit {
    /// Creates a new `Permit` with the specified number of concurrent permits.
    ///
    /// # Arguments
    ///
    /// * `permits` - The maximum number of concurrent operations allowed.
    pub fn new(permits: u32) -> Self {
        Self { semaphore: Arc::new(Semaphore::new(permits as usize)) }
    }

    /// Acquires a permit, waiting if necessary.
    ///
    /// This method will yield to the async runtime if no permits are currently
    /// available, rather than blocking the thread. The returned permit is automatically
    /// released when dropped, making it safe to use with async operations.
    ///
    /// # Returns
    ///
    /// * `Result<OwnedSemaphorePermit, StarknetApiError>` - A permit that will be released when
    ///   dropped, or an error if the semaphore was closed.
    pub async fn acquire(&self) -> Result<tokio::sync::OwnedSemaphorePermit, StarknetApiError> {
        self.semaphore.clone().acquire_owned().await.map_err(|_| {
            StarknetApiError::UnexpectedError {
                reason: "Failed to acquire estimate_fee semaphore permit".to_string(),
            }
        })
    }
}
