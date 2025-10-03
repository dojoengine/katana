use std::future::Future;
use std::time::Duration;

use anyhow::Result;
use backon::{BackoffBuilder, ExponentialBuilder};
use tracing::trace;

#[derive(Debug, Clone)]
pub struct BatchDownloader<D, B = ExponentialBuilder> {
    backoff: B,
    downloader: D,
    batch_size: usize,
}

impl<D> BatchDownloader<D> {
    pub fn new(downloader: D, batch_size: usize) -> Self {
        let backoff = ExponentialBuilder::default().with_min_delay(Duration::from_secs(3));
        Self { backoff, downloader, batch_size }
    }

    /// Set the backoff strategy for retrying failed downloads.
    #[cfg(test)]
    pub fn backoff<B>(self, strategy: B) -> BatchDownloader<D, B> {
        BatchDownloader {
            backoff: strategy,
            downloader: self.downloader,
            batch_size: self.batch_size,
        }
    }
}

impl<D, B> BatchDownloader<D, B>
where
    D: Downloader,
    B: BackoffBuilder + Clone,
{
    pub async fn download(&self, keys: &[D::Key]) -> Result<Vec<D::Value>, D::Error> {
        let mut items = Vec::with_capacity(keys.len());

        for chunk in keys.chunks(self.batch_size) {
            let batch = self.download_batch_with_retry(chunk.to_vec()).await?;
            items.extend(batch);
        }

        Ok(items)
    }

    async fn download_batch(&self, keys: &[D::Key]) -> Vec<DownloaderResult<D::Value, D::Error>> {
        let mut requests = Vec::with_capacity(keys.len());
        for key in keys {
            requests.push(self.downloader.download(key));
        }
        futures::future::join_all(requests).await
    }

    async fn download_batch_with_retry(
        &self,
        keys: Vec<D::Key>,
    ) -> Result<Vec<D::Value>, D::Error> {
        let mut results: Vec<Option<D::Value>> = (0..keys.len()).map(|_| None).collect();

        let mut remaining_keys = keys.clone();
        let mut backoff = self.backoff.clone().build();

        loop {
            let batch_result = self.download_batch(&remaining_keys).await;

            let mut failed_keys = Vec::with_capacity(remaining_keys.len());
            let mut last_error = None;

            for (key, result) in remaining_keys.iter().zip(batch_result.into_iter()) {
                let (key_idx, _) =
                    keys.iter().enumerate().find(|(_, k)| *k == key).expect("qed; must exist");

                match result {
                    // cache the result for successful requests
                    DownloaderResult::Ok(value) => {
                        results[key_idx] = Some(value);
                    }
                    // flag the failed request for retry, if the error is retryable
                    DownloaderResult::Retry(error) => {
                        failed_keys.push(key.clone());
                        last_error = Some(error);
                    }
                    DownloaderResult::Err(error) => {
                        // Non-retryable error, fail immediately
                        return Err(error);
                    }
                }
            }

            // if not failed keys, all requests succeeded
            if failed_keys.is_empty() {
                break;
            }

            // Check if we should retry
            if let Some(delay) = backoff.next() {
                if let Some(ref error) = last_error {
                    trace!(%error, failed_keys = %failed_keys.len(), "Retrying download for failed keys.");
                }

                tokio::time::sleep(delay).await;
                remaining_keys = failed_keys;
            } else {
                // No more retries allowed
                if let Some(error) = last_error {
                    return Err(error);
                }
            }
        }

        Ok(results.into_iter().map(|v| v.expect("qed; all values must be set")).collect())
    }
}

#[derive(Debug, Clone)]
pub enum DownloaderResult<T, E> {
    Ok(T),
    Err(E),
    Retry(E),
}

pub trait Downloader {
    type Key: Clone + PartialEq + Eq + Send + Sync;
    type Value: Send + Sync;
    type Error: std::error::Error + Send;

    fn download(
        &self,
        key: &Self::Key,
    ) -> impl Future<Output = DownloaderResult<Self::Value, Self::Error>>;
}

