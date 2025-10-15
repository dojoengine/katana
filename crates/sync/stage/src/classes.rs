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
use rayon::prelude::*;
use tracing::{debug, error};

use super::{Stage, StageExecutionInput, StageExecutionOutput, StageResult};
use crate::downloader::{BatchDownloader, Downloader, DownloaderResult};

/// A stage for downloading and storing contract classes.
pub struct Classes<P> {
    provider: P,
    downloader: BatchDownloader<ClassDownloader>,
    /// Thread pool for parallel class hash verification
    verification_pool: rayon::ThreadPool,
}

impl<P> std::fmt::Debug for Classes<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Classes")
            .field("provider", &std::any::type_name::<P>())
            .field("downloader", &self.downloader)
            .field("verification_pool", &"<ThreadPool>")
            .finish()
    }
}

impl<P> Classes<P> {
    /// Create a new Classes stage using the Feeder Gateway downloader.
    pub fn new(provider: P, gateway: SequencerGateway, batch_size: usize) -> Self {
        let downloader = ClassDownloader { gateway };
        let downloader = BatchDownloader::new(downloader, batch_size);

        // Create a dedicated thread pool for class hash verification
        // Use the number of available CPUs for optimal parallelization
        let verification_pool = rayon::ThreadPoolBuilder::new()
            .build()
            .expect("Failed to create verification thread pool");

        Self { provider, downloader, verification_pool }
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
            let declared_classes = self.get_declared_classes(input.from(), input.to())?;

            if !declared_classes.is_empty() {
                // fetch the classes artifacts
                let class_artifacts = self
                    .downloader
                    .download(declared_classes.clone())
                    .await
                    .map_err(Error::Gateway)?;

                debug!(target: "stage", id = self.id(), total = %class_artifacts.len(), "Verifying class artifacts.");

                // First pass: verify class hashes in parallel using the dedicated thread pool
                // We need to convert to primitives types for verification
                let verification_results: Vec<
                    Result<katana_primitives::class::ContractClass, Error>,
                > = {
                    let pool = &self.verification_pool;
                    pool.install(|| {
                        declared_classes
                            .par_iter()
                            .zip(class_artifacts.par_iter())
                            .map(|(key, rpc_class)| {
                                // Convert to primitives type
                                let class: katana_primitives::class::ContractClass =
                                    rpc_class.clone().try_into().map_err(Error::Conversion)?;

                                // Compute the class hash
                                let computed_hash = class.class_hash().map_err(|source| {
                                    Error::ClassHashComputation { block: key.block, source }
                                })?;

                                // Verify it matches the expected hash
                                if computed_hash != key.class_hash {
                                    return Err(Error::ClassHashMismatch {
                                        expected: key.class_hash,
                                        actual: computed_hash,
                                        block: key.block,
                                    });
                                }

                                Ok(class)
                            })
                            .collect()
                    })
                };

                // Check if any verification failed
                let verified_classes: Vec<_> =
                    verification_results.into_iter().collect::<Result<Vec<_>, _>>()?;

                debug!(target: "stage", id = self.id(), total = %verified_classes.len(), "Storing class artifacts.");

                // Second pass: insert the verified classes into storage
                // This must be done sequentially as database only supports single write transaction
                for (key, class) in declared_classes.iter().zip(verified_classes) {
                    self.provider.set_class(key.class_hash, class)?;
                }
            }

            Ok(StageExecutionOutput { last_block_processed: input.to() })
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

    /// Error when a downloaded class produces a different hash than expected
    #[error(
        "class hash mismatch for class at block {block}: expected {expected:#x}, got {actual:#x}"
    )]
    ClassHashMismatch {
        /// The expected class hash
        expected: ClassHash,
        /// The actual computed class hash
        actual: ClassHash,
        /// The block number where the class was declared
        block: BlockNumber,
    },

    /// Error when computing the class hash
    #[error("failed to compute class hash for class at block {block}: {source}")]
    ClassHashComputation {
        /// The block number where the class was declared
        block: BlockNumber,
        /// The underlying error
        #[source]
        source: katana_primitives::class::ComputeClassHashError,
    },
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
