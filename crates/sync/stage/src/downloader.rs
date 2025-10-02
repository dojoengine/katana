use std::error::Error as StdError;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use backon::{BackoffBuilder, ExponentialBuilder, Retryable};
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
}

/// Configuration for retry behavior in the downloader.
#[derive(Debug, Clone)]
struct RetryConfig {
    /// Minimum delay between retries in seconds.
    min_delay_secs: u64,
    /// Maximum delay between retries in seconds (optional).
    max_delay_secs: Option<u64>,
    /// Maximum number of retry attempts (None for unlimited).
    max_attempts: Option<usize>,
}

impl RetryConfig {
    /// Create a new retry configuration with default values.
    fn new() -> Self {
        Self { min_delay_secs: 3, max_delay_secs: None, max_attempts: None }
    }

    /// Create an exponential backoff builder from this configuration.
    fn to_backoff_builder(&self) -> impl BackoffBuilder {
        let mut builder =
            ExponentialBuilder::default().with_min_delay(Duration::from_secs(self.min_delay_secs));

        if let Some(max_delay) = self.max_delay_secs {
            builder = builder.with_max_delay(Duration::from_secs(max_delay));
        }

        if let Some(max_attempts) = self.max_attempts {
            builder = builder.with_max_times(max_attempts);
        }

        builder
    }
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self::new()
    }
}

/// A generic downloader that fetches data from the sequencer gateway in batches.
#[derive(Debug, Clone)]
pub struct Downloader<T> {
    batch_size: usize,
    client: Arc<SequencerGateway>,
    retry_config: RetryConfig,
    _phantom: std::marker::PhantomData<T>,
}

impl<T: Fetchable> Downloader<T> {
    /// Create a builder for configuring a new downloader.
    pub fn builder(client: SequencerGateway) -> DownloaderBuilder<T> {
        DownloaderBuilder::new(client)
    }

    /// Get the batch size.
    #[cfg(test)]
    pub fn batch_size(&self) -> usize {
        self.batch_size
    }

    /// Get the retry configuration's min delay.
    #[cfg(test)]
    pub fn retry_min_delay_secs(&self) -> u64 {
        self.retry_config.min_delay_secs
    }

    /// Get the retry configuration's max attempts.
    #[cfg(test)]
    pub fn retry_max_attempts(&self) -> Option<usize> {
        self.retry_config.max_attempts
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
        let retry_config = self.retry_config.clone();
        let keys = keys.to_vec();
        let request = move || {
            let downloader = Self {
                client: Arc::clone(&client),
                batch_size,
                retry_config: retry_config.clone(),
                _phantom: std::marker::PhantomData,
            };
            let keys = keys.clone();
            async move { downloader.fetch_batch(&keys).await }
        };

        // Retry only when being rate limited
        let backoff = self.retry_config.to_backoff_builder();
        let result = request
            .retry(backoff)
            .when(Self::is_retryable_error)
            .notify(|error, _attempt| {
                // Use the pipeline target for retry logging
                warn!(target: "pipeline", %error, "Retrying download.");
            })
            .await?;

        Ok(result)
    }