#[cfg(test)]
mod tests {
    //! Unit tests for [`BatchDownloader`].
    //!
    //! These tests use a mock downloader implementation to verify all control flows
    //! including successful downloads, retries, error handling, and batching behavior.

    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use backon::{ConstantBuilder, ExponentialBuilder};

    use super::*;

    /// Mock error type for testing the downloader.
    ///
    /// This error type is used to simulate both retryable and non-retryable errors
    /// in the test scenarios.
    #[derive(Debug, Clone, thiserror::Error)]
    #[error("MockError: {0}")]
    struct MockError(String);

    /// Mock downloader implementation for testing [`BatchDownloader`].
    ///
    /// This mock allows precise control over download behavior by pre-configuring
    /// responses for each key. It tracks download attempts to verify retry logic.
    ///
    /// # Usage
    ///
    /// Configure the mock with expected responses for each key, where each key maps
    /// to a sequence of results returned on successive download attempts:
    ///
    /// ```ignore
    /// let downloader = MockDownloader::new()
    ///     .with_response(1, vec![DownloaderResult::Ok("success".to_string())])
    ///     .with_response(2, vec![
    ///         DownloaderResult::Retry(MockError("temp".to_string())),
    ///         DownloaderResult::Ok("success".to_string()),
    ///     ]);
    /// ```
    ///
    /// The first call to `download(&1)` returns `Ok("success")`.
    /// The first call to `download(&2)` returns `Retry`, the second returns `Ok("success")`.
    #[derive(Clone)]
    struct MockDownloader {
        /// Map of key to a list of results to return on each successive attempt.
        responses: Arc<Mutex<HashMap<u64, Vec<DownloaderResult<String, MockError>>>>>,
        /// Tracks the number of download attempts per key.
        attempts: Arc<Mutex<HashMap<u64, Arc<AtomicUsize>>>>,
    }

    impl MockDownloader {
        /// Creates a new mock downloader with no pre-configured responses.
        fn new() -> Self {
            Self {
                responses: Arc::new(Mutex::new(HashMap::new())),
                attempts: Arc::new(Mutex::new(HashMap::new())),
            }
        }

        /// Configures the response sequence for a specific key.
        ///
        /// Each element in `responses` corresponds to the result returned on successive
        /// download attempts for this key. If attempts exceed the configured responses,
        /// an error is returned.
        ///
        /// # Arguments
        ///
        /// * `key` - The key to configure
        /// * `responses` - Sequence of results to return on each attempt
        ///
        /// # Example
        ///
        /// ```ignore
        /// let downloader = MockDownloader::new()
        ///     .with_response(1, vec![
        ///         DownloaderResult::Retry(MockError("first try fails".to_string())),
        ///         DownloaderResult::Ok("second try succeeds".to_string()),
        ///     ]);
        /// ```
        fn with_response(
            self,
            key: u64,
            responses: Vec<DownloaderResult<String, MockError>>,
        ) -> Self {
            self.responses.lock().unwrap().insert(key, responses);
            self.attempts.lock().unwrap().insert(key, Arc::new(AtomicUsize::new(0)));
            self
        }

        /// Returns the total number of download attempts made for a specific key.
        ///
        /// This is useful for verifying retry behavior in tests.
        ///
        /// # Arguments
        ///
        /// * `key` - The key to query
        ///
        /// # Returns
        ///
        /// The number of times `download` was called for this key, or 0 if never called.
        fn get_attempts(&self, key: u64) -> usize {
            self.attempts.lock().unwrap().get(&key).map(|a| a.load(Ordering::SeqCst)).unwrap_or(0)
        }
    }

    impl Downloader for MockDownloader {
        type Key = u64;
        type Value = String;
        type Error = MockError;

