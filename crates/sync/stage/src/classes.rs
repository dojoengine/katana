use anyhow::Result;
use katana_feeder_gateway::client::{self, SequencerGateway};
use katana_feeder_gateway::types::BlockId;
use katana_primitives::block::BlockNumber;
use katana_primitives::class::{ClassHash, ContractClass};
use katana_provider::api::contract::ContractClassWriter;
use katana_provider::api::state_update::StateUpdateProvider;
use katana_provider::api::ProviderError;
use katana_rpc_types::class::ConversionError;
use tracing::{debug, error};

use super::downloader::{Downloader, Fetchable, RetryConfig};
use super::{Stage, StageExecutionInput, StageResult};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("missing declared classes for block {block}")]
    MissingBlockDeclaredClasses {
        /// The block number whose declared classes are missing.
        block: BlockNumber,
    },

    /// Error returnd by the client used to download the classes from.
    #[error(transparent)]
    Gateway(#[from] client::Error),

    /// Error that can occur when converting the classes types to the internal types.
    #[error(transparent)]
    Conversion(#[from] ConversionError),

    #[error(transparent)]
    Provider(#[from] ProviderError),
}

/// Trait for downloading contract classes.
///
/// This abstraction allows the Classes stage to work with different download implementations,
/// making it easy to test with mock downloaders or swap implementations.
#[async_trait::async_trait]
pub trait ClassDownloader: Send + Sync {
    /// Download classes for the given class hashes at a specific block.
    ///
    /// Returns a vector of tuples containing the class hash and the downloaded class.
    async fn download(
        &self,
        classes: &[(ClassHash, BlockNumber)],
    ) -> Result<Vec<(ClassHash, ContractClass)>, Error>;
}

#[derive(Debug)]
pub struct Classes<P, D> {
    provider: P,
    downloader: D,
}

impl<P> Classes<P, FeederGatewayClassDownloader> {
    pub fn new(provider: P, feeder_gateway: SequencerGateway, download_batch_size: usize) -> Self {
        let downloader = FeederGatewayClassDownloader::new(feeder_gateway, download_batch_size);
        Self { provider, downloader }
    }
}

impl<P, D> Classes<P, D> {
    /// Create a new Classes stage with a custom downloader.
    ///
    /// This is useful for testing with mock downloaders.
    pub fn with_downloader(provider: P, downloader: D) -> Self {
        Self { provider, downloader }
    }
}

#[async_trait::async_trait]
impl<P, D> Stage for Classes<P, D>
where
    P: StateUpdateProvider + ContractClassWriter,
    D: ClassDownloader,
{
    fn id(&self) -> &'static str {
        "Classes"
    }

    async fn execute(&mut self, input: &StageExecutionInput) -> StageResult {
        let mut classes_to_fetch: Vec<(ClassHash, BlockNumber)> = Vec::new();

        for block in input.from..=input.to {
            // get the classes declared at block `i`
            let class_hashes = self
                .provider
                .declared_classes(block.into())?
                .ok_or(Error::MissingBlockDeclaredClasses { block })?;

            // collect classes to fetch with their block numbers
            for hash in class_hashes.keys().copied() {
                classes_to_fetch.push((hash, block));
            }
        }

        if !classes_to_fetch.is_empty() {
            // fetch the classes artifacts
            let class_artifacts = self.downloader.download(&classes_to_fetch).await?;

            debug!(target: "stage", id = self.id(), total = %class_artifacts.len(), "Storing class artifacts.");
            for (hash, class) in class_artifacts {
                self.provider.set_class(hash, class)?;
            }
        }

        Ok(())
    }
}

/// Implementation of ClassDownloader using the feeder gateway.
pub struct FeederGatewayClassDownloader {
    downloader: Downloader<ClassWithContext>,
}

impl FeederGatewayClassDownloader {
    pub fn new(feeder_gateway: SequencerGateway, download_batch_size: usize) -> Self {
        let downloader = Downloader::new(feeder_gateway, download_batch_size);
        Self { downloader }
    }

    pub fn with_retry_config(
        feeder_gateway: SequencerGateway,
        download_batch_size: usize,
        retry_config: RetryConfig,
    ) -> Self {
        let downloader =
            Downloader::with_retry_config(feeder_gateway, download_batch_size, retry_config);
        Self { downloader }
    }
}

#[async_trait::async_trait]
impl ClassDownloader for FeederGatewayClassDownloader {
    async fn download(
        &self,
        classes: &[(ClassHash, BlockNumber)],
    ) -> Result<Vec<(ClassHash, ContractClass)>, Error> {
        let keys: Vec<ClassKey> =
            classes.iter().map(|(hash, block)| ClassKey { hash: *hash, block: *block }).collect();

        let class_artifacts = self.downloader.download(&keys).await?;

        let result = class_artifacts
            .into_iter()
            .map(|class_with_ctx| (class_with_ctx.key.hash, class_with_ctx.class))
            .collect();

        Ok(result)
    }
}

/// A key that identifies a class to fetch, including the block context.
#[derive(Debug, Clone, Copy)]
struct ClassKey {
    hash: ClassHash,
    block: BlockNumber,
}

/// Wrapper type for a fetched class with its associated key.
#[derive(Debug)]
struct ClassWithContext {
    key: ClassKey,
    class: ContractClass,
}

// Implement Fetchable trait for ClassWithContext
#[async_trait::async_trait]
impl Fetchable for ClassWithContext {
    type Key = ClassKey;
    type Error = Error;

    async fn fetch(client: &SequencerGateway, key: Self::Key) -> Result<Self, Self::Error> {
        let class = client.get_class(key.hash, BlockId::Number(key.block)).await.inspect_err(
            |error| {
                if !error.is_rate_limited() {
                    error!(target: "pipeline", %error, block = %key.block, class = %format!("{:#x}", key.hash), "Fetching class.")
                }
            },
        )?;
        Ok(ClassWithContext { key, class: class.try_into()? })
    }
}

#[cfg(test)]
mod tests {
    use katana_primitives::block::BlockNumber;
    use katana_primitives::class::{ClassHash, ContractClass};

    use super::{ClassDownloader, Classes, Error};

    // Example of how to create a mock downloader for testing
    struct MockClassDownloader {
        classes: Vec<(ClassHash, ContractClass)>,
    }

    #[async_trait::async_trait]
    impl ClassDownloader for MockClassDownloader {
        async fn download(
            &self,
            _classes: &[(ClassHash, BlockNumber)],
        ) -> Result<Vec<(ClassHash, ContractClass)>, Error> {
            Ok(self.classes.clone())
        }
    }

    #[test]
    fn test_mock_downloader_can_be_created() {
        // This test demonstrates that you can easily create mock downloaders
        // for testing the Classes stage in isolation
        let _mock = MockClassDownloader { classes: vec![] };
        // In a full test, you would use:
        // let stage = Classes::with_downloader(provider, mock);
    }
}
