use std::future::Future;

use anyhow::Result;
use katana_gateway::client::Client as GatewayClient;
use katana_gateway::types::StateUpdateWithBlock;
use katana_primitives::block::BlockNumber;

use crate::downloader::{BatchDownloader, Downloader};

/// Trait for downloading block data.
///
/// This trait abstracts the mechanism for downloading blocks, allowing different
/// implementations (e.g., gateway-based, P2P-based) to be used with the [`Blocks`] stage.
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

/// [`BatchDownloader`]-based implementation of [`BlockDownloader`].
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
        // convert the range to a list of block keys
        let block_keys = (from..=to).collect::<Vec<BlockNumber>>();
        self.inner.download(block_keys)
    }
}

mod impls {
    use std::future::Future;

    use katana_gateway::client::Client as GatewayClient;
    use katana_gateway::types::StateUpdateWithBlock;
    use katana_primitives::block::BlockNumber;

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
            async {
                match self.gateway.get_state_update_with_block((*key).into()).await {
                    Ok(data) => DownloaderResult::Ok(data),
                    Err(err) if err.is_rate_limited() => DownloaderResult::Retry(err),
                    Err(err) => DownloaderResult::Err(err),
                }
            }
        }
    }
}
