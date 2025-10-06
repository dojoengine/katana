# Downloader Architecture Guide

This guide explains how to use the `Downloader` trait and `BatchDownloader` architecture for downloading data with automatic retry logic and batch processing.

## Table of Contents

- [Overview](#overview)
- [Architecture](#architecture)
- [Core Components](#core-components)
- [How It Works](#how-it-works)
- [Implementation Guide](#implementation-guide)
- [Usage Patterns](#usage-patterns)
- [Examples](#examples)

## Overview

The Downloader architecture provides a generic framework for:
- **Batch Downloads**: Download multiple items concurrently in configurable batches
- **Automatic Retries**: Retry transient failures with exponential backoff
- **Partial Failure Handling**: Only retry failed items, not entire batches
- **Flexible Error Handling**: Distinguish between retryable and permanent errors

## Architecture

### Component Relationships

```
┌─────────────────────────────────────────────────────────────┐
│                    BatchDownloader<D>                       │
│  ┌───────────────────────────────────────────────────────┐ │
│  │ • Orchestrates batch downloads                        │ │
│  │ • Manages retry logic with backoff                    │ │
│  │ • Handles partial batch failures                      │ │
│  │ • Caches successful results                           │ │
│  └───────────────────────────────────────────────────────┘ │
└──────────────────────────┬──────────────────────────────────┘
                           │ uses
                           ↓
               ┌───────────────────────┐
               │   Downloader Trait    │
               │  ┌─────────────────┐  │
               │  │ download(key)   │  │
               │  │  → Result<T, E> │  │
               │  └─────────────────┘  │
               └───────────────────────┘
                           ↑
                           │ implement
         ┌─────────────────┴─────────────────┐
         │                                   │
┌────────┴──────────┐            ┌──────────┴─────────┐
│ GatewayDownloader │            │ YourDownloader     │
│ (built-in)        │            │ (custom)           │
└───────────────────┘            └────────────────────┘
```

### Data Flow

```
User Request
    │
    ↓
┌────────────────────────────────────────────────────────────┐
│ BatchDownloader::download(keys)                            │
│                                                             │
│  Split keys into batches: [batch1, batch2, ...]           │
└──────────────────┬─────────────────────────────────────────┘
                   │
                   ↓
        ┌──────────────────────┐
        │  For each batch:     │
        │  (processed SEQUEN-  │
        │   TIALLY)            │
        └──────────┬───────────┘
                   │
                   ↓
┌──────────────────────────────────────────────────────────────┐
│ download_batch_with_retry(batch_keys)                        │
│                                                               │
│  ┌────────────────────────────────────────────────────┐     │
│  │ Loop until all succeed or retries exhausted:       │     │
│  │                                                     │     │
│  │  1. Download items CONCURRENTLY                    │     │
│  │     ┌─────────────────────────────────────┐        │     │
│  │     │  For each key in batch:             │        │     │
│  │     │    downloader.download(key)         │        │     │
│  │     │      (runs concurrently)            │        │     │
│  │     └─────────────────────────────────────┘        │     │
│  │                                                     │     │
│  │  2. Classify results:                              │     │
│  │     • Ok(value)    → Cache and continue            │     │
│  │     • Retry(error) → Add to failed_keys            │     │
│  │     • Err(error)   → Return error immediately      │     │
│  │                                                     │     │
│  │  3. If failed_keys.is_empty():                     │     │
│  │     → All succeeded, break loop                    │     │
│  │                                                     │     │
│  │  4. Wait for backoff delay                         │     │
│  │                                                     │     │
│  │  5. Retry only failed_keys                         │     │
│  │                                                     │     │
│  └────────────────────────────────────────────────────┘     │
│                                                               │
│  Return cached results in original order                     │
└───────────────────────────────────────────────────────────────┘
                   │
                   ↓
        Return all downloaded values
```

## Core Components

### 1. `Downloader` Trait

The trait you implement to define how individual items are downloaded.

```rust
pub trait Downloader {
    type Key: Clone + PartialEq + Eq + Send + Sync;
    type Value: Send + Sync;
    type Error: std::error::Error + Send;

    fn download(
        &self,
        key: &Self::Key,
    ) -> impl Future<Output = DownloaderResult<Self::Value, Self::Error>> + Send;
}
```

**Key Points:**
- `Key`: Identifies what to download (e.g., block number, URL, ID)
- `Value`: The downloaded data
- `Error`: Your custom error type
- Returns `DownloaderResult` (not regular `Result`)

### 2. `DownloaderResult` Enum

A three-way result type that enables smart retry logic:

```rust
pub enum DownloaderResult<T, E> {
    Ok(T),       // Success
    Err(E),      // Permanent failure (do NOT retry)
    Retry(E),    // Transient failure (should retry)
}
```

**Decision Tree:**

```
Download Attempt
       │
       ↓
   Succeeded?
    ┌───┴───┐
   YES     NO
    │       │
    ↓       ↓
Return  Is Error
  Ok    Retryable?
         ┌───┴───┐
        YES     NO
         │       │
         ↓       ↓
      Return  Return
      Retry    Err
```

### 3. `BatchDownloader`

Orchestrates batch downloads with retry logic.

```rust
pub struct BatchDownloader<D, B = ExponentialBuilder> {
    downloader: D,      // Your Downloader implementation
    batch_size: usize,  // Max items per batch
    backoff: B,         // Retry strategy
}
```

**Configuration:**

```rust
// Default: exponential backoff with 3s-60s delays, max 3 retries
let downloader = BatchDownloader::new(my_downloader, batch_size);

// Custom backoff
let downloader = BatchDownloader::new(my_downloader, batch_size)
    .backoff(ExponentialBuilder::default()
        .with_min_delay(Duration::from_secs(1))
        .with_max_delay(Duration::from_secs(30))
        .with_max_times(5));
```

## How It Works

### Batch Processing

Batches are processed **sequentially** (one after another), but downloads **within** each batch happen **concurrently**.

```
Timeline:
├─ Batch 1 [items 1-10] ─────────────┤
                                      ├─ Batch 2 [items 11-20] ────────┤
                                                                        ├─ Batch 3 [items 21-25] ──┤

Within Batch 1 (concurrent):
├─ download(1)  ─────┤
├─ download(2)  ──┤
├─ download(3)  ────────┤
├─ download(4)  ───┤
...
├─ download(10) ─────────┤
```

**Why sequential batches?**
- Prevents overwhelming the server/network
- Provides backpressure
- Easier to manage memory usage

### Partial Batch Failure Handling

When some items in a batch fail with `Retry`, only those items are retried:

```
Initial Batch: [1, 2, 3, 4, 5]
Results:       [Ok, Ok, Retry, Ok, Retry]
                         ↓           ↓
Retry Attempt: [3, 5]  (only these two)
Results:       [Ok, Ok]
                ↓   ↓
Final Results: [val1, val2, val3, val4, val5]
               └───────────────────────────────┘
                    (in original order)
```

**Benefits:**
- Efficient: No redundant downloads
- Fast: Successful results available immediately
- Resilient: Partial failures don't waste work

### Retry Backoff

Default exponential backoff strategy:

```
Attempt 1: Immediate
           ↓ (fails with Retry)
Attempt 2: Wait 3s
           ↓ (fails with Retry)
Attempt 3: Wait 6s
           ↓ (fails with Retry)
Attempt 4: Wait 12s
           ↓ (fails with Retry)
Error: Retries exhausted
```

## Implementation Guide

### Step 1: Define Your Types

```rust
use katana_stage::downloader::{Downloader, DownloaderResult};

// Your key type
#[derive(Clone, PartialEq, Eq)]
struct MyKey(u64);

// Your value type
struct MyValue {
    data: Vec<u8>,
}

// Your error type
#[derive(Debug, thiserror::Error)]
enum MyError {
    #[error("Rate limited")]
    RateLimited,
    #[error("Not found")]
    NotFound,
    #[error("Network error: {0}")]
    Network(String),
}
```

### Step 2: Implement `Downloader`

```rust
struct MyDownloader {
    client: HttpClient,
}

impl Downloader for MyDownloader {
    type Key = MyKey;
    type Value = MyValue;
    type Error = MyError;

    async fn download(&self, key: &Self::Key) -> DownloaderResult<Self::Value, Self::Error> {
        match self.client.get(format!("/api/items/{}", key.0)).await {
            // Success
            Ok(data) => DownloaderResult::Ok(MyValue { data }),

            // Transient errors - RETRY
            Err(e) if e.is_timeout() => DownloaderResult::Retry(MyError::Network(e.to_string())),
            Err(e) if e.status() == Some(429) => DownloaderResult::Retry(MyError::RateLimited),
            Err(e) if e.status() == Some(503) => DownloaderResult::Retry(MyError::Network(e.to_string())),

            // Permanent errors - DO NOT RETRY
            Err(e) if e.status() == Some(404) => DownloaderResult::Err(MyError::NotFound),
            Err(e) => DownloaderResult::Err(MyError::Network(e.to_string())),
        }
    }
}
```

### Step 3: Use with `BatchDownloader`

```rust
let downloader = MyDownloader::new(client);
let batch_downloader = BatchDownloader::new(downloader, 10);

let keys = vec![MyKey(1), MyKey(2), MyKey(3), /* ... */];
let values = batch_downloader.download(keys).await?;
```

### When to Return Each Variant

| Scenario | Return | Example |
|----------|--------|---------|
| Success | `Ok(value)` | Data successfully retrieved |
| Rate limit hit | `Retry(error)` | HTTP 429 |
| Temporary network issue | `Retry(error)` | Connection timeout, DNS failure |
| Server overloaded | `Retry(error)` | HTTP 503 Service Unavailable |
| Server error | `Retry(error)` | HTTP 500 Internal Server Error |
| Resource doesn't exist | `Err(error)` | HTTP 404 Not Found |
| Invalid request | `Err(error)` | HTTP 400 Bad Request |
| Unauthorized | `Err(error)` | HTTP 401/403 |
| Parse/validation error | `Err(error)` | Malformed data |

**General Rule:**
- `Retry`: Error might succeed if we try again later
- `Err`: Error won't be fixed by retrying

## Usage Patterns

### Pattern 1: Simple Gateway Client

```rust
use katana_stage::downloader::{BatchDownloader, Downloader, DownloaderResult};
use katana_gateway::client::Client as GatewayClient;
use katana_primitives::block::BlockNumber;

struct BlockDownloader {
    gateway: GatewayClient,
}

impl Downloader for BlockDownloader {
    type Key = BlockNumber;
    type Value = StateUpdateWithBlock;
    type Error = katana_gateway::client::Error;

    async fn download(&self, key: &Self::Key) -> DownloaderResult<Self::Value, Self::Error> {
        match self.gateway.get_state_update_with_block((*key).into()).await {
            Ok(data) => DownloaderResult::Ok(data),
            Err(err) if err.is_rate_limited() => DownloaderResult::Retry(err),
            Err(err) => DownloaderResult::Err(err),
        }
    }
}

// Usage
let downloader = BatchDownloader::new(BlockDownloader::new(gateway), 50);
let blocks = downloader.download(block_numbers).await?;
```

### Pattern 2: With Custom Retry Strategy

```rust
use backon::{ConstantBuilder, ExponentialBuilder};
use std::time::Duration;

// Constant backoff (same delay each retry)
let downloader = BatchDownloader::new(my_downloader, 10)
    .backoff(ConstantBuilder::default()
        .with_delay(Duration::from_secs(5))
        .with_max_times(3));

// Custom exponential backoff
let downloader = BatchDownloader::new(my_downloader, 10)
    .backoff(ExponentialBuilder::default()
        .with_min_delay(Duration::from_millis(100))
        .with_max_delay(Duration::from_secs(10))
        .with_factor(1.5)
        .with_max_times(5));
```

### Pattern 3: Wrapping in Higher-Level API

```rust
pub trait BlockDownloader: Send + Sync {
    fn download_blocks(
        &self,
        from: BlockNumber,
        to: BlockNumber,
    ) -> impl Future<Output = Result<Vec<Block>, Error>> + Send;
}

pub struct BatchBlockDownloader<D> {
    inner: BatchDownloader<D>,
}

impl<D> BlockDownloader for BatchBlockDownloader<D>
where
    D: Downloader<Key = BlockNumber, Value = Block, Error = Error>,
    D: Send + Sync,
{
    async fn download_blocks(&self, from: BlockNumber, to: BlockNumber) -> Result<Vec<Block>, Error> {
        let keys = (from..=to).collect::<Vec<_>>();
        self.inner.download(keys).await
    }
}
```

### Pattern 4: Testing with Mocks

```rust
#[derive(Clone)]
struct MockDownloader {
    responses: Arc<Mutex<HashMap<u64, Vec<DownloaderResult<String, Error>>>>>,
    attempts: Arc<Mutex<HashMap<u64, Arc<AtomicUsize>>>>,
}

impl Downloader for MockDownloader {
    type Key = u64;
    type Value = String;
    type Error = Error;

    async fn download(&self, key: &Self::Key) -> DownloaderResult<Self::Value, Self::Error> {
        let attempt = self.attempts
            .lock().unwrap()
            .entry(*key)
            .or_insert_with(|| Arc::new(AtomicUsize::new(0)))
            .fetch_add(1, Ordering::SeqCst);

        self.responses.lock().unwrap()
            .get(key)
            .and_then(|results| results.get(attempt).cloned())
            .unwrap_or(DownloaderResult::Err(Error::NoResponse))
    }
}

// Test retry behavior
let downloader = MockDownloader::new()
    .with_response(1, vec![
        DownloaderResult::Retry(Error::Temporary),
        DownloaderResult::Ok("success".to_string()),
    ]);

let batch = BatchDownloader::new(downloader.clone(), 10);
let result = batch.download(vec![1]).await.unwrap();
assert_eq!(downloader.get_attempts(1), 2); // Initial + 1 retry
```

## Examples

See the `examples/` directory for runnable code:

- **`simple_downloader.rs`**: Basic HTTP downloader implementation
- **`custom_retry_downloader.rs`**: Advanced retry strategies
- **`mock_downloader.rs`**: Testing patterns and mocks

Run examples with:
```bash
cargo run --example simple_downloader
```

## Best Practices

### ✅ DO

- Implement idempotent downloads when possible
- Use `Retry` for transient network/server issues
- Use `Err` for permanent failures (404, validation errors)
- Configure appropriate batch sizes for your use case
- Test retry behavior with mock implementations
- Use exponential backoff for production systems

### ❌ DON'T

- Return `Retry` for errors that won't be fixed by retrying
- Make downloads stateful/non-idempotent
- Use excessively large batch sizes
- Ignore rate limiting signals from servers
- Skip testing partial failure scenarios

## Performance Tuning

### Batch Size

```rust
// Small batches (1-10): Better for rate-limited APIs
let downloader = BatchDownloader::new(my_downloader, 5);

// Medium batches (10-50): Good default for most APIs
let downloader = BatchDownloader::new(my_downloader, 25);

// Large batches (50-100): For high-throughput scenarios
let downloader = BatchDownloader::new(my_downloader, 100);
```

**Considerations:**
- Smaller batches → Less memory, better rate limiting
- Larger batches → Higher throughput, more memory

### Retry Strategy

```rust
// Aggressive (fast recovery, more server load)
.backoff(ExponentialBuilder::default()
    .with_min_delay(Duration::from_millis(500))
    .with_max_times(5))

// Balanced (default)
.backoff(ExponentialBuilder::default()
    .with_min_delay(Duration::from_secs(3))
    .with_max_times(3))

// Conservative (gentle on server)
.backoff(ExponentialBuilder::default()
    .with_min_delay(Duration::from_secs(10))
    .with_max_delay(Duration::from_secs(120))
    .with_max_times(2))
```

## FAQ

**Q: Why not use regular `Result<T, E>`?**
A: We need to distinguish between retryable and permanent errors. Regular `Result` can't express this.

**Q: Can I retry the entire batch instead of individual items?**
A: No, `BatchDownloader` always retries only failed items. This is more efficient.

**Q: How do I handle authentication/headers?**
A: Store authenticated clients in your `Downloader` struct:
```rust
struct MyDownloader {
    client: AuthenticatedClient,
}
```

**Q: Can batches be processed in parallel?**
A: No, batches are sequential to provide backpressure. Items within a batch are concurrent.

**Q: What happens if I return `Err` during a retry?**
A: The entire download operation fails immediately, even if some items succeeded.

**Q: How do I customize timeout behavior?**
A: Set timeouts on your HTTP client, then treat timeout errors as `Retry` in your implementation.
