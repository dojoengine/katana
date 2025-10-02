use katana_feeder_gateway::client::SequencerGateway;
use katana_primitives::block::BlockNumber;
use katana_primitives::class::ClassHash;
use katana_provider::api::contract::ContractClassWriter;
use katana_provider::api::state_update::StateUpdateProvider;
use tracing::debug;

// Re-export Error and other public types from downloader::classes
pub use super::downloader::classes::Error;
use super::downloader::classes::{ClassDownloader, FeederGatewayClassDownloader};
use super::{Stage, StageExecutionInput, StageResult};

/// A stage for downloading and storing contract classes.
///
/// This stage is generic over the downloader implementation, allowing for different
/// download strategies (e.g., Feeder Gateway, P2P, L1).
#[derive(Debug)]
pub struct Classes<P, D> {
    provider: P,
    downloader: D,
}

impl<P> Classes<P, FeederGatewayClassDownloader> {
    /// Create a new Classes stage using the Feeder Gateway downloader.
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
