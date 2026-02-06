//! Custom Retry Strategies Example
//!
//! This example demonstrates advanced usage of the `BatchDownloader` with different
//! retry strategies and backoff configurations.
//!
//! # What This Example Shows
//!
//! 1. How to configure custom backoff strategies
//! 2. Exponential vs. constant backoff patterns
//! 3. How to tune retry parameters for different scenarios
//! 4. Observing retry behavior in action
//!
//! # Running This Example
//!
//! ```bash
//! cargo run --example custom_retry_downloader
//! ```

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use backon::{ConstantBuilder, ExponentialBuilder, FibonacciBuilder};
use katana_stage::downloader::{BatchDownloader, Downloader, DownloaderResult};

/// Simple key type for demonstration.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ItemId(u64);

/// Simple value type.
#[derive(Debug, Clone)]
struct Item {
    id: u64,
    data: String,
}

/// Custom error type.
#[derive(Debug, Clone, thiserror::Error)]
enum DownloadError {
    #[error("Transient error (attempt {0})")]
    Transient(usize),

    #[error("Permanent error")]
    Permanent,
}

/// A downloader that simulates failures for demonstration purposes.
///
/// This downloader tracks the number of attempts and can be configured to:
/// - Succeed after N attempts
/// - Always fail
/// - Succeed immediately
#[derive(Clone)]
struct ConfigurableDownloader {
    /// How many attempts before success (0 = succeed immediately)
    attempts_before_success: Arc<AtomicUsize>,
    /// Current attempt counter
    current_attempts: Arc<AtomicUsize>,
    /// Whether to fail permanently
    permanent_failure: bool,
}

impl ConfigurableDownloader {
    /// Creates a downloader that succeeds after N retries.
    fn with_retries(retries: usize) -> Self {
        Self {
            attempts_before_success: Arc::new(AtomicUsize::new(retries)),
            current_attempts: Arc::new(AtomicUsize::new(0)),
            permanent_failure: false,
        }
    }

    /// Creates a downloader that always fails permanently.
    fn with_permanent_failure() -> Self {
        Self {
            attempts_before_success: Arc::new(AtomicUsize::new(0)),
            current_attempts: Arc::new(AtomicUsize::new(0)),
            permanent_failure: true,
        }
    }

    /// Creates a downloader that always succeeds immediately.
    fn immediate_success() -> Self {
        Self::with_retries(0)
    }

    /// Reset the attempt counter.
    fn reset(&self) {
        self.current_attempts.store(0, Ordering::SeqCst);
    }

    /// Get the current attempt count.
    fn get_attempts(&self) -> usize {
        self.current_attempts.load(Ordering::SeqCst)
    }
}

impl Downloader for ConfigurableDownloader {
    type Key = ItemId;
    type Value = Item;
    type Error = DownloadError;

    async fn download(&self, key: &Self::Key) -> DownloaderResult<Self::Value, Self::Error> {
        let attempt = self.current_attempts.fetch_add(1, Ordering::SeqCst);
        let attempts_needed = self.attempts_before_success.load(Ordering::SeqCst);

        // Simulate network delay
        tokio::time::sleep(Duration::from_millis(50)).await;

        if self.permanent_failure {
            println!("  [Item {}] ❌ Permanent failure", key.0);
            return DownloaderResult::Err(DownloadError::Permanent);
        }

        if attempt < attempts_needed {
            println!(
                "  [Item {}] ⏳ Transient failure (attempt {}/{})",
                key.0,
                attempt + 1,
                attempts_needed + 1
            );
            DownloaderResult::Retry(DownloadError::Transient(attempt + 1))
        } else {
            println!("  [Item {}] ✅ Success (after {} attempts)", key.0, attempt + 1);
            DownloaderResult::Ok(Item { id: key.0, data: format!("Data for item {}", key.0) })
        }
    }
}

