//! Block downloading abstractions and implementations for the Blocks stage.
//!
//! This module defines the [`BlockDownloader`] trait, which provides a stage-specific
//! interface for downloading block data. The trait is designed to be flexible and can
//! be implemented in various ways depending on the download strategy and data source
//! (e.g., gateway-based, JSON-RPC-based, P2P-based, or custom implementations).
//!
//! [`BatchBlockDownloader`] is one such implementation that leverages the generic
//! [`BatchDownloader`](crate::downloader::BatchDownloader) utility for concurrent
//! downloads with retry logic. This is suitable for many use cases but is not the
//! only way to implement block downloading.

use std::future::Future;

use katana_primitives::block::BlockNumber;

use super::BlockData;
use crate::downloader::{BatchDownloader, Downloader};

/// Trait for downloading block data.
///
/// This trait provides a stage-specific abstraction for downloading blocks, allowing different
/// implementations (e.g., gateway-based, JSON-RPC-based, custom strategies) to be used with the
/// [`Blocks`](crate::blocks::Blocks) stage.
///
/// Implementors can use any download strategy they choose, including but not limited to the
/// [`BatchDownloader`](crate::downloader::BatchDownloader) utility provided by this crate.
pub trait BlockDownloader: Send + Sync {
    type Error: std::error::Error + Send + Sync + 'static;

    /// Downloads blocks for the given block number range and returns them as [`BlockData`].
    fn download_blocks(
        &self,
        from: BlockNumber,
        to: BlockNumber,
    ) -> impl Future<Output = Result<Vec<BlockData>, Self::Error>> + Send;
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

impl<D, V> BlockDownloader for BatchBlockDownloader<D>
where
    D: Downloader<Key = BlockNumber, Value = V>,
    D: Send + Sync,
    D::Error: Send + Sync + 'static,
    V: Into<BlockData> + Send + Sync,
{
    type Error = D::Error;

    async fn download_blocks(
        &self,
        from: BlockNumber,
        to: BlockNumber,
    ) -> Result<Vec<BlockData>, Self::Error> {
        let block_keys = (from..=to).collect::<Vec<BlockNumber>>();
        let results = self.inner.download(block_keys).await?;
        Ok(results.into_iter().map(Into::into).collect())
    }
}

pub mod gateway {
    use katana_gateway_client::Client as GatewayClient;
    use katana_gateway_types::StateUpdateWithBlock;
    use katana_primitives::block::BlockNumber;

    use super::BatchBlockDownloader;
    use crate::downloader::{Downloader, DownloaderResult};

    impl BatchBlockDownloader<GatewayDownloader> {
        /// Create a new [`BatchBlockDownloader`] using the Starknet gateway for downloading
        /// blocks.
        pub fn new_gateway(
            client: GatewayClient,
            batch_size: usize,
        ) -> BatchBlockDownloader<GatewayDownloader> {
            Self::new(GatewayDownloader::new(client), batch_size)
        }
    }

    /// Internal [`Downloader`] implementation that uses the sequencer gateway for downloading a
    /// block.
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
        type Error = katana_gateway_client::Error;

        #[allow(clippy::manual_async_fn)]
        fn download(
            &self,
            key: &Self::Key,
        ) -> impl std::future::Future<Output = DownloaderResult<Self::Value, Self::Error>> {
            use katana_gateway_client::Error as GatewayClientError;
            use tracing::error;

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

pub mod json_rpc {
    use katana_primitives::block::{BlockIdOrTag, BlockNumber};
    use katana_starknet::rpc::Client as JsonRpcClient;
    use tracing::error;

    use super::{BlockData, BlockDownloader};

    /// A [`BlockDownloader`] that fetches blocks via JSON-RPC.
    ///
    /// This downloads blocks one at a time sequentially using
    /// `starknet_getBlockWithReceipts` and `starknet_getStateUpdate`.
    #[derive(Debug)]
    pub struct JsonRpcBlockDownloader {
        client: JsonRpcClient,
    }

    impl JsonRpcBlockDownloader {
        pub fn new(client: JsonRpcClient) -> Self {
            Self { client }
        }
    }

    #[derive(Debug, thiserror::Error)]
    pub enum Error {
        #[error(transparent)]
        Rpc(#[from] katana_starknet::rpc::Error),

        #[error(transparent)]
        Other(#[from] anyhow::Error),
    }

    impl BlockDownloader for JsonRpcBlockDownloader {
        type Error = Error;

        async fn download_blocks(
            &self,
            from: BlockNumber,
            to: BlockNumber,
        ) -> Result<Vec<BlockData>, Self::Error> {
            let mut blocks = Vec::with_capacity((to - from + 1) as usize);

            for block_num in from..=to {
                let block_id = BlockIdOrTag::Number(block_num);

                let (block_resp, state_update) = tokio::try_join!(
                    async {
                        self.client
                            .get_block_with_receipts(block_id)
                            .await
                            .inspect_err(|e| {
                                error!(
                                    block = %block_num,
                                    error = %e,
                                    "Error downloading block via JSON-RPC."
                                )
                            })
                            .map_err(Error::from)
                    },
                    async {
                        self.client
                            .get_state_update(block_id)
                            .await
                            .inspect_err(|e| {
                                error!(
                                    block = %block_num,
                                    error = %e,
                                    "Error downloading state update via JSON-RPC."
                                )
                            })
                            .map_err(Error::from)
                    },
                )?;

                let block_data = BlockData::from_rpc(block_resp, state_update)?;
                blocks.push(block_data);
            }

            Ok(blocks)
        }
    }
}
