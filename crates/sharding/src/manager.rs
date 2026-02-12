use std::collections::HashMap;
use std::fmt::Debug;
use std::sync::Arc;

use anyhow::Result;
use katana_chain_spec::ChainSpec;
use katana_executor::ExecutorFactory;
use katana_gas_price_oracle::GasPriceOracle;
use katana_rpc_server::starknet::StarknetApiConfig;
use katana_tasks::TaskSpawner;
use parking_lot::RwLock;

use crate::types::{Shard, ShardId};

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
}

impl LazyShardManager {
    pub fn new(
        chain_spec: Arc<ChainSpec>,
        executor_factory: Arc<dyn ExecutorFactory>,
        gas_oracle: GasPriceOracle,
        starknet_api_config: StarknetApiConfig,
        task_spawner: TaskSpawner,
    ) -> Self {
        Self {
            inner: Arc::new(LazyShardManagerInner {
                shards: RwLock::new(HashMap::new()),
                chain_spec,
                executor_factory,
                gas_oracle,
                starknet_api_config,
                task_spawner,
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
