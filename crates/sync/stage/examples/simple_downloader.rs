//! Simple HTTP Downloader Example
//!
//! This example demonstrates the basic usage of the `Downloader` trait and `BatchDownloader`
//! for downloading data from a REST API with automatic retry logic.
//!
//! # What This Example Shows
//!
//! 1. How to implement the `Downloader` trait for a simple HTTP client
//! 2. How to classify errors as retryable vs. permanent
//! 3. How to use `BatchDownloader` to download multiple items
//! 4. Basic error handling patterns
//!
//! # Running This Example
//!
//! ```bash
//! cargo run --example simple_downloader
//! ```

use std::fmt;
use std::time::Duration;

use katana_stage::downloader::{BatchDownloader, Downloader, DownloaderResult};

/// Simulates an HTTP client for downloading user data from a REST API.
///
/// In a real application, you would use an actual HTTP client like `reqwest`.
#[derive(Clone)]
struct HttpClient;

impl HttpClient {
    fn new() -> Self {
        Self {}
    }

    /// Simulates an HTTP GET request to fetch user data.
    ///
    /// This mock implementation demonstrates different error scenarios:
    /// - User IDs 1-10: Success
    /// - User ID 500: Rate limited (retryable)
    /// - User ID 503: Server error (retryable)
    /// - User ID 404: Not found (permanent)
    async fn get(&self, path: &str) -> Result<String, HttpError> {
        // Simulate network delay
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Parse user ID from path (e.g., "/users/123")
        let user_id = path
            .strip_prefix("/users/")
            .and_then(|s| s.parse::<u64>().ok())
            .ok_or(HttpError::InvalidRequest)?;

        // Simulate different response scenarios
        match user_id {
            1..=10 => Ok(format!(
                r#"{{"id": {}, "name": "User {}", "email": "user{}@example.com"}}"#,
                user_id, user_id, user_id
            )),
            404 => Err(HttpError::NotFound),
            500 => Err(HttpError::RateLimited),
            503 => Err(HttpError::ServerError("Service temporarily unavailable".to_string())),
            _ => Ok(format!(
                r#"{{"id": {}, "name": "User {}", "email": "user{}@example.com"}}"#,
                user_id, user_id, user_id
            )),
        }
    }
}

/// HTTP error types that can occur during downloads.
#[derive(Debug, Clone, thiserror::Error)]
enum HttpError {
    #[error("Rate limited - too many requests")]
    RateLimited,

    #[error("Not found")]
    NotFound,

    #[error("Server error: {0}")]
    ServerError(String),

    #[error("Invalid request")]
    InvalidRequest,

    #[error("Network timeout")]
    Timeout,
}

/// A user ID that serves as the key for downloading user data.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct UserId(u64);

impl fmt::Display for UserId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// User data returned from the API.
#[derive(Debug, Clone)]
struct User {
    id: u64,
    name: String,
    email: String,
}

impl User {
    /// Parse user data from JSON string.
    ///
    /// In a real application, you would use a proper JSON parser like `serde_json`.
    fn from_json(json: &str, expected_id: u64) -> Result<Self, HttpError> {
        // Simple mock parsing - just verify it contains the expected ID
        if json.contains(&format!(r#""id": {}"#, expected_id)) {
            Ok(User {
                id: expected_id,
                name: format!("User {}", expected_id),
                email: format!("user{}@example.com", expected_id),
            })
        } else {
            Err(HttpError::InvalidRequest)
        }
    }
}

/// Implementation of the `Downloader` trait for fetching users from an HTTP API.
///
/// This demonstrates the core pattern:
/// 1. Store your client/connection in the struct
/// 2. Implement `download` to fetch a single item
/// 3. Classify errors as `Ok`, `Retry`, or `Err`
#[derive(Clone)]
struct UserDownloader {
    client: HttpClient,
}

impl UserDownloader {
    fn new() -> Self {
        Self { client: HttpClient::new() }
    }
}

impl Downloader for UserDownloader {
    type Key = UserId;
    type Value = User;
    type Error = HttpError;