/// Measure and print the time taken for an operation.
async fn timed<F, T>(name: &str, f: F) -> T
where
    F: std::future::Future<Output = T>,
{
    let start = Instant::now();
    let result = f.await;
    let duration = start.elapsed();
    println!("{}", name);
    println!("⏱️  Time taken: {:.2}s\n", duration.as_secs_f64());
    result
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("🚀 Custom Retry Strategies Example\n");
    println!("This example demonstrates different retry strategies and backoff patterns.\n");

    // ═══════════════════════════════════════════════════════════
    // Example 1: Exponential Backoff (Default)
    // ═══════════════════════════════════════════════════════════
    println!("═══════════════════════════════════════════════════════════");
    println!("Example 1: Exponential Backoff (Default Strategy)");
    println!("═══════════════════════════════════════════════════════════\n");

    println!("Configuration:");
    println!("  • Min delay: 3 seconds");
    println!("  • Max delay: 60 seconds");
    println!("  • Factor: 2.0 (delay doubles each retry)");
    println!("  • Max retries: 3");
    println!("  • Expected delays: 3s, 6s, 12s\n");

    let downloader = ConfigurableDownloader::with_retries(2);
    let batch_downloader = BatchDownloader::new(downloader.clone(), 10);

    println!("Downloading item that fails twice before succeeding...\n");

    timed("Exponential backoff", async {
        let result = batch_downloader.download(vec![ItemId(1)]).await;
        match result {
            Ok(items) => println!("✅ Downloaded {} items", items.len()),
            Err(e) => println!("❌ Failed: {}", e),
        }
    })
    .await;

    // ═══════════════════════════════════════════════════════════
    // Example 2: Constant Backoff
    // ═══════════════════════════════════════════════════════════
    println!("═══════════════════════════════════════════════════════════");
    println!("Example 2: Constant Backoff");
    println!("═══════════════════════════════════════════════════════════\n");

    println!("Configuration:");
    println!("  • Constant delay: 2 seconds");
    println!("  • Max retries: 3");
    println!("  • Expected delays: 2s, 2s, 2s\n");

    let downloader = ConfigurableDownloader::with_retries(2);
    let batch_downloader = BatchDownloader::new(downloader.clone(), 10)
        .backoff(ConstantBuilder::default().with_delay(Duration::from_secs(2)).with_max_times(3));

    println!("Downloading item that fails twice before succeeding...\n");

    timed("Constant backoff", async {
        let result = batch_downloader.download(vec![ItemId(2)]).await;
        match result {
            Ok(items) => println!("✅ Downloaded {} items", items.len()),
            Err(e) => println!("❌ Failed: {}", e),
        }
    })
    .await;

    // ═══════════════════════════════════════════════════════════
    // Example 3: Fibonacci Backoff
    // ═══════════════════════════════════════════════════════════
    println!("═══════════════════════════════════════════════════════════");
    println!("Example 3: Fibonacci Backoff");
    println!("═══════════════════════════════════════════════════════════\n");

    println!("Configuration:");
    println!("  • Min delay: 1 second");
    println!("  • Max delay: 10 seconds");
    println!("  • Max retries: 4");
    println!("  • Expected delays: 1s, 1s, 2s, 3s (Fibonacci sequence)\n");

    let downloader = ConfigurableDownloader::with_retries(3);
    let batch_downloader = BatchDownloader::new(downloader.clone(), 10).backoff(
        FibonacciBuilder::default()
            .with_min_delay(Duration::from_secs(1))
            .with_max_delay(Duration::from_secs(10))
            .with_max_times(4),
    );

    println!("Downloading item that fails three times before succeeding...\n");

    timed("Fibonacci backoff", async {
        let result = batch_downloader.download(vec![ItemId(3)]).await;
        match result {
            Ok(items) => println!("✅ Downloaded {} items", items.len()),
            Err(e) => println!("❌ Failed: {}", e),
        }
    })
    .await;

    // ═══════════════════════════════════════════════════════════
    // Example 4: Aggressive Retry (Fast Recovery)
    // ═══════════════════════════════════════════════════════════
    println!("═══════════════════════════════════════════════════════════");
    println!("Example 4: Aggressive Retry Strategy (Fast Recovery)");
    println!("═══════════════════════════════════════════════════════════\n");

    println!("Configuration:");
    println!("  • Min delay: 500 milliseconds");
    println!("  • Max retries: 5");
    println!("  • Use case: Fast networks, low latency APIs\n");

    let downloader = ConfigurableDownloader::with_retries(2);
    let batch_downloader = BatchDownloader::new(downloader.clone(), 10).backoff(
        ExponentialBuilder::default().with_min_delay(Duration::from_millis(500)).with_max_times(5),
    );

    println!("Downloading item with fast retry...\n");

    timed("Aggressive retry", async {
        let result = batch_downloader.download(vec![ItemId(4)]).await;
        match result {
            Ok(items) => println!("✅ Downloaded {} items", items.len()),
            Err(e) => println!("❌ Failed: {}", e),
        }
    })
    .await;

    // ═══════════════════════════════════════════════════════════
    // Example 5: Conservative Retry (Gentle on Server)
    // ═══════════════════════════════════════════════════════════
    println!("═══════════════════════════════════════════════════════════");
    println!("Example 5: Conservative Retry Strategy (Gentle on Server)");
    println!("═══════════════════════════════════════════════════════════\n");

    println!("Configuration:");
    println!("  • Min delay: 5 seconds");
    println!("  • Max delay: 30 seconds");
    println!("  • Max retries: 2");
    println!("  • Use case: Rate-limited APIs, production systems\n");

    let downloader = ConfigurableDownloader::with_retries(1);
    let batch_downloader = BatchDownloader::new(downloader.clone(), 10).backoff(
        ExponentialBuilder::default()
            .with_min_delay(Duration::from_secs(5))
            .with_max_delay(Duration::from_secs(30))
            .with_max_times(2),
    );

    println!("Downloading item with conservative retry...\n");

    timed("Conservative retry", async {
        let result = batch_downloader.download(vec![ItemId(5)]).await;
        match result {
            Ok(items) => println!("✅ Downloaded {} items", items.len()),
            Err(e) => println!("❌ Failed: {}", e),
        }
    })
    .await;

    // ═══════════════════════════════════════════════════════════
    // Example 6: Retry Exhaustion
    // ═══════════════════════════════════════════════════════════
    println!("═══════════════════════════════════════════════════════════");
    println!("Example 6: Retry Exhaustion");
    println!("═══════════════════════════════════════════════════════════\n");

    println!("Configuration:");
    println!("  • Item needs 5 attempts to succeed");
    println!("  • But we only allow 2 retries (3 total attempts)");
    println!("  • Expected result: Failure after retries exhausted\n");

    let downloader = ConfigurableDownloader::with_retries(4); // Needs 5 attempts
    let batch_downloader = BatchDownloader::new(downloader.clone(), 10).backoff(
        ConstantBuilder::default().with_delay(Duration::from_millis(500)).with_max_times(2), // Only 2 retries
    );

    println!("Attempting download that will exhaust retries...\n");

    timed("Retry exhaustion", async {
        let result = batch_downloader.download(vec![ItemId(6)]).await;
        match result {
            Ok(items) => println!("✅ Unexpected success: {} items", items.len()),
            Err(e) => println!("❌ Failed as expected: {}", e),
        }
    })
    .await;

    println!("Total attempts made: {}", downloader.get_attempts());

    // ═══════════════════════════════════════════════════════════
    // Example 7: Batch with Mixed Success/Retry
    // ═══════════════════════════════════════════════════════════
    println!("\n═══════════════════════════════════════════════════════════");
    println!("Example 7: Batch with Mixed Success/Retry Behavior");
    println!("═══════════════════════════════════════════════════════════\n");

    println!("Scenario:");
    println!("  • Item 100: Succeeds immediately");
    println!("  • Item 101: Needs 1 retry");
    println!("  • Item 102: Succeeds immediately");
    println!("  • Item 103: Needs 2 retries");
    println!("  • Expected: Only items 101 and 103 are retried\n");

    // We need separate downloaders for each item to simulate different behaviors
    // In practice, this would be based on the actual API response
    println!("Note: In this simplified example, all items have the same retry behavior.");
    println!("In a real scenario, each item's behavior depends on the API response.\n");

    let downloader = ConfigurableDownloader::with_retries(1);
    let batch_downloader = BatchDownloader::new(downloader, 10).backoff(
        ConstantBuilder::default().with_delay(Duration::from_millis(500)).with_max_times(3),
    );

    println!("Downloading batch of 4 items...\n");

    timed("Mixed batch", async {
        let keys = vec![ItemId(100), ItemId(101), ItemId(102), ItemId(103)];
        let result = batch_downloader.download(keys).await;
        match result {
            Ok(items) => {
                println!("✅ Downloaded {} items:", items.len());
                for item in items {
                    println!("   • Item {}: {}", item.id, item.data);
                }
            }
            Err(e) => println!("❌ Failed: {}", e),
        }
    })
    .await;

    // ═══════════════════════════════════════════════════════════
    // Summary
    // ═══════════════════════════════════════════════════════════
    println!("═══════════════════════════════════════════════════════════");
    println!("✨ Example Completed!");
    println!("═══════════════════════════════════════════════════════════\n");

    println!("Retry Strategy Recommendations:\n");
    println!("1. Exponential Backoff (Default)");
    println!("   • Best for: General purpose, most APIs");
    println!("   • Pros: Balances speed and server load");
    println!("   • Cons: Can be slow for very transient failures\n");

    println!("2. Constant Backoff");
    println!("   • Best for: Predictable retry timing, testing");
    println!("   • Pros: Simple, predictable");
    println!("   • Cons: Doesn't adapt to system load\n");

    println!("3. Fibonacci Backoff");
    println!("   • Best for: Gradual ramp-up scenarios");
    println!("   • Pros: More gradual than exponential");
    println!("   • Cons: More complex to reason about\n");

    println!("4. Aggressive (Fast Recovery)");
    println!("   • Best for: Low-latency networks, internal APIs");
    println!("   • Pros: Fast recovery from transient issues");
    println!("   • Cons: Can overwhelm struggling servers\n");

    println!("5. Conservative (Gentle)");
    println!("   • Best for: Rate-limited APIs, production systems");
    println!("   • Pros: Gentle on servers, respects rate limits");
    println!("   • Cons: Slow recovery\n");

    println!("Choose your strategy based on:");
    println!("  • API rate limits and behavior");
    println!("  • Network characteristics (latency, reliability)");
    println!("  • Urgency of data retrieval");
    println!("  • Server load tolerance\n");

    Ok(())
}