    /// Determine if an error is retryable (rate limit errors).
    fn is_retryable_error(error: &T::Error) -> bool {
        if let Some(gateway_error) =
            StdError::source(error).and_then(|e| e.downcast_ref::<client::Error>())
        {
            matches!(gateway_error, client::Error::RateLimited)
        } else {
            false
        }
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

/// Builder for configuring and creating a `Downloader`.
///
/// This builder encapsulates all retry configuration options and provides
/// a clean, fluent API for constructing downloaders.
pub struct DownloaderBuilder<T> {
    client: SequencerGateway,
    batch_size: usize,
    min_delay_secs: u64,
    max_delay_secs: Option<u64>,
    max_attempts: Option<usize>,
    _phantom: std::marker::PhantomData<T>,
}

impl<T: Fetchable> DownloaderBuilder<T> {
    /// Create a new builder with default configuration.
    fn new(client: SequencerGateway) -> Self {
        Self {
            client,
            batch_size: 10,
            min_delay_secs: 3,
            max_delay_secs: None,
            max_attempts: None,
            _phantom: std::marker::PhantomData,
        }
    }

    /// Set the batch size for downloading items.
    ///
    /// The downloader will fetch this many items concurrently in each batch.
    pub fn batch_size(mut self, size: usize) -> Self {
        self.batch_size = size;
        self
    }

    /// Set the minimum delay between retries in seconds.
    ///
    /// This is the initial delay that will be used with exponential backoff.
    pub fn min_retry_delay_secs(mut self, secs: u64) -> Self {
        self.min_delay_secs = secs;
        self
    }

    /// Set the maximum delay between retries in seconds.
    ///
    /// This caps the exponential backoff at the specified duration.
    pub fn max_retry_delay_secs(mut self, secs: u64) -> Self {
        self.max_delay_secs = Some(secs);
        self
    }

    /// Set the maximum number of retry attempts.
    ///
    /// After this many failed attempts, the download will fail permanently.
    /// If not set, retries are unlimited.
    pub fn max_retry_attempts(mut self, attempts: usize) -> Self {
        self.max_attempts = Some(attempts);
        self
    }

    /// Build the configured downloader.
    pub fn build(self) -> Downloader<T> {
        let retry_config = RetryConfig {
            min_delay_secs: self.min_delay_secs,
            max_delay_secs: self.max_delay_secs,
            max_attempts: self.max_attempts,
        };

        Downloader {
            client: Arc::new(self.client),
            batch_size: self.batch_size,
            retry_config,
            _phantom: std::marker::PhantomData,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    use katana_feeder_gateway::client::SequencerGateway;
    use katana_primitives::block::BlockNumber;

    use super::*;

    // Test error type that wraps client errors
    #[derive(Debug, thiserror::Error)]
    enum TestError {
        #[error(transparent)]
        Gateway(#[from] client::Error),
        #[error("test error: {0}")]
        Custom(String),
    }

    // Test data type for fetching
    #[derive(Debug, Clone, PartialEq)]
    struct TestData {
        id: BlockNumber,
        value: String,
    }

    // Mock fetchable implementation with controllable behavior
    struct MockFetchable {
        // Counter to track number of fetch attempts
        pub fetch_count: Arc<AtomicUsize>,
        // Control whether to return rate limit error
        pub should_rate_limit: Arc<Mutex<Vec<bool>>>,
    }

    impl MockFetchable {
        fn new() -> Self {
            Self {
                fetch_count: Arc::new(AtomicUsize::new(0)),
                should_rate_limit: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn with_rate_limit_sequence(rate_limits: Vec<bool>) -> Self {
            Self {
                fetch_count: Arc::new(AtomicUsize::new(0)),
                should_rate_limit: Arc::new(Mutex::new(rate_limits)),
            }
        }

        fn get_fetch_count(&self) -> usize {
            self.fetch_count.load(Ordering::SeqCst)
        }
    }

    // Note: We cannot implement Fetchable for MockFetchable directly because
    // Fetchable requires a reference to SequencerGateway. Instead, we would need
    // to use a mock gateway client or integration tests with a test server.

    #[test]
    fn test_downloader_builder() {
        // Create a dummy client for testing configuration
        let client = SequencerGateway::new(
            url::Url::parse("http://localhost:9545").unwrap(),
            url::Url::parse("http://localhost:9545/feeder_gateway").unwrap(),
        );

        let downloader: Downloader<TestData> = Downloader::builder(client)
            .batch_size(10)
            .min_retry_delay_secs(1)
            .max_retry_delay_secs(30)
            .max_retry_attempts(5)
            .build();

        assert_eq!(downloader.batch_size(), 10);
        assert_eq!(downloader.retry_min_delay_secs(), 1);
        assert_eq!(downloader.retry_max_attempts(), Some(5));
    }

    #[test]
    fn test_downloader_builder_defaults() {
        let client = SequencerGateway::new(
            url::Url::parse("http://localhost:9545").unwrap(),
            url::Url::parse("http://localhost:9545/feeder_gateway").unwrap(),
        );

        let downloader: Downloader<TestData> = Downloader::builder(client).build();

        assert_eq!(downloader.batch_size(), 10);
        assert_eq!(downloader.retry_min_delay_secs(), 3);
        assert_eq!(downloader.retry_max_attempts(), None);
    }

    #[test]
    fn test_is_retryable_error() {
        // Test that rate limit errors are retryable
        let rate_limit_error: TestError = client::Error::RateLimited.into();
        assert!(Downloader::<TestData>::is_retryable_error(&rate_limit_error));

        // Test that other errors are not retryable
        let other_error = TestError::Custom("not a rate limit".to_string());
        assert!(!Downloader::<TestData>::is_retryable_error(&other_error));
    }

    // Integration test would require a mock server or test gateway
    // Example structure for integration test:
    //
    // #[tokio::test]
    // async fn test_retry_on_rate_limit() {
    //     // Setup: Create a mock server that returns rate limit error first, then success
    //     let mock_server = MockServer::start().await;
    //     // ... configure mock responses
    //
    //     let client = SequencerGateway::new(mock_server.uri(), mock_server.uri());
    //     let retry_config = RetryConfig::new().with_min_delay_secs(0).with_max_attempts(3);
    //     let downloader = Downloader::with_retry_config(client, 5, retry_config);
    //
    //     let result = downloader.download(&[1, 2, 3]).await;
    //     assert!(result.is_ok());
    //     // Assert that the mock server received multiple requests (retries)
    // }
}

// Note: For proper testing of retry behavior, you would want to:
// 1. Create a mock HTTP server (using wiremock or similar)
// 2. Configure it to return rate limit errors for first N requests
// 3. Then return successful responses
// 4. Verify that the downloader retries the correct number of times
// 5. Verify that non-retryable errors fail immediately
