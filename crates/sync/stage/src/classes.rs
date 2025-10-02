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

use super::downloader::{Downloader, Fetchable};
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

#[derive(Debug)]
pub struct Classes<P> {
    provider: P,
    downloader: Downloader<ClassWithContext>,
}

impl<P> Classes<P> {
    pub fn new(provider: P, feeder_gateway: SequencerGateway, download_batch_size: usize) -> Self {
        let downloader = Downloader::new(feeder_gateway, download_batch_size);
        Self { provider, downloader }
    }
}

#[async_trait::async_trait]
impl<P> Stage for Classes<P>
where
    P: StateUpdateProvider + ContractClassWriter,
{
    fn id(&self) -> &'static str {
        "Classes"
    }

    async fn execute(&mut self, input: &StageExecutionInput) -> StageResult {
        let mut keys: Vec<ClassKey> = Vec::new();

        for block in input.from..=input.to {
            // get the classes declared at block `i`
            let class_hashes = self
                .provider
                .declared_classes(block.into())?
                .ok_or(Error::MissingBlockDeclaredClasses { block })?;

            // create keys for each class hash with its block number
            for hash in class_hashes.keys().copied() {
                keys.push(ClassKey { hash, block });
            }
        }

        if !keys.is_empty() {
            // fetch the classes artifacts
            let class_artifacts = self.downloader.download(&keys).await?;

            debug!(target: "stage", id = self.id(), total = %class_artifacts.len(), "Storing class artifacts.");
            for class_with_ctx in class_artifacts {
                self.provider.set_class(class_with_ctx.key.hash, class_with_ctx.class)?;
            }
        }

        Ok(())
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

    fn retry_min_delay_secs() -> u64 {
        3
    }
}
