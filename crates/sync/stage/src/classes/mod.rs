use std::future::Future;

use anyhow::Result;
use futures::channel::oneshot;
use futures::future::BoxFuture;
use katana_primitives::block::BlockNumber;
use katana_primitives::class::{ClassHash, ContractClass};
use katana_provider::api::contract::ContractClassWriter;
use katana_provider::api::state_update::StateUpdateProvider;
use katana_provider::api::ProviderError;
use katana_provider::{DbProviderFactory, MutableProvider, ProviderFactory};
use katana_rpc_types::class::ConversionError;
use katana_rpc_types::Class;
use rayon::prelude::*;
use tracing::{debug, error, info_span, Instrument};

use super::{
    PruneInput, PruneOutput, PruneResult, Stage, StageExecutionInput, StageExecutionOutput,
    StageResult,
};
mod downloader;

pub use downloader::gateway::GatewayClassDownloader;
pub use downloader::json_rpc::JsonRpcClassDownloader;

/// Trait for downloading contract class artifacts.
///
/// This provides a stage-specific abstraction for downloading classes, allowing different
/// implementations (e.g., gateway-based, JSON-RPC-based) to be used with the [`Classes`] stage.
pub trait ClassDownloader: Send + Sync {
    type Error: std::error::Error + Send + Sync + 'static;

    /// Downloads the class artifact for the given class hash at the given block.
    fn download_classes(
        &self,
        keys: Vec<ClassDownloadKey>,
    ) -> impl Future<Output = Result<Vec<Class>, Self::Error>> + Send;
}

/// A stage for downloading and storing contract classes.
pub struct Classes<D> {
    provider: DbProviderFactory,
    downloader: D,
    /// Thread pool for parallel class hash verification
    verification_pool: rayon::ThreadPool,
}

impl<D> std::fmt::Debug for Classes<D> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Classes")
            .field("provider", &self.provider)
            .field("verification_pool", &"<ThreadPool>")
            .finish()
    }
}

impl<D> Classes<D> {
    /// Create a new [`Classes`] stage with the given downloader.
    pub fn new(provider: DbProviderFactory, downloader: D) -> Self {
        // A dedicated thread pool for class hash verification
        let verification_pool = rayon::ThreadPoolBuilder::new()
            .build()
            .expect("Failed to create verification thread pool");

        Self { provider, downloader, verification_pool }
    }

    /// Returns the hashes of the classes declared in the given range of blocks.
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
                .provider()
                .declared_classes(block.into())?
                .ok_or(Error::MissingBlockDeclaredClasses { block })?;

            // collect classes to fetch with their block numbers
            for class_hash in class_hashes.keys().copied() {
                classes_keys.push(ClassDownloadKey { class_hash, block });
            }
        }

        Ok(classes_keys)
    }

    async fn verify_class_hashes(
        &self,
        class_hashes: &[ClassDownloadKey],
        class_artifacts: Vec<Class>,
    ) -> Result<Vec<ContractClass>, Error> {
        let (tx, rx) = oneshot::channel();
        let class_hashes = class_hashes.to_vec();

        self.verification_pool.spawn(move || {
            let result = class_hashes
                .par_iter()
                .zip(class_artifacts.into_par_iter())
                .map(|(key, class)| {
                    let block = key.block;

                    let expected_hash = key.class_hash;
                    let computed_hash = class.hash();

                    if computed_hash != expected_hash {
                        return Err(Error::ClassHashMismatch {
                            expected: expected_hash,
                            actual: computed_hash,
                            block,
                        });
                    }

                    ContractClass::try_from(class).map_err(Error::Conversion)
                })
                .collect::<Result<Vec<_>, Error>>();

            let _ = tx.send(result);
        });

        rx.await.unwrap()
    }
}

impl<D> Stage for Classes<D>
where
    D: ClassDownloader,
{
    fn id(&self) -> &'static str {
        "Classes"
    }

    fn execute<'a>(&'a mut self, input: &'a StageExecutionInput) -> BoxFuture<'a, StageResult> {
        Box::pin(async move {
            let declared_class_hashes = self.get_declared_classes(input.from(), input.to())?;

            if declared_class_hashes.is_empty() {
                debug!(from = %input.from(), to = %input.to(), "No classes declared within the block range");
            } else {
                let total_classes = declared_class_hashes.len();

                // fetch the classes artifacts
                let class_artifacts = self
                    .downloader
                    .download_classes(declared_class_hashes.clone())
                    .instrument(info_span!(target: "stage", "classes.download", %total_classes))
                    .await
                    .map_err(|e| Error::Download(Box::new(e)))?;

                let verified_classes =
                    self.verify_class_hashes(&declared_class_hashes, class_artifacts).await?;

                debug!(target: "stage", id = self.id(), total = %verified_classes.len(), "Storing class artifacts.");

                let provider_mut = self.provider.provider_mut();
                // Second pass: insert the verified classes into storage
                // This must be done sequentially as database only supports single write transaction
                for (key, class) in declared_class_hashes.iter().zip(verified_classes.into_iter()) {
                    provider_mut.set_class(key.class_hash, class)?;
                }

                provider_mut.commit()?;
            }

            Ok(StageExecutionOutput { last_block_processed: input.to() })
        })
    }

    fn prune<'a>(&'a mut self, _: &'a PruneInput) -> BoxFuture<'a, PruneResult> {
        Box::pin(async move {
            // Classes are immutable once declared and don't need pruning.
            // A class declared at block N can still be used to deploy contracts at block N+1000.
            // Therefore, we cannot safely prune classes based on block age alone.
            Ok(PruneOutput::default())
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

    /// Error returned by the downloader used to download the classes.
    #[error(transparent)]
    Download(Box<dyn std::error::Error + Send + Sync>),

    /// Error that can occur when converting the classes types to the internal types.
    #[error(transparent)]
    Conversion(ConversionError),

    #[error(transparent)]
    Provider(#[from] ProviderError),

    /// Error when a downloaded class produces a different hash than expected
    #[error(
        "class hash mismatch for class at block {block}: expected {expected:#x}, got {actual:#x}"
    )]
    ClassHashMismatch {
        /// The block number where the class was declared
        block: BlockNumber,
        /// The expected class hash
        expected: ClassHash,
        /// The actual computed class hash
        actual: ClassHash,
    },

    /// Error when computing the class hash
    #[error("failed to recompute class hash for class {class_hash} at block {block}: {source}")]
    ClassHashComputation {
        /// The block number where the class was declared
        block: BlockNumber,
        /// The hash of the class
        class_hash: ClassHash,
        /// The underlying error
        #[source]
        source: katana_primitives::class::ComputeClassHashError,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassDownloadKey {
    /// The hash of the class artifact to download
    pub class_hash: ClassHash,
    pub block: BlockNumber,
}
