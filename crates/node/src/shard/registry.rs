use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use katana_chain_spec::ChainSpec;
use katana_executor::ExecutorFactory;
use katana_gas_price_oracle::GasPriceOracle;
use katana_rpc_server::starknet::StarknetApiConfig;
use katana_tasks::TaskSpawner;
use parking_lot::RwLock;

use super::types::{Shard, ShardId};

/// Registry of all active shards, with lazy creation support.
#[derive(Clone)]
pub struct ShardRegistry {
    inner: Arc<ShardRegistryInner>,
}

struct ShardRegistryInner {
    shards: RwLock<HashMap<ShardId, Arc<Shard>>>,
    // Shared resources for lazy shard creation
    chain_spec: Arc<ChainSpec>,
    executor_factory: Arc<dyn ExecutorFactory>,
    gas_oracle: GasPriceOracle,
    starknet_api_config: StarknetApiConfig,
    task_spawner: TaskSpawner,
}

impl ShardRegistry {
    pub fn new(
        chain_spec: Arc<ChainSpec>,
        executor_factory: Arc<dyn ExecutorFactory>,
        gas_oracle: GasPriceOracle,
        starknet_api_config: StarknetApiConfig,
        task_spawner: TaskSpawner,
    ) -> Self {
        Self {
            inner: Arc::new(ShardRegistryInner {
                shards: RwLock::new(HashMap::new()),
                chain_spec,
                executor_factory,
                gas_oracle,
                starknet_api_config,
                task_spawner,
            }),
        }
    }

    /// Look up a shard by id. Returns `None` if the shard doesn't exist.
    pub fn get(&self, id: &ShardId) -> Option<Arc<Shard>> {
        self.inner.shards.read().get(id).cloned()
    }

    /// Get an existing shard or create a new one lazily.
    pub fn get_or_create(&self, id: ShardId) -> Result<Arc<Shard>> {
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

    /// List all registered shard ids.
    pub fn shard_ids(&self) -> Vec<ShardId> {
        self.inner.shards.read().keys().copied().collect()
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

impl std::fmt::Debug for ShardRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShardRegistry").field("shard_count", &self.len()).finish_non_exhaustive()
    }
}
