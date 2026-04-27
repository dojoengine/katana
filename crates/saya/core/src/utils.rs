use std::{future::Future, time::Duration};

use tracing::debug;

pub async fn retry_with_backoff<F, Fut, T, E>(
    operation: F,
    label: &str,
    max_attempts: u32,
    base_delay: Duration,
) -> Result<T, E>
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<T, E>>,
    E: std::fmt::Display,
{
    let mut attempts = 0;
    loop {
        match operation().await {
            Ok(value) => return Ok(value),
            Err(e) => {
                attempts += 1;
                if attempts >= max_attempts {
                    return Err(e);
                }
                let delay = base_delay * attempts;
                debug!(
                    "Operation {} failed on attempt {}/{}: {}. Retrying after {:?}...",
                    label, attempts, max_attempts, e, delay
                );
                tokio::time::sleep(delay).await;
            }
        }
    }
}
