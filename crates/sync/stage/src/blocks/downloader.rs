//! Block downloading abstractions and implementations for the Blocks stage.
//!
//! This module defines the [`BlockDownloader`] trait, which provides a stage-specific
//! interface for downloading block data. The trait is designed to be flexible and can
//! be implemented in various ways depending on the download strategy and data source
//! (e.g., gateway-based, P2P-based, or custom implementations).
//!
//! [`BatchBlockDownloader`] is one such implementation that leverages the generic
//! [`BatchDownloader`](crate::downloader::BatchDownloader) utility for concurrent
//! downloads with retry logic. This is suitable for many use cases but is not the
//! only way to implement block downloading.

use std::future::Future;

use anyhow::Result;
use katana_gateway::client::Client as GatewayClient;
use katana_gateway::types::StateUpdateWithBlock;
use katana_primitives::block::BlockNumber;
use tracing::{info_span, Instrument};

use crate::downloader::{BatchDownloader, Downloader};

/// Trait for downloading block data.
///
/// This trait provides a stage-specific abstraction for downloading blocks, allowing different
/// implementations (e.g., gateway-based, P2P-based, custom strategies) to be used with the
/// [`Blocks`](crate::blocks::Blocks) stage.
///
/// Implementors can use any download strategy they choose, including but not limited to the
/// [`BatchDownloader`](crate::downloader::BatchDownloader) utility provided by this crate.
///
/// Currently, it's still coupled with the gateway (as seen by the types used in the trait method)
/// but this level of abstraction will allow for easier testing and preparation for future
/// flexibility.
pub trait BlockDownloader: Send + Sync {
    /// Downloads blocks for the given block numbers.
    fn download_blocks(
        &self,
        from: BlockNumber,
        to: BlockNumber,
    ) -> impl Future<Output = Result<Vec<StateUpdateWithBlock>, katana_gateway::client::Error>> + Send;
}

///////////////////////////////////////////////////////////////////////////////////
// Implementations
///////////////////////////////////////////////////////////////////////////////////

/// An implementation of [`BlockDownloader`] that uses the [`BatchDownloader`] utility.
///
/// This implementation leverages the generic
/// [`BatchDownloader`](crate::downloader::BatchDownloader) to download blocks concurrently in
/// batches with automatic retry logic. It's a straightforward approach suitable for many scenarios.
#[derive(Debug)]
pub struct BatchBlockDownloader<D> {
    inner: BatchDownloader<D>,
}

impl<D> BatchBlockDownloader<D> {
    /// Create a new [`BatchBlockDownloader`] with the given [`Downloader`] and batch size.
    pub fn new(downloader: D, batch_size: usize) -> Self {
        Self { inner: BatchDownloader::new(downloader, batch_size) }
    }
}

impl BatchBlockDownloader<impls::GatewayDownloader> {
    /// Create a new [`BatchBlockDownloader`] using the Starknet gateway for downloading blocks.
    pub fn new_gateway(
        client: GatewayClient,
        batch_size: usize,
    ) -> BatchBlockDownloader<impls::GatewayDownloader> {
        Self::new(impls::GatewayDownloader::new(client), batch_size)
    }
}

impl<D> BlockDownloader for BatchBlockDownloader<D>
where
    D: Downloader<
        Key = BlockNumber,
        Value = StateUpdateWithBlock,
        Error = katana_gateway::client::Error,
    >,
    D: Send + Sync,
{
    fn download_blocks(
        &self,
        from: BlockNumber,
        to: BlockNumber,
    ) -> impl Future<Output = Result<Vec<StateUpdateWithBlock>, katana_gateway::client::Error>> + Send
    {
        async move {
            // convert the range to a list of block keys
            let block_keys = (from..=to).collect::<Vec<BlockNumber>>();
            self.inner.download(block_keys).await
        }
        .instrument(info_span!("download_blocks", %from, %to))
    }
}

mod impls {
    use std::future::Future;

    use katana_gateway::client::{Client as GatewayClient, Error as GatewayClientError};
    use katana_gateway::types::StateUpdateWithBlock;
    use katana_primitives::block::BlockNumber;
    use tracing::error;

    use crate::downloader::{Downloader, DownloaderResult};

    /// Internal [`Downloader`] implementation that uses the sequencer gateway for downloading a
    /// block. This is used by [`GatewayBlockDownloader`].
    #[derive(Debug)]
    pub struct GatewayDownloader {
        gateway: GatewayClient,
    }

    impl GatewayDownloader {
        pub fn new(gateway: GatewayClient) -> Self {
            Self { gateway }
        }
    }

    impl Downloader for GatewayDownloader {
        type Key = BlockNumber;
        type Value = StateUpdateWithBlock;
        type Error = katana_gateway::client::Error;

        #[allow(clippy::manual_async_fn)]
        fn download(
            &self,
            key: &Self::Key,
        ) -> impl Future<Output = DownloaderResult<Self::Value, Self::Error>> {
            trace!(block = %key, "Downloading block.");
            async {
                match self.gateway.get_state_update_with_block((*key).into()).await.inspect_err(
                    |error| error!(block = %*key, ?error, "Error downloading block from gateway."),
                ) {
                    Ok(data) => DownloaderResult::Ok(data),
                    Err(err) => match err {
                        GatewayClientError::RateLimited
                        | GatewayClientError::UnknownFormat { .. } => DownloaderResult::Retry(err),
                        _ => DownloaderResult::Err(err),
                    },
                }
            }
        }
    }
}
