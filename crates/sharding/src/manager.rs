use std::collections::HashMap;
use std::fmt::Debug;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use katana_chain_spec::ChainSpec;
use katana_executor::ExecutorFactory;
use katana_gas_price_oracle::GasPriceOracle;
use katana_primitives::block::BlockIdOrTag;
use katana_primitives::env::BlockEnv;
use katana_rpc_client::starknet::Client as StarknetClient;
use katana_rpc_server::starknet::StarknetApiConfig;
use katana_rpc_types::block::GetBlockWithTxHashesResponse;
use katana_tasks::TaskSpawner;
use parking_lot::RwLock;
use url::Url;

use crate::types::{Shard, ShardId};

type InitialBlockEnvFetcher = dyn Fn() -> Result<BlockEnv> + Send + Sync + 'static;

fn new_base_chain_block_env_fetcher(base_chain_url: Url) -> Arc<InitialBlockEnvFetcher> {
    let client = StarknetClient::new(base_chain_url);
    Arc::new(move || {
        // Run on a dedicated OS thread to avoid blocking/panicking inside an existing Tokio
        // runtime thread while still allowing this synchronous API to fetch async RPC data.
        let client = client.clone();
        let handle = std::thread::Builder::new()
            .name("shard-base-chain-block-env".to_owned())
            .spawn(move || -> Result<BlockEnv> {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .context("failed to build runtime for latest base-chain block fetch")?;

                runtime.block_on(async move {
                    let response = client
                        .get_block_with_tx_hashes(BlockIdOrTag::Latest)
                        .await
                        .context("failed to fetch latest block from base chain")?;
                    block_env_from_latest_block_response(response)
                })
            })
            .context("failed to spawn latest base-chain block fetch thread")?;

        handle.join().map_err(|_| anyhow!("latest base-chain block fetch thread panicked"))?
    })
}

fn block_env_from_latest_block_response(
    response: GetBlockWithTxHashesResponse,
) -> Result<BlockEnv> {
    let (
        number,
        timestamp,
        sequencer_address,
        l1_gas_price,
        l2_gas_price,
        l1_data_gas_price,
        starknet_version,
    ) = match response {
        GetBlockWithTxHashesResponse::Block(block) => (
            block.block_number,
            block.timestamp,
            block.sequencer_address,
            block.l1_gas_price,
            block.l2_gas_price,
            block.l1_data_gas_price,
            block.starknet_version,
        ),
        GetBlockWithTxHashesResponse::PreConfirmed(block) => (
            block.block_number,
            block.timestamp,
            block.sequencer_address,
            block.l1_gas_price,
            block.l2_gas_price,
            block.l1_data_gas_price,
            block.starknet_version,
        ),
    };

    let starknet_version = starknet_version
        .try_into()
        .context("invalid Starknet version in latest base-chain block")?;

    Ok(BlockEnv {
        number,
        timestamp,
        sequencer_address,
        l1_gas_prices: l1_gas_price.into(),
        l2_gas_prices: l2_gas_price.into(),
        l1_data_gas_prices: l1_data_gas_price.into(),
        starknet_version,
    })
}

/// Pluggable abstraction for shard storage and creation policy.
///
/// Implementations decide what happens when a shard is requested:
/// - [`LazyShardManager`] creates shards on first access (dev mode).
/// - A future `OnchainShardManager` would only look up pre-registered shards (production mode,
///   where shards are created from onchain events).
pub trait ShardManager: Send + Sync + Debug + 'static {
    /// Resolve a shard by ID. The creation policy is implementation-defined:
    /// lazy managers create on miss, strict managers return an error.
    fn get(&self, id: ShardId) -> Result<Arc<Shard>>;

    /// List all registered shard IDs.
    fn shard_ids(&self) -> Vec<ShardId>;
}

/// Dev-mode shard manager that lazily creates shards on first access.
///
/// Holds shared resources (chain spec, executor factory, gas oracle, etc.)
/// needed to construct new shards. Uses double-checked locking to ensure
/// at most one shard instance per ID.
pub struct LazyShardManager {
    inner: Arc<LazyShardManagerInner>,
}

struct LazyShardManagerInner {
    shards: RwLock<HashMap<ShardId, Arc<Shard>>>,
    // Shared resources for lazy shard creation
    chain_spec: Arc<ChainSpec>,
    executor_factory: Arc<dyn ExecutorFactory>,
    gas_oracle: GasPriceOracle,
    starknet_api_config: StarknetApiConfig,
    task_spawner: TaskSpawner,
    initial_block_env_fetcher: Arc<InitialBlockEnvFetcher>,
}

impl LazyShardManager {
    pub fn new(
        chain_spec: Arc<ChainSpec>,
        executor_factory: Arc<dyn ExecutorFactory>,
        gas_oracle: GasPriceOracle,
        starknet_api_config: StarknetApiConfig,
        task_spawner: TaskSpawner,
        base_chain_url: Url,
    ) -> Self {
        let initial_block_env_fetcher = new_base_chain_block_env_fetcher(base_chain_url);

        Self::new_with_block_env_fetcher(
            chain_spec,
            executor_factory,
            gas_oracle,
            starknet_api_config,
            task_spawner,
            initial_block_env_fetcher,
        )
    }

    fn new_with_block_env_fetcher(
        chain_spec: Arc<ChainSpec>,
        executor_factory: Arc<dyn ExecutorFactory>,
        gas_oracle: GasPriceOracle,
        starknet_api_config: StarknetApiConfig,
        task_spawner: TaskSpawner,
        initial_block_env_fetcher: Arc<InitialBlockEnvFetcher>,
    ) -> Self {
        Self {
            inner: Arc::new(LazyShardManagerInner {
                shards: RwLock::new(HashMap::new()),
                chain_spec,
                executor_factory,
                gas_oracle,
                starknet_api_config,
                task_spawner,
                initial_block_env_fetcher,
            }),
        }
    }

    /// Returns the number of registered shards.
    pub fn len(&self) -> usize {
        self.inner.shards.read().len()
    }

    /// Returns `true` if no shards are registered.
    pub fn is_empty(&self) -> bool {
        self.inner.shards.read().is_empty()
    }
}

impl ShardManager for LazyShardManager {
    fn get(&self, id: ShardId) -> Result<Arc<Shard>> {
        // Fast path: read lock
        {
            let shards = self.inner.shards.read();
            if let Some(shard) = shards.get(&id) {
                return Ok(Arc::clone(shard));
            }
        }

        // Fetch initial block context from the latest base-chain block.
        let initial_block_env = (self.inner.initial_block_env_fetcher.as_ref())()
            .context("failed to generate initial block context for shard")?;

        // Slow path: write lock to create
        let mut shards = self.inner.shards.write();

        // Double-check after acquiring write lock
        if let Some(shard) = shards.get(&id) {
            return Ok(Arc::clone(shard));
        }

        let shard = Arc::new(Shard::new(
            id,
            self.inner.chain_spec.clone(),
            self.inner.executor_factory.clone(),
            self.inner.gas_oracle.clone(),
            self.inner.starknet_api_config.clone(),
            self.inner.task_spawner.clone(),
            initial_block_env,
        )?);

        shards.insert(id, Arc::clone(&shard));
        Ok(shard)
    }

    fn shard_ids(&self) -> Vec<ShardId> {
        self.inner.shards.read().keys().copied().collect()
    }
}

impl Debug for LazyShardManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LazyShardManager").field("shard_count", &self.len()).finish_non_exhaustive()
    }
}