        async fn download(&self, key: &Self::Key) -> DownloaderResult<Self::Value, Self::Error> {
            let attempt_counter = {
                let mut attempts = self.attempts.lock().unwrap();
                attempts.entry(*key).or_insert_with(|| Arc::new(AtomicUsize::new(0))).clone()
            };

            let attempt = attempt_counter.fetch_add(1, Ordering::SeqCst);

            let responses = self.responses.lock().unwrap();
            responses.get(key).and_then(|r| r.get(attempt).cloned()).unwrap_or_else(|| {
                DownloaderResult::Err(MockError(format!(
                    "No response configured for key {} attempt {}",
                    key, attempt
                )))
            })
        }
    }

    #[tokio::test]
    async fn all_downloads_succeed_first_try() {
        let downloader = MockDownloader::new()
            .with_response(1, vec![DownloaderResult::Ok("value1".to_string())])
            .with_response(2, vec![DownloaderResult::Ok("value2".to_string())])
            .with_response(3, vec![DownloaderResult::Ok("value3".to_string())]);

        let batch_downloader = BatchDownloader::new(downloader.clone(), 10);
        let keys = vec![1, 2, 3];
        let result = batch_downloader.download(&keys).await;

        assert!(result.is_ok());
        let values = result.unwrap();
        assert_eq!(values, vec!["value1".to_string(), "value2".to_string(), "value3".to_string()]);

        // Each key should be downloaded exactly once
        assert_eq!(downloader.get_attempts(1), 1);
        assert_eq!(downloader.get_attempts(2), 1);
        assert_eq!(downloader.get_attempts(3), 1);
    }