    async fn download(&self, key: &Self::Key) -> DownloaderResult<Self::Value, Self::Error> {
        println!("📥 Downloading user {}", key.0);

        let path = format!("/users/{}", key.0);

        match self.client.get(&path).await {
            // Success - parse and return the user data
            Ok(json) => match User::from_json(&json, key.0) {
                Ok(user) => {
                    println!("✅ Successfully downloaded user {}", key.0);
                    DownloaderResult::Ok(user)
                }
                Err(err) => {
                    println!("❌ Failed to parse user {}: {}", key.0, err);
                    DownloaderResult::Err(err)
                }
            },

            // Transient errors - should retry
            Err(HttpError::RateLimited) => {
                println!("⏳ Rate limited on user {} - will retry", key.0);
                DownloaderResult::Retry(HttpError::RateLimited)
            }
            Err(HttpError::ServerError(msg)) => {
                println!("⏳ Server error for user {} - will retry: {}", key.0, msg);
                DownloaderResult::Retry(HttpError::ServerError(msg))
            }
            Err(HttpError::Timeout) => {
                println!("⏳ Timeout for user {} - will retry", key.0);
                DownloaderResult::Retry(HttpError::Timeout)
            }

            // Permanent errors - do not retry
            Err(HttpError::NotFound) => {
                println!("❌ User {} not found - will NOT retry", key.0);
                DownloaderResult::Err(HttpError::NotFound)
            }
            Err(HttpError::InvalidRequest) => {
                println!("❌ Invalid request for user {} - will NOT retry", key.0);
                DownloaderResult::Err(HttpError::InvalidRequest)
            }
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("🚀 Simple Downloader Example\n");
    println!("This example demonstrates basic usage of the Downloader trait");
    println!("for downloading user data from a REST API.\n");

    // Create a downloader with a batch size of 5
    let downloader = UserDownloader::new();
    let batch_downloader = BatchDownloader::new(downloader, 5);

    println!("Configuration:");
    println!("  • Batch size: 5 users per batch");
    println!("  • Retry strategy: Exponential backoff (3s, 6s, 12s)");
    println!("  • Max retries: 3\n");

    // Example 1: Download a small batch of users
    println!("═══════════════════════════════════════════════════════════");
    println!("Example 1: Download users 1-5 (all should succeed)");
    println!("═══════════════════════════════════════════════════════════\n");

    let user_ids = vec![UserId(1), UserId(2), UserId(3), UserId(4), UserId(5)];

    let users = batch_downloader.download(user_ids).await?;

    println!("\n✅ Successfully downloaded {} users:", users.len());
    for user in &users {
        println!("   • User {}: {} ({})", user.id, user.name, user.email);
    }

    // Example 2: Download with some transient failures
    println!("\n═══════════════════════════════════════════════════════════");
    println!("Example 2: Download users including one with rate limit");
    println!("═══════════════════════════════════════════════════════════\n");
    println!("User 500 will trigger a rate limit (retryable error)");
    println!("The BatchDownloader will automatically retry it.\n");

    let user_ids = vec![UserId(1), UserId(500), UserId(3)];

    match batch_downloader.download(user_ids).await {
        Ok(users) => {
            println!("\n✅ All users downloaded successfully after retries:");
            for user in &users {
                println!("   • User {}: {}", user.id, user.name);
            }
        }
        Err(e) => {
            println!("\n❌ Download failed: {}", e);
            println!("   (This happens when retries are exhausted)");
        }
    }

    // Example 3: Permanent failure (404 Not Found)
    println!("\n═══════════════════════════════════════════════════════════");
    println!("Example 3: Download with permanent failure");
    println!("═══════════════════════════════════════════════════════════\n");
    println!("User 404 will trigger a Not Found error (non-retryable)");
    println!("The BatchDownloader will fail immediately.\n");

    let user_ids = vec![UserId(1), UserId(404), UserId(3)];

    match batch_downloader.download(user_ids).await {
        Ok(users) => {
            println!("\n✅ Unexpected success: {} users", users.len());
        }
        Err(e) => {
            println!("\n❌ Download failed immediately: {}", e);
            println!("   This is expected for permanent errors like 404.");
        }
    }

    println!("\n═══════════════════════════════════════════════════════════");
    println!("✨ Example completed!");
    println!("═══════════════════════════════════════════════════════════\n");

    println!("Key Takeaways:");
    println!("  1. Implement Downloader trait for your data source");
    println!("  2. Return Ok for success, Retry for transient errors, Err for permanent errors");
    println!("  3. BatchDownloader handles retry logic automatically");
    println!("  4. Batches are processed sequentially, items within batches concurrently");
    println!("  5. Only failed items are retried, not entire batches\n");

    Ok(())
}
