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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use katana_chain_spec::ChainSpec;
    use katana_executor::blockifier::cache::ClassCache;
    use katana_executor::blockifier::BlockifierFactory;
    use katana_executor::{BlockLimits, ExecutionFlags, ExecutorFactory};
    use katana_gas_price_oracle::GasPriceOracle;
    use katana_primitives::address;
    use katana_primitives::block::FinalityStatus;
    use katana_primitives::da::L1DataAvailabilityMode;
    use katana_primitives::env::BlockEnv;
    use katana_primitives::Felt;
    use katana_rpc_server::starknet::StarknetApiConfig;
    use katana_rpc_types::block::{
        BlockWithTxHashes, GetBlockWithTxHashesResponse, PreConfirmedBlockWithTxHashes,
    };
    use katana_tasks::TaskManager;
    use starknet::core::types::ResourcePrice;

    use super::{block_env_from_latest_block_response, LazyShardManager, ShardManager};

    #[test]
    fn converts_confirmed_latest_block_to_block_env() {
        let response = GetBlockWithTxHashesResponse::Block(BlockWithTxHashes {
            status: FinalityStatus::AcceptedOnL2,
            block_hash: Felt::from(1u64),
            parent_hash: Felt::from(2u64),
            block_number: 42,
            new_root: Felt::from(3u64),
            timestamp: 1_234_567_890,
            sequencer_address: address!("0x1234"),
            l1_gas_price: resource_price(101, 102),
            l2_gas_price: resource_price(201, 202),
            l1_data_gas_price: resource_price(301, 302),
            l1_da_mode: L1DataAvailabilityMode::Blob,
            starknet_version: "0.14.0".to_owned(),
            transactions: vec![],
        });

        let env = block_env_from_latest_block_response(response).expect("should convert");
        assert_eq!(env.number, 42);
        assert_eq!(env.timestamp, 1_234_567_890);
        assert_eq!(env.sequencer_address, address!("0x1234"));
        assert_eq!(env.l1_gas_prices.eth.get(), 101);
        assert_eq!(env.l1_gas_prices.strk.get(), 102);
        assert_eq!(env.l2_gas_prices.eth.get(), 201);
        assert_eq!(env.l2_gas_prices.strk.get(), 202);
        assert_eq!(env.l1_data_gas_prices.eth.get(), 301);
        assert_eq!(env.l1_data_gas_prices.strk.get(), 302);
        assert_eq!(env.starknet_version.to_string(), "0.14.0");
    }

    #[test]
    fn rejects_invalid_starknet_version_from_latest_block() {
        let response = GetBlockWithTxHashesResponse::PreConfirmed(PreConfirmedBlockWithTxHashes {
            block_number: 7,
            timestamp: 1_234_567_890,
            sequencer_address: address!("0x1234"),
            l1_gas_price: resource_price(101, 102),
            l2_gas_price: resource_price(201, 202),
            l1_data_gas_price: resource_price(301, 302),
            l1_da_mode: L1DataAvailabilityMode::Blob,
            starknet_version: "not.a.version".to_owned(),
            transactions: vec![],
        });

        let err = block_env_from_latest_block_response(response).expect_err("must fail");
        assert!(err.to_string().contains("invalid Starknet version"));
    }

    #[tokio::test]
    async fn get_fails_and_does_not_register_shard_when_initial_env_fetch_fails() {
        let chain_spec = Arc::new(ChainSpec::dev());
        let manager = LazyShardManager::new_with_block_env_fetcher(
            chain_spec.clone(),
            test_executor_factory(chain_spec),
            GasPriceOracle::create_for_testing(),
            test_starknet_api_config(),
            TaskManager::current().task_spawner(),
            Arc::new(|| -> anyhow::Result<BlockEnv> { anyhow::bail!("base chain unavailable") }),
        );

        let shard_id = address!("0x42");
        let err = manager.get(shard_id).expect_err("must fail");

        assert!(err.to_string().contains("failed to generate initial block context"));
        assert!(manager.is_empty());
        assert!(manager.shard_ids().is_empty());
    }

    fn resource_price(price_in_wei: u128, price_in_fri: u128) -> ResourcePrice {
        ResourcePrice { price_in_wei: price_in_wei.into(), price_in_fri: price_in_fri.into() }
    }

    fn test_starknet_api_config() -> StarknetApiConfig {
        StarknetApiConfig {
            max_event_page_size: None,
            max_proof_keys: None,
            max_call_gas: None,
            max_concurrent_estimate_fee_requests: None,
            simulation_flags: ExecutionFlags::new(),
            versioned_constant_overrides: None,
            #[cfg(feature = "cartridge")]
            paymaster: None,
        }
    }

    fn test_executor_factory(chain_spec: Arc<ChainSpec>) -> Arc<dyn ExecutorFactory> {
        let class_cache = ClassCache::new().expect("class cache should initialize");
        Arc::new(BlockifierFactory::new(
            None,
            ExecutionFlags::new(),
            BlockLimits::default(),
            class_cache,
            chain_spec,
        ))
    }
}
