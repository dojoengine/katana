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
        let mut results = Vec::with_capacity(keys.len());

        let mut remaining_keys = keys.clone();
        let mut backoff = self.backoff.clone().build();

        loop {
            let batch_result = self.download_batch(&remaining_keys).await;

            let mut failed_keys = Vec::with_capacity(remaining_keys.len());
            let mut last_error = None;

            for (key, result) in remaining_keys.iter().zip(batch_result.into_iter()) {
                let (key_idx, key) =
                    keys.iter().enumerate().find(|(_, k)| *k == key).expect("qed; must exist");

                match result {
                    // cache the result for successful requests
                    DownloaderResult::Ok(value) => results.insert(key_idx, value),
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

        Ok(results)
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
