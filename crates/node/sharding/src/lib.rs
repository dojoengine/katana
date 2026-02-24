pub mod config;
pub mod exit;

use std::sync::Arc;

use anyhow::Result;
use jsonrpsee::types::ErrorObjectOwned;
use jsonrpsee::RpcModule;
use katana_chain_spec::ChainSpec;
use katana_executor::blockifier::cache::ClassCache;
use katana_executor::blockifier::BlockifierFactory;
use katana_executor::{ExecutionFlags, ExecutorFactory};
use katana_gas_price_oracle::GasPriceOracle;
use katana_pool::TxPool;
use katana_primitives::env::VersionedConstantsOverrides;
use katana_primitives::{ContractAddress, Felt};
use katana_provider::DbProviderFactory;
use katana_rpc_api::shard::ShardApiServer;
use katana_rpc_server::shard::{ShardProvider, ShardRpc};
use katana_rpc_server::starknet::{StarknetApi, StarknetApiConfig};
use katana_rpc_server::{RpcServer, RpcServerHandle};
use katana_sharding::manager::{LazyShardManager, ShardManager};
use katana_sharding::runtime::{Runtime, RuntimeHandle};
use katana_sharding::types::NoPendingBlockProvider;
use katana_tasks::TaskManager;
use parking_lot::Mutex;
use tracing::info;

use self::config::ShardNodeConfig;
use self::exit::ShardNodeStoppedFuture;

/// A shard node instance.
#[must_use = "ShardNode does nothing unless launched."]
#[derive(Debug)]
pub struct Node {
    pub config: Arc<ShardNodeConfig>,
    pub manager: Arc<dyn ShardManager>,
    pub handle: RuntimeHandle,
    pub task_manager: TaskManager,
    pub rpc_server: RpcServer,
    pub runtime: Mutex<Option<Runtime>>,
}

impl Node {
    pub fn build(config: ShardNodeConfig) -> Result<Self> {
        let task_manager = TaskManager::current();
        let task_spawner = task_manager.task_spawner();

        // --- Build executor factory (same pattern as sequencer node)
        let overrides = Some(VersionedConstantsOverrides {
            invoke_tx_max_n_steps: Some(config.execution.invocation_max_steps),
            validate_max_n_steps: Some(config.execution.validation_max_steps),
            max_recursion_depth: Some(config.execution.max_recursion_depth),
        });

        let execution_flags = ExecutionFlags::new()
            .with_account_validation(config.dev.account_validation)
            .with_fee(config.dev.fee);

        let executor_factory = {
            #[allow(unused_mut)]
            let mut class_cache = ClassCache::builder();

            #[cfg(feature = "native")]
            {
                class_cache = class_cache.compile_native(config.execution.compile_native);
            }

            let global_class_cache = class_cache.build_global()?;

            Arc::new(BlockifierFactory::new(
                overrides,
                execution_flags,
                Default::default(), // BlockLimits::default()
                global_class_cache,
                config.chain.clone(),
            )) as Arc<dyn ExecutorFactory>
        };

        // --- Build gas oracle (sampled from base chain)
        let gas_oracle = GasPriceOracle::sampled_starknet(config.base_chain_url.clone());

        // --- Build StarknetApiConfig
        let versioned_constant_overrides = executor_factory.overrides().cloned();
        let starknet_api_config = StarknetApiConfig {
            max_event_page_size: config.rpc.max_event_page_size,
            max_proof_keys: config.rpc.max_proof_keys,
            max_call_gas: config.rpc.max_call_gas,
            max_concurrent_estimate_fee_requests: config.rpc.max_concurrent_estimate_fee_requests,
            simulation_flags: executor_factory.execution_flags().clone(),
            versioned_constant_overrides,
        };

        // --- Build runtime (scheduler + workers)
        let runtime = Runtime::new(config.worker_count, config.time_quantum);
        let handle = runtime.handle();

        // --- Build shard manager
        let manager: Arc<dyn ShardManager> = Arc::new(LazyShardManager::new(
            config.chain.clone(),
            executor_factory,
            gas_oracle,
            starknet_api_config,
            task_spawner.clone(),
            config.base_chain_url.clone(),
        ));

        // --- Build RPC server with shard API
        let provider = NodeShardProvider {
            manager: manager.clone(),
            handle: handle.clone(),
            chain_spec: config.chain.clone(),
        };

        let mut rpc_modules = RpcModule::new(());
        rpc_modules.merge(ShardApiServer::into_rpc(ShardRpc::new(provider)))?;

        let rpc_server = RpcServer::new().health_check(true).module(rpc_modules)?;

        let config = Arc::new(config);

        Ok(Node {
            config,
            manager,
            handle,
            task_manager,
            rpc_server,
            runtime: Mutex::new(Some(runtime)),
        })
    }

