use std::error::Error as StdError;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use backon::{ExponentialBuilder, Retryable};
use katana_feeder_gateway::client::{self, SequencerGateway};
use tracing::warn;

/// Trait for types that can be fetched from the sequencer gateway.
///
/// This trait abstracts the specific fetching logic, allowing the generic `Downloader`
/// to work with different data types (blocks, classes, etc.).
#[async_trait::async_trait]
pub trait Fetchable: Sized + Send {
    /// The key type used to identify what to fetch (e.g., BlockNumber, ClassHash).
    type Key: Send + Sync;

    /// The error type that can occur during fetching.
    type Error: From<client::Error> + std::error::Error + Send;

    /// Fetch a single item from the sequencer gateway.
    async fn fetch(client: &SequencerGateway, key: Self::Key) -> Result<Self, Self::Error>;

    /// Returns the minimum delay for retry backoff in seconds.
    fn retry_min_delay_secs() -> u64 {
        3
    }
}

/// A generic downloader that fetches data from the sequencer gateway in batches.
#[derive(Debug, Clone)]
pub struct Downloader<T> {
    batch_size: usize,
    client: Arc<SequencerGateway>,
    _phantom: std::marker::PhantomData<T>,
}

impl<T: Fetchable> Downloader<T> {
    pub fn new(client: SequencerGateway, batch_size: usize) -> Self {
        Self { client: Arc::new(client), batch_size, _phantom: std::marker::PhantomData }
    }

    /// Download items in batches.
    pub async fn download(&self, keys: &[T::Key]) -> Result<Vec<T>, T::Error>
    where
        T::Key: Clone,
    {
        let mut items = Vec::with_capacity(keys.len());

        for chunk in keys.chunks(self.batch_size) {
            let batch = self.fetch_with_retry(chunk).await?;
            items.extend(batch);
        }

        Ok(items)
    }

    /// Fetch items with retry mechanism at a batch level.
    async fn fetch_with_retry(&self, keys: &[T::Key]) -> Result<Vec<T>, T::Error>
    where
        T::Key: Clone,
    {
        let client = Arc::clone(&self.client);
        let batch_size = self.batch_size;
        let keys = keys.to_vec();
        let request = move || {
            let downloader = Self {
                client: Arc::clone(&client),
                batch_size,
                _phantom: std::marker::PhantomData,
            };
            let keys = keys.clone();
            async move { downloader.fetch_batch(&keys).await }
        };

        // Retry only when being rate limited
        let backoff = ExponentialBuilder::default()
            .with_min_delay(Duration::from_secs(T::retry_min_delay_secs()));
        let result = request
            .retry(backoff)
            .when(|error: &T::Error| {
                // Check if the error is a rate limit error by downcasting
                if let Some(gateway_error) =
                    StdError::source(error).and_then(|e| e.downcast_ref::<client::Error>())
                {
                    matches!(gateway_error, client::Error::RateLimited)
                } else {
                    false
                }
            })
            .notify(|error, _| {
                // Use the pipeline target for retry logging
                warn!(target: "pipeline", %error, "Retrying download.");
            })
            .await?;

        Ok(result)
    }

    /// Fetch a batch of items concurrently.
    async fn fetch_batch(&self, keys: &[T::Key]) -> Result<Vec<T>, T::Error>
    where
        T::Key: Clone,
    {
        let mut requests = Vec::with_capacity(keys.len());

        for key in keys {
            let client = Arc::clone(&self.client);
            let key = key.clone();
            requests.push(async move { T::fetch(&client, key).await });
        }

        let results = futures::future::join_all(requests).await;
        results.into_iter().collect()
    }
}