    #[tokio::test]
    async fn empty_keys_list() {
        let downloader = MockDownloader::new();
        let batch_downloader = BatchDownloader::new(downloader, 10);
        let keys: Vec<u64> = vec![];
        let result = batch_downloader.download(&keys).await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), Vec::<String>::new());
    }

    #[tokio::test]
    async fn retry_then_succeed() {
        let downloader = MockDownloader::new()
            .with_response(
                1,
                vec![
                    DownloaderResult::Retry(MockError("temporary error".to_string())),
                    DownloaderResult::Ok("value1".to_string()),
                ],
            )
            .with_response(2, vec![DownloaderResult::Ok("value2".to_string())]);

        let batch_downloader = BatchDownloader::new(downloader.clone(), 10)
            .backoff(ConstantBuilder::default().with_delay(Duration::from_millis(1)));

        let keys = vec![1, 2];
        let result = batch_downloader.download(&keys).await;

        assert!(result.is_ok());
        let values = result.unwrap();
        assert_eq!(values, vec!["value1".to_string(), "value2".to_string()]);

        // Key 1 should be downloaded twice (initial + 1 retry)
        assert_eq!(downloader.get_attempts(1), 2);
        // Key 2 should be downloaded once
        assert_eq!(downloader.get_attempts(2), 1);
    }

    #[tokio::test]
    async fn multiple_retries_then_succeed() {
        let downloader = MockDownloader::new().with_response(
            1,
            vec![
                DownloaderResult::Retry(MockError("error 1".to_string())),
                DownloaderResult::Retry(MockError("error 2".to_string())),
                DownloaderResult::Retry(MockError("error 3".to_string())),
                DownloaderResult::Ok("value1".to_string()),
            ],
        );

        let batch_downloader = BatchDownloader::new(downloader.clone(), 10).backoff(
            ConstantBuilder::default().with_delay(Duration::from_millis(1)).with_max_times(5),
        );

        let keys = vec![1];
        let result = batch_downloader.download(&keys).await;

        assert!(result.is_ok());
        let values = result.unwrap();
        assert_eq!(values, vec!["value1".to_string()]);

        // Key should be downloaded 4 times
        assert_eq!(downloader.get_attempts(1), 4);
    }

    #[tokio::test]
    async fn retry_exhaustion() {
        let downloader = MockDownloader::new().with_response(
            1,
            vec![
                DownloaderResult::Retry(MockError("error 1".to_string())),
                DownloaderResult::Retry(MockError("error 2".to_string())),
                DownloaderResult::Retry(MockError("error 3".to_string())),
                DownloaderResult::Ok("value1".to_string()),
            ],
        );

        // Only allow 2 retry attempts (3 total attempts)
        let batch_downloader = BatchDownloader::new(downloader.clone(), 10).backoff(
            ConstantBuilder::default().with_delay(Duration::from_millis(1)).with_max_times(2),
        );

        let keys = vec![1];
        let result = batch_downloader.download(&keys).await;

        // Should fail because retries exhausted
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().to_string(), "MockError: error 3");

        // Key should be downloaded 3 times (initial + 2 retries)
        assert_eq!(downloader.get_attempts(1), 3);
    }

    #[tokio::test]
    async fn non_retryable_error_fails_immediately() {
        let downloader = MockDownloader::new()
            .with_response(1, vec![DownloaderResult::Err(MockError("fatal error".to_string()))])
            .with_response(2, vec![DownloaderResult::Ok("value2".to_string())]);

        let batch_downloader = BatchDownloader::new(downloader.clone(), 10);
        let keys = vec![1, 2];
        let result = batch_downloader.download(&keys).await;

        // Should fail immediately with non-retryable error
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().to_string(), "MockError: fatal error");

        // Key 1 should be downloaded exactly once (no retry on Err)
        assert_eq!(downloader.get_attempts(1), 1);
    }

    #[tokio::test]
    async fn mixed_results_in_batch() {
        let downloader = MockDownloader::new()
            .with_response(1, vec![DownloaderResult::Ok("value1".to_string())])
            .with_response(
                2,
                vec![
                    DownloaderResult::Retry(MockError("temp error".to_string())),
                    DownloaderResult::Ok("value2".to_string()),
                ],
            )
            .with_response(3, vec![DownloaderResult::Ok("value3".to_string())]);

        let batch_downloader = BatchDownloader::new(downloader.clone(), 10)
            .backoff(ConstantBuilder::default().with_delay(Duration::from_millis(1)));

        let keys = vec![1, 2, 3];
        let result = batch_downloader.download(&keys).await;

        assert!(result.is_ok());
        let values = result.unwrap();
        assert_eq!(values, vec!["value1".to_string(), "value2".to_string(), "value3".to_string()]);

        // Keys 1 and 3 should be downloaded once
        assert_eq!(downloader.get_attempts(1), 1);
        assert_eq!(downloader.get_attempts(3), 1);
        // Key 2 should be downloaded twice
        assert_eq!(downloader.get_attempts(2), 2);
    }

    #[tokio::test]
    async fn batching_multiple_chunks() {
        let downloader = MockDownloader::new()
            .with_response(1, vec![DownloaderResult::Ok("value1".to_string())])
            .with_response(2, vec![DownloaderResult::Ok("value2".to_string())])
            .with_response(3, vec![DownloaderResult::Ok("value3".to_string())])
            .with_response(4, vec![DownloaderResult::Ok("value4".to_string())])
            .with_response(5, vec![DownloaderResult::Ok("value5".to_string())]);

        // Batch size of 2 means 5 keys will be split into 3 batches: [1,2], [3,4], [5]
        let batch_downloader = BatchDownloader::new(downloader.clone(), 2);
        let keys = vec![1, 2, 3, 4, 5];
        let result = batch_downloader.download(&keys).await;

        assert!(result.is_ok());
        let values = result.unwrap();
        assert_eq!(
            values,
            vec![
                "value1".to_string(),
                "value2".to_string(),
                "value3".to_string(),
                "value4".to_string(),
                "value5".to_string()
            ]
        );

        // All keys should be downloaded exactly once
        for key in 1..=5 {
            assert_eq!(downloader.get_attempts(key), 1);
        }
    }

    #[tokio::test]
    async fn batching_exact_multiple() {
        let downloader = MockDownloader::new()
            .with_response(1, vec![DownloaderResult::Ok("value1".to_string())])
            .with_response(2, vec![DownloaderResult::Ok("value2".to_string())])
            .with_response(3, vec![DownloaderResult::Ok("value3".to_string())])
            .with_response(4, vec![DownloaderResult::Ok("value4".to_string())]);

        // Batch size of 2 with 4 keys should create exactly 2 batches
        let batch_downloader = BatchDownloader::new(downloader.clone(), 2);
        let keys = vec![1, 2, 3, 4];
        let result = batch_downloader.download(&keys).await;

        assert!(result.is_ok());
        let values = result.unwrap();
        assert_eq!(
            values,
            vec![
                "value1".to_string(),
                "value2".to_string(),
                "value3".to_string(),
                "value4".to_string()
            ]
        );

        for key in 1..=4 {
            assert_eq!(downloader.get_attempts(key), 1);
        }
    }

    #[tokio::test]
    async fn batching_smaller_than_batch_size() {
        let downloader = MockDownloader::new()
            .with_response(1, vec![DownloaderResult::Ok("value1".to_string())])
            .with_response(2, vec![DownloaderResult::Ok("value2".to_string())]);

        // Batch size larger than number of keys
        let batch_downloader = BatchDownloader::new(downloader.clone(), 10);
        let keys = vec![1, 2];
        let result = batch_downloader.download(&keys).await;

        assert!(result.is_ok());
        let values = result.unwrap();
        assert_eq!(values, vec!["value1".to_string(), "value2".to_string()]);

        assert_eq!(downloader.get_attempts(1), 1);
        assert_eq!(downloader.get_attempts(2), 1);
    }

    #[tokio::test]
    async fn custom_backoff_strategy() {
        let downloader = MockDownloader::new().with_response(
            1,
            vec![
                DownloaderResult::Retry(MockError("error 1".to_string())),
                DownloaderResult::Retry(MockError("error 2".to_string())),
                DownloaderResult::Ok("value1".to_string()),
            ],
        );

        // Use exponential backoff with custom settings
        let custom_backoff = ExponentialBuilder::default()
            .with_min_delay(Duration::from_millis(1))
            .with_max_delay(Duration::from_millis(10))
            .with_max_times(5);

        let batch_downloader = BatchDownloader::new(downloader.clone(), 10).backoff(custom_backoff);

        let keys = vec![1];
        let result = batch_downloader.download(&keys).await;

        assert!(result.is_ok());
        let values = result.unwrap();
        assert_eq!(values, vec!["value1".to_string()]);

        // Should have made 3 attempts
        assert_eq!(downloader.get_attempts(1), 3);
    }

    #[tokio::test]
    async fn retry_across_multiple_batches() {
        let downloader = MockDownloader::new()
            .with_response(1, vec![DownloaderResult::Ok("value1".to_string())])
            .with_response(
                2,
                vec![
                    DownloaderResult::Retry(MockError("temp error".to_string())),
                    DownloaderResult::Ok("value2".to_string()),
                ],
            )
            .with_response(3, vec![DownloaderResult::Ok("value3".to_string())])
            .with_response(
                4,
                vec![
                    DownloaderResult::Retry(MockError("temp error".to_string())),
                    DownloaderResult::Ok("value4".to_string()),
                ],
            );

        let batch_downloader = BatchDownloader::new(downloader.clone(), 2)
            .backoff(ConstantBuilder::default().with_delay(Duration::from_millis(1)));

        let keys = vec![1, 2, 3, 4];
        let result = batch_downloader.download(&keys).await;

        assert!(result.is_ok());
        let values = result.unwrap();
        assert_eq!(
            values,
            vec![
                "value1".to_string(),
                "value2".to_string(),
                "value3".to_string(),
                "value4".to_string()
            ]
        );

        // Keys 1 and 3 downloaded once
        assert_eq!(downloader.get_attempts(1), 1);
        assert_eq!(downloader.get_attempts(3), 1);
        // Keys 2 and 4 downloaded twice
        assert_eq!(downloader.get_attempts(2), 2);
        assert_eq!(downloader.get_attempts(4), 2);
    }
}