    pub async fn launch(self) -> Result<LaunchedShardNode> {
        self.runtime.lock().as_mut().expect("runtime already taken").start();

        let rpc = self.rpc_server.start(self.config.rpc.socket_addr()).await?;

        info!(
            addr = %rpc.addr(),
            workers = self.config.worker_count,
            "Shard node launched."
        );

        Ok(LaunchedShardNode { node: self, rpc })
    }
}

/// Handle to a running shard node.
pub struct LaunchedShardNode {
    pub node: Node,
    pub rpc: RpcServerHandle,
}

impl LaunchedShardNode {
    pub async fn stop(&self) -> Result<()> {
        self.rpc.stop()?;

        let runtime = self.node.runtime.lock().take();
        if let Some(runtime) = runtime {
            let _ = self
                .node
                .task_manager
                .task_spawner()
                .spawn_blocking(move || {
                    runtime.shutdown_timeout(std::time::Duration::from_secs(30));
                })
                .await;
        }

        self.node.task_manager.shutdown().await;
        Ok(())
    }

    pub fn stopped(&self) -> ShardNodeStoppedFuture<'_> {
        ShardNodeStoppedFuture::new(self)
    }
}

// ---------------------------------------------------------------------------
// ShardProvider — bridges node-specific shard types into the rpc-server layer
// ---------------------------------------------------------------------------

/// Implements [`ShardProvider`] for the shard node, connecting the shard
/// manager and scheduler to the generic `ShardRpc` RPC handler.
struct NodeShardProvider {
    manager: Arc<dyn ShardManager>,
    handle: RuntimeHandle,
    chain_spec: Arc<ChainSpec>,
}

impl ShardProvider for NodeShardProvider {
    type Api = StarknetApi<TxPool, NoPendingBlockProvider, DbProviderFactory>;

    fn starknet_api(&self, shard_id: ContractAddress) -> Result<Self::Api, ErrorObjectOwned> {
        let shard = self.manager.get(shard_id).map_err(|e| {
            ErrorObjectOwned::owned(
                jsonrpsee::types::error::INVALID_PARAMS_CODE,
                format!("Shard not found: {e}"),
                None::<()>,
            )
        })?;
        Ok(shard.starknet_api.clone())
    }

    fn starknet_api_for_write(
        &self,
        shard_id: ContractAddress,
    ) -> Result<Self::Api, ErrorObjectOwned> {
        let shard = self.manager.get(shard_id).map_err(|e| {
            ErrorObjectOwned::owned(
                jsonrpsee::types::error::INTERNAL_ERROR_CODE,
                format!("Failed to get shard: {e}"),
                None::<()>,
            )
        })?;
        Ok(shard.starknet_api.clone())
    }

    fn schedule(&self, shard_id: ContractAddress) {
        if let Ok(shard) = self.manager.get(shard_id) {
            self.handle.schedule(shard);
        }
    }

    fn shard_ids(&self) -> Vec<ContractAddress> {
        self.manager.shard_ids()
    }

    fn chain_id(&self) -> Felt {
        self.chain_spec.id().id()
    }
}
