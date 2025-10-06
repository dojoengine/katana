use std::future::Future;

use anyhow::Result;
use futures::future::BoxFuture;
use katana_gateway::client::Client as SequencerGateway;
use katana_gateway::types::ContractClass;
use katana_primitives::block::BlockNumber;
use katana_primitives::class::ClassHash;
use katana_provider::api::contract::ContractClassWriter;
use katana_provider::api::state_update::StateUpdateProvider;
use katana_provider::api::ProviderError;
use katana_rpc_types::class::ConversionError;
use tracing::{debug, error};

use super::{Stage, StageExecutionInput, StageResult};
use crate::downloader::{BatchDownloader, Downloader, DownloaderResult};

/// A stage for downloading and storing contract classes.
#[derive(Debug)]
pub struct Classes<P> {
    provider: P,
    downloader: BatchDownloader<ClassDownloader>,
}

impl<P> Classes<P> {
    /// Create a new Classes stage using the Feeder Gateway downloader.
    pub fn new(provider: P, gateway: SequencerGateway, batch_size: usize) -> Self {
        let downloader = ClassDownloader { gateway };
        let downloader = BatchDownloader::new(downloader, batch_size);
        Self { provider, downloader }
    }
}

impl<P: StateUpdateProvider> Classes<P> {
    fn get_declared_classes(
        &self,
        from_block: BlockNumber,
        to_block: BlockNumber,
    ) -> Result<Vec<ClassDownloadKey>, Error> {
        let mut classes_keys: Vec<ClassDownloadKey> = Vec::new();

        for block in from_block..=to_block {
            // get the classes declared at block `i`
            let class_hashes = self
                .provider
                .declared_classes(block.into())?
                .ok_or(Error::MissingBlockDeclaredClasses { block })?;

            // collect classes to fetch with their block numbers
            for class_hash in class_hashes.keys().copied() {
                classes_keys.push(ClassDownloadKey { class_hash, block });
            }
        }

        Ok(classes_keys)
    }
}

impl<P> Stage for Classes<P>
where
    P: StateUpdateProvider + ContractClassWriter,
{
    fn id(&self) -> &'static str {
        "Classes"
    }

    fn execute<'a>(&'a mut self, input: &'a StageExecutionInput) -> BoxFuture<'a, StageResult> {
        Box::pin(async move {
            let declared_classes = self.get_declared_classes(input.from, input.to)?;

            if !declared_classes.is_empty() {
                // fetch the classes artifacts
                let class_artifacts = self
                    .downloader
                    .download(declared_classes.clone())
                    .await
                    .map_err(Error::Gateway)?;

                debug!(target: "stage", id = self.id(), total = %class_artifacts.len(), "Storing class artifacts.");
                for (key, rpc_class) in declared_classes.iter().zip(class_artifacts) {
                    let class = rpc_class.try_into().map_err(Error::Conversion)?;
                    self.provider.set_class(key.class_hash, class)?;
                }
            }

            Ok(())
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("missing declared classes for block {block}")]
    MissingBlockDeclaredClasses {
        /// The block number whose declared classes are missing.
        block: BlockNumber,
    },

    /// Error returnd by the client used to download the classes from.
    #[error(transparent)]
    Gateway(#[from] katana_gateway::client::Error),

    /// Error that can occur when converting the classes types to the internal types.
    #[error(transparent)]
    Conversion(#[from] ConversionError),

    #[error(transparent)]
    Provider(#[from] ProviderError),
}

#[derive(Debug)]
struct ClassDownloader {
    gateway: SequencerGateway,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ClassDownloadKey {
    /// The hash of the class artifact to download
    class_hash: ClassHash,
    block: BlockNumber,
}

impl Downloader for ClassDownloader {
    type Key = ClassDownloadKey;
    type Value = ContractClass;
    type Error = katana_gateway::client::Error;

    #[allow(clippy::manual_async_fn)]
    fn download(
        &self,
        key: &Self::Key,
    ) -> impl Future<Output = DownloaderResult<Self::Value, Self::Error>> {
        async {
            match self.gateway.get_class(key.class_hash, key.block.into()).await {
                Ok(data) => DownloaderResult::Ok(data),
                Err(err) if err.is_rate_limited() => DownloaderResult::Retry(err),
                Err(err) => DownloaderResult::Err(err),
            }
        }
    }
}
