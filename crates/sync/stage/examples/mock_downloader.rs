//! Mock Downloader for Testing
//!
//! This example demonstrates how to create mock downloader implementations for testing
//! the Downloader architecture. Mock downloaders are essential for:
//! - Unit testing stages that depend on downloaders
//! - Testing retry logic without real network calls
//! - Simulating various error scenarios
//! - Verifying download behavior and attempt counts
//!
//! # What This Example Shows
//!
//! 1. How to create a configurable mock downloader
//! 2. How to test retry behavior
//! 3. How to verify download attempts
//! 4. How to simulate different error scenarios
//! 5. How to test partial batch failures
//!
//! # Running This Example
//!
//! ```bash
//! cargo run --example mock_downloader
//! ```

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use backon::ConstantBuilder;
use katana_stage::downloader::{BatchDownloader, Downloader, DownloaderResult};

/// Simple key type for testing.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ItemKey(u64);

/// Simple value type for testing.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ItemValue(String);

/// Test error type.
#[derive(Debug, Clone, thiserror::Error, PartialEq)]
enum TestError {
    #[error("Transient error: {0}")]
    Transient(String),

    #[error("Permanent error: {0}")]
    Permanent(String),
}

/// A mock downloader for testing.
///
/// This mock allows you to pre-configure responses for each key, where each key
/// maps to a sequence of results returned on successive download attempts.
///
/// # Example
///
/// ```ignore
/// let downloader = MockDownloader::new()
///     .with_response(1, vec![
///         DownloaderResult::Ok(ItemValue("success".to_string()))
///     ])
///     .with_response(2, vec![
///         DownloaderResult::Retry(TestError::Transient("first try".to_string())),
///         DownloaderResult::Ok(ItemValue("success after retry".to_string())),
///     ]);
/// ```
#[derive(Clone)]
struct MockDownloader {
    /// Pre-configured responses for each key.
    /// The outer Vec represents multiple attempts for the same key.
    responses: Arc<Mutex<HashMap<u64, Vec<DownloaderResult<ItemValue, TestError>>>>>,

    /// Track the number of download attempts per key.
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
    /// download attempts for this key.
    ///
    /// # Arguments
    ///
    /// * `key` - The key to configure
    /// * `responses` - Sequence of results to return on each attempt
    fn with_response(
        self,
        key: u64,
        responses: Vec<DownloaderResult<ItemValue, TestError>>,
    ) -> Self {
        self.responses.lock().unwrap().insert(key, responses);
        self.attempts.lock().unwrap().insert(key, Arc::new(AtomicUsize::new(0)));
        self
    }

    /// Returns the number of times `download` was called for a specific key.
    fn get_attempts(&self, key: u64) -> usize {
        self.attempts.lock().unwrap().get(&key).map(|a| a.load(Ordering::SeqCst)).unwrap_or(0)
    }

    /// Resets all attempt counters.
    #[allow(dead_code)]
    fn reset_attempts(&self) {
        for counter in self.attempts.lock().unwrap().values() {
            counter.store(0, Ordering::SeqCst);
        }
    }
}

impl Downloader for MockDownloader {
    type Key = ItemKey;
    type Value = ItemValue;
    type Error = TestError;

    async fn download(&self, key: &Self::Key) -> DownloaderResult<Self::Value, Self::Error> {
        // Get or create attempt counter for this key
        let attempt_counter = {
            let mut attempts = self.attempts.lock().unwrap();
            attempts.entry(key.0).or_insert_with(|| Arc::new(AtomicUsize::new(0))).clone()
        };

        let attempt = attempt_counter.fetch_add(1, Ordering::SeqCst);

        // Look up the pre-configured response for this attempt
        let responses = self.responses.lock().unwrap();
        responses.get(&key.0).and_then(|r| r.get(attempt).cloned()).unwrap_or_else(|| {
            DownloaderResult::Err(TestError::Permanent(format!(
                "No response configured for key {} attempt {}",
                key.0, attempt
            )))
        })
    }
}

