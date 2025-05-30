use std::sync::Arc;

pub use tokio::sync::AcquireError;
use tokio::sync::Semaphore;

/// `Permits` provides rate limiting functionality for compute-intensive RPC methods.
///
/// The implementation uses a tokio semaphore to track available permits. When
/// the semaphore is exhausted (all permits are in use), new requests will wait
/// and yield to the async runtime rather than blocking, allowing other RPC
/// methods to continue processing.
#[derive(Debug, Clone)]
pub struct Permits {
    semaphore: Arc<Semaphore>,
}

impl Permits {
    /// Creates a new `Permits` with the specified number of concurrent permits.
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
    pub async fn acquire(&self) -> Result<AcquiredPermit, AcquireError> {
        self.semaphore.clone().acquire_owned().await.map(AcquiredPermit)
    }
}

/// An acquired permit.
///
/// This type is created by the [`acquire`] method.
///
/// [`acquire`]: Permits::acquire()
#[must_use]
#[clippy::has_significant_drop]
#[allow(dead_code)]
#[derive(Debug)]
pub struct AcquiredPermit(tokio::sync::OwnedSemaphorePermit);