/// Helper function to run a test scenario.
async fn run_test_scenario(
    name: &str,
    description: &str,
    test_fn: impl std::future::Future<Output = ()>,
) {
    println!("═══════════════════════════════════════════════════════════");
    println!("Test: {}", name);
    println!("═══════════════════════════════════════════════════════════");
    println!("{}\n", description);
    test_fn.await;
    println!("✅ Test passed!\n");
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("🧪 Mock Downloader Testing Example\n");
    println!("This example demonstrates how to use mock downloaders for testing.\n");

    // ═══════════════════════════════════════════════════════════
    // Test 1: All Downloads Succeed
    // ═══════════════════════════════════════════════════════════
    run_test_scenario(
        "All Downloads Succeed",
        "All items download successfully on the first attempt.",
        async {
            let downloader = MockDownloader::new()
                .with_response(1, vec![DownloaderResult::Ok(ItemValue("value1".to_string()))])
                .with_response(2, vec![DownloaderResult::Ok(ItemValue("value2".to_string()))])
                .with_response(3, vec![DownloaderResult::Ok(ItemValue("value3".to_string()))]);

            let batch_downloader = BatchDownloader::new(downloader.clone(), 10);
            let keys = vec![ItemKey(1), ItemKey(2), ItemKey(3)];

            let result = batch_downloader.download(keys).await;
            assert!(result.is_ok(), "Expected success");

            let values = result.unwrap();
            assert_eq!(values.len(), 3, "Expected 3 values");
            assert_eq!(values[0], ItemValue("value1".to_string()));
            assert_eq!(values[1], ItemValue("value2".to_string()));
            assert_eq!(values[2], ItemValue("value3".to_string()));

            // Verify each key was downloaded exactly once
            assert_eq!(downloader.get_attempts(1), 1);
            assert_eq!(downloader.get_attempts(2), 1);
            assert_eq!(downloader.get_attempts(3), 1);

            println!("✓ All 3 items downloaded successfully");
            println!("✓ Each item was downloaded exactly once");
        },
    )
    .await;

    // ═══════════════════════════════════════════════════════════
    // Test 2: Retry Then Succeed
    // ═══════════════════════════════════════════════════════════
    run_test_scenario(
        "Retry Then Succeed",
        "One item fails with a retryable error, then succeeds on retry.",
        async {
            let downloader = MockDownloader::new()
                .with_response(
                    1,
                    vec![
                        DownloaderResult::Retry(TestError::Transient(
                            "temporary error".to_string(),
                        )),
                        DownloaderResult::Ok(ItemValue("value1".to_string())),
                    ],
                )
                .with_response(2, vec![DownloaderResult::Ok(ItemValue("value2".to_string()))]);

            let batch_downloader = BatchDownloader::new(downloader.clone(), 10)
                .backoff(ConstantBuilder::default().with_delay(Duration::from_millis(10)));

            let keys = vec![ItemKey(1), ItemKey(2)];
            let result = batch_downloader.download(keys).await;

            assert!(result.is_ok(), "Expected success after retry");

            let values = result.unwrap();
            assert_eq!(values.len(), 2);

            // Key 1 should be downloaded twice (initial + 1 retry)
            assert_eq!(downloader.get_attempts(1), 2);
            // Key 2 should be downloaded once
            assert_eq!(downloader.get_attempts(2), 1);

            println!("✓ Item 1 succeeded after 1 retry (2 total attempts)");
            println!("✓ Item 2 succeeded immediately (1 attempt)");
            println!("✓ Only the failed item was retried");
        },
    )
    .await;

    // ═══════════════════════════════════════════════════════════
    // Test 3: Multiple Retries
    // ═══════════════════════════════════════════════════════════
    run_test_scenario(
        "Multiple Retries Before Success",
        "An item fails multiple times before succeeding.",
        async {
            let downloader = MockDownloader::new().with_response(
                1,
                vec![
                    DownloaderResult::Retry(TestError::Transient("error 1".to_string())),
                    DownloaderResult::Retry(TestError::Transient("error 2".to_string())),
                    DownloaderResult::Retry(TestError::Transient("error 3".to_string())),
                    DownloaderResult::Ok(ItemValue("value1".to_string())),
                ],
            );

            let batch_downloader = BatchDownloader::new(downloader.clone(), 10).backoff(
                ConstantBuilder::default().with_delay(Duration::from_millis(10)).with_max_times(5),
            );

            let keys = vec![ItemKey(1)];
            let result = batch_downloader.download(keys).await;

            assert!(result.is_ok(), "Expected success after multiple retries");
            assert_eq!(downloader.get_attempts(1), 4); // Initial + 3 retries

            println!("✓ Item succeeded after 3 retries (4 total attempts)");
        },
    )
    .await;

    // ═══════════════════════════════════════════════════════════
    // Test 4: Retry Exhaustion
    // ═══════════════════════════════════════════════════════════
    run_test_scenario(
        "Retry Exhaustion",
        "An item keeps failing until retries are exhausted.",
        async {
            let downloader = MockDownloader::new().with_response(
                1,
                vec![
                    DownloaderResult::Retry(TestError::Transient("error 1".to_string())),
                    DownloaderResult::Retry(TestError::Transient("error 2".to_string())),
                    DownloaderResult::Retry(TestError::Transient("error 3".to_string())),
                    DownloaderResult::Ok(ItemValue("value1".to_string())),
                ],
            );

            // Only allow 2 retry attempts (3 total attempts)
            let batch_downloader = BatchDownloader::new(downloader.clone(), 10).backoff(
                ConstantBuilder::default().with_delay(Duration::from_millis(10)).with_max_times(2),
            );

            let keys = vec![ItemKey(1)];
            let result = batch_downloader.download(keys).await;

            assert!(result.is_err(), "Expected failure after exhausting retries");
            assert_eq!(downloader.get_attempts(1), 3); // Initial + 2 retries

            println!("✓ Failed as expected after exhausting 2 retries");
            println!("✓ Total attempts: 3 (initial + 2 retries)");
        },
    )
    .await;

    // ═══════════════════════════════════════════════════════════
    // Test 5: Non-Retryable Error
    // ═══════════════════════════════════════════════════════════
    run_test_scenario(
        "Non-Retryable Error Fails Immediately",
        "A permanent error causes immediate failure without retries.",
        async {
            let downloader = MockDownloader::new()
                .with_response(
                    1,
                    vec![DownloaderResult::Err(TestError::Permanent("fatal error".to_string()))],
                )
                .with_response(2, vec![DownloaderResult::Ok(ItemValue("value2".to_string()))]);

            let batch_downloader = BatchDownloader::new(downloader.clone(), 10);
            let keys = vec![ItemKey(1), ItemKey(2)];
            let result = batch_downloader.download(keys).await;

            assert!(result.is_err(), "Expected immediate failure");
            // Key 1 should be downloaded exactly once (no retry on Err)
            assert_eq!(downloader.get_attempts(1), 1);

            println!("✓ Failed immediately on permanent error");
            println!("✓ No retries were attempted (1 attempt total)");
        },
    )
    .await;

    // ═══════════════════════════════════════════════════════════
    // Test 6: Mixed Results in Batch
    // ═══════════════════════════════════════════════════════════
    run_test_scenario(
        "Mixed Results in Batch",
        "A batch with successful, retryable, and immediately successful items.",
        async {
            let downloader = MockDownloader::new()
                .with_response(1, vec![DownloaderResult::Ok(ItemValue("value1".to_string()))])
                .with_response(
                    2,
                    vec![
                        DownloaderResult::Retry(TestError::Transient("temp error".to_string())),
                        DownloaderResult::Ok(ItemValue("value2".to_string())),
                    ],
                )
                .with_response(3, vec![DownloaderResult::Ok(ItemValue("value3".to_string()))]);

            let batch_downloader = BatchDownloader::new(downloader.clone(), 10)
                .backoff(ConstantBuilder::default().with_delay(Duration::from_millis(10)));

            let keys = vec![ItemKey(1), ItemKey(2), ItemKey(3)];
            let result = batch_downloader.download(keys).await;

            assert!(result.is_ok(), "Expected success");

            // Keys 1 and 3 should be downloaded once
            assert_eq!(downloader.get_attempts(1), 1);
            assert_eq!(downloader.get_attempts(3), 1);
            // Key 2 should be downloaded twice
            assert_eq!(downloader.get_attempts(2), 2);

            println!("✓ Items 1 and 3 succeeded immediately (1 attempt each)");
            println!("✓ Item 2 succeeded after 1 retry (2 attempts)");
            println!("✓ Successful items were not re-downloaded during retry");
        },
    )
    .await;

    // ═══════════════════════════════════════════════════════════
    // Test 7: Empty Keys List
    // ═══════════════════════════════════════════════════════════
    run_test_scenario(
        "Empty Keys List",
        "Downloading an empty list should succeed immediately.",
        async {
            let downloader = MockDownloader::new();
            let batch_downloader = BatchDownloader::new(downloader, 10);
            let keys: Vec<ItemKey> = vec![];

            let result = batch_downloader.download(keys).await;
            assert!(result.is_ok(), "Expected success for empty list");

            let values = result.unwrap();
            assert_eq!(values.len(), 0, "Expected empty result");

            println!("✓ Empty key list handled correctly");
            println!("✓ No downloads were attempted");
        },
    )
    .await;

    // ═══════════════════════════════════════════════════════════
    // Test 8: Batching Behavior
    // ═══════════════════════════════════════════════════════════
    run_test_scenario(
        "Batching Behavior",
        "Items are split into batches of the configured size.",
        async {
            let downloader = MockDownloader::new()
                .with_response(1, vec![DownloaderResult::Ok(ItemValue("value1".to_string()))])
                .with_response(2, vec![DownloaderResult::Ok(ItemValue("value2".to_string()))])
                .with_response(3, vec![DownloaderResult::Ok(ItemValue("value3".to_string()))])
                .with_response(4, vec![DownloaderResult::Ok(ItemValue("value4".to_string()))])
                .with_response(5, vec![DownloaderResult::Ok(ItemValue("value5".to_string()))]);

            // Batch size of 2 means 5 keys will be split into 3 batches: [1,2], [3,4], [5]
            let batch_downloader = BatchDownloader::new(downloader.clone(), 2);
            let keys = vec![ItemKey(1), ItemKey(2), ItemKey(3), ItemKey(4), ItemKey(5)];

            let result = batch_downloader.download(keys).await;
            assert!(result.is_ok(), "Expected success");

            let values = result.unwrap();
            assert_eq!(values.len(), 5);

            // All keys should be downloaded exactly once
            for key in 1..=5 {
                assert_eq!(downloader.get_attempts(key), 1);
            }

            println!("✓ 5 items split into 3 batches of size 2");
            println!("✓ Batches processed sequentially: [1,2], [3,4], [5]");
            println!("✓ All items downloaded exactly once");
        },
    )
    .await;

    // ═══════════════════════════════════════════════════════════
    // Summary
    // ═══════════════════════════════════════════════════════════
    println!("═══════════════════════════════════════════════════════════");
    println!("✨ All Tests Passed!");
    println!("═══════════════════════════════════════════════════════════\n");

    println!("Key Testing Patterns Demonstrated:\n");

    println!("1. Success Verification");
    println!("   • Check that successful downloads return expected values");
    println!("   • Verify the correct number of items are returned\n");

    println!("2. Retry Logic Testing");
    println!("   • Configure items to fail N times before succeeding");
    println!("   • Verify the correct number of attempts are made");
    println!("   • Test retry exhaustion scenarios\n");

    println!("3. Error Classification");
    println!("   • Use Retry for transient errors");
    println!("   • Use Err for permanent errors");
    println!("   • Verify immediate failure on permanent errors\n");

    println!("4. Partial Batch Failures");
    println!("   • Mix successful and failing items in a batch");
    println!("   • Verify only failed items are retried");
    println!("   • Confirm successful items are not re-downloaded\n");

    println!("5. Attempt Counting");
    println!("   • Track download attempts per key");
    println!("   • Verify retry logic through attempt counts");
    println!("   • Useful for debugging and verification\n");

    println!("Using Mocks in Your Tests:\n");

    println!("1. Create a mock downloader with pre-configured responses");
    println!("2. Configure different response sequences for different scenarios");
    println!("3. Use the mock in place of real downloaders in your tests");
    println!("4. Verify behavior through attempt counters and results");
    println!("5. Test edge cases without network dependencies\n");

    println!("Benefits:");
    println!("  ✓ Fast test execution (no network calls)");
    println!("  ✓ Deterministic test outcomes");
    println!("  ✓ Easy to simulate error scenarios");
    println!("  ✓ No external dependencies");
    println!("  ✓ Complete control over test scenarios\n");

    Ok(())
}
