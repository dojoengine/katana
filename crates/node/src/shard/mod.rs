pub mod block_context;
pub mod config;
pub mod exit;
pub mod registry;
pub mod scheduler;
pub mod types;
pub mod worker;

use std::sync::Arc;
use std::thread;

use anyhow::Result;
use jsonrpsee::core::async_trait;
use jsonrpsee::types::ErrorObjectOwned;
use jsonrpsee::RpcModule;
use katana_chain_spec::ChainSpec;
use katana_executor::blockifier::cache::ClassCache;
use katana_executor::blockifier::BlockifierFactory;
use katana_executor::{ExecutionFlags, ExecutorFactory};
use katana_gas_price_oracle::{FixedPriceOracle, GasPriceOracle};
use katana_pool::TxPool;
use katana_primitives::block::BlockIdOrTag;
use katana_primitives::contract::{Nonce, StorageKey};
use katana_primitives::env::{BlockEnv, VersionedConstantsOverrides};
use katana_primitives::transaction::TxHash;
use katana_primitives::{ContractAddress, Felt};
use katana_provider::DbProviderFactory;
use katana_rpc_api::shard::ShardApiServer;
use katana_rpc_api::starknet::{StarknetApiServer, StarknetTraceApiServer, StarknetWriteApiServer};
use katana_rpc_client::starknet::Client as StarknetClient;
use katana_rpc_server::shard::{ShardLookup, ShardRpcApi, ShardStarknetApi};
use katana_rpc_server::starknet::{StarknetApi, StarknetApiConfig};
use katana_rpc_server::{RpcServer, RpcServerHandle};
use katana_rpc_types::block::{
    BlockHashAndNumberResponse, BlockNumberResponse, GetBlockWithTxHashesResponse,
    MaybePreConfirmedBlock,
};
use katana_rpc_types::broadcasted::{
    AddDeclareTransactionResponse, AddDeployAccountTransactionResponse,
    AddInvokeTransactionResponse, BroadcastedDeclareTx, BroadcastedDeployAccountTx,
    BroadcastedInvokeTx, BroadcastedTx,
};
use katana_rpc_types::event::{EventFilterWithPage, GetEventsResponse};
use katana_rpc_types::receipt::TxReceiptWithBlockInfo;
use katana_rpc_types::state_update::StateUpdate;
use katana_rpc_types::trace::TxTrace;
use katana_rpc_types::transaction::RpcTxWithHash;
use katana_rpc_types::{
    CallResponse, EstimateFeeSimulationFlag, FeeEstimate, FunctionCall, TxStatus,
};
use katana_tasks::TaskManager;
use parking_lot::{Mutex, RwLock};
use tracing::info;

use self::block_context::BlockContextListener;
use self::config::ShardNodeConfig;
use self::exit::ShardNodeStoppedFuture;
use self::registry::ShardRegistry;
use self::scheduler::ShardScheduler;
use self::types::NoPendingBlockProvider;

/// A shard node instance.
#[must_use = "ShardNode does nothing unless launched."]
#[derive(Debug)]
pub struct ShardNode {
    pub config: Arc<ShardNodeConfig>,
    pub registry: ShardRegistry,
    pub scheduler: ShardScheduler,
    pub block_env: Arc<RwLock<BlockEnv>>,
    pub task_manager: TaskManager,
    pub rpc_server: RpcServer,
    pub block_context_listener: BlockContextListener,
}

impl ShardNode {
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

        // --- Build gas oracle (fixed for shard node)
        let gas_oracle = GasPriceOracle::Fixed(FixedPriceOracle::default());

        // --- Build StarknetApiConfig
        let versioned_constant_overrides = executor_factory.overrides().cloned();
        let starknet_api_config = StarknetApiConfig {
            max_event_page_size: config.rpc.max_event_page_size,
            max_proof_keys: config.rpc.max_proof_keys,
            max_call_gas: config.rpc.max_call_gas,
            max_concurrent_estimate_fee_requests: config.rpc.max_concurrent_estimate_fee_requests,
            simulation_flags: executor_factory.execution_flags().clone(),
            versioned_constant_overrides,
            #[cfg(feature = "cartridge")]
            paymaster: None,
        };

        // --- Build initial BlockEnv from genesis
        let genesis = config.chain.genesis();
        let block_env = Arc::new(RwLock::new(BlockEnv {
            number: genesis.number,
            timestamp: genesis.timestamp,
            sequencer_address: genesis.sequencer_address,
            l1_gas_prices: genesis.gas_prices.clone(),
            l2_gas_prices: genesis.gas_prices.clone(),
            l1_data_gas_prices: genesis.gas_prices.clone(),
            starknet_version: katana_primitives::version::CURRENT_STARKNET_VERSION,
        }));

        // --- Build base chain client and block context listener
        let starknet_client = StarknetClient::new(config.base_chain_url.clone());
        let block_context_listener = BlockContextListener::new(
            starknet_client,
            block_env.clone(),
            config.chain.clone(),
            config.block_poll_interval,
        );

        // --- Build registry
        let registry = ShardRegistry::new(
            config.chain.clone(),
            executor_factory,
            gas_oracle,
            starknet_api_config,
            task_spawner.clone(),
        );

        // --- Build scheduler
        let scheduler = ShardScheduler::new(config.time_quantum);

        // --- Build RPC server with shard API
        let shard_lookup = NodeShardLookup {
            registry: registry.clone(),
            scheduler: scheduler.clone(),
            chain_spec: config.chain.clone(),
        };

        let shard_rpc_api = ShardRpcApi::new(shard_lookup);
        let mut rpc_modules = RpcModule::new(());
        rpc_modules.merge(ShardApiServer::into_rpc(shard_rpc_api))?;

        let rpc_server = RpcServer::new().health_check(true).module(rpc_modules)?;

        let config = Arc::new(config);

        Ok(ShardNode {
            config,
            registry,
            scheduler,
            block_env,
            task_manager,
            rpc_server,
            block_context_listener,
        })
    }

    pub async fn launch(self) -> Result<LaunchedShardNode> {
        let listener = self.block_context_listener.clone();
        self.task_manager
            .task_spawner()
            .build_task()
            .graceful_shutdown()
            .name("Block context listener")
            .spawn(listener.run());

        let worker_handles = worker::spawn_workers(
            self.config.worker_count,
            self.scheduler.clone(),
            self.block_env.clone(),
        );

        let rpc = self.rpc_server.start(self.config.rpc.socket_addr()).await?;

        info!(
            addr = %rpc.addr(),
            workers = self.config.worker_count,
            "Shard node launched."
        );

        Ok(LaunchedShardNode {
            node: self,
            rpc,
            worker_handles: parking_lot::Mutex::new(worker_handles),
        })
    }
}

/// Handle to a running shard node.
pub struct LaunchedShardNode {
    pub node: ShardNode,
    pub rpc: RpcServerHandle,
    /// Worker thread handles, taken during shutdown to join them.
    worker_handles: Mutex<Vec<thread::JoinHandle<()>>>,
}

impl LaunchedShardNode {
    pub async fn stop(&self) -> Result<()> {
        // Signal the scheduler to stop — workers will exit their loops.
        self.node.scheduler.shutdown();
        self.rpc.stop()?;

        // Join worker threads via the task manager's blocking executor.
        let handles: Vec<_> = self.worker_handles.lock().drain(..).collect();
        let _ = self
            .node
            .task_manager
            .task_spawner()
            .cpu_bound()
            .spawn(move || {
                for h in handles {
                    let _ = h.join();
                }
            })
            .await;

        self.node.task_manager.shutdown().await;
        Ok(())
    }

    pub fn stopped(&self) -> ShardNodeStoppedFuture<'_> {
        ShardNodeStoppedFuture::new(self)
    }
}

// ---------------------------------------------------------------------------
// Bridge between the node crate's shard types and the rpc-server's traits
// ---------------------------------------------------------------------------

/// Implements `ShardLookup` for the shard node, bridging registry/scheduler to the RPC layer.
struct NodeShardLookup {
    registry: ShardRegistry,
    scheduler: ShardScheduler,
    chain_spec: Arc<ChainSpec>,
}

impl ShardLookup for NodeShardLookup {
    fn get_starknet_api(
        &self,
        shard_id: &ContractAddress,
    ) -> Result<Arc<dyn ShardStarknetApi>, ErrorObjectOwned> {
        let shard = self.registry.get(shard_id).ok_or_else(|| {
            ErrorObjectOwned::owned(
                jsonrpsee::types::error::INVALID_PARAMS_CODE,
                "Shard not found",
                None::<()>,
            )
        })?;
        Ok(Arc::new(ShardStarknetApiImpl { api: shard.starknet_api.clone() }))
    }

    fn get_or_create_starknet_api(
        &self,
        shard_id: ContractAddress,
    ) -> Result<Arc<dyn ShardStarknetApi>, ErrorObjectOwned> {
        let shard = self.registry.get_or_create(shard_id).map_err(|e| {
            ErrorObjectOwned::owned(
                jsonrpsee::types::error::INTERNAL_ERROR_CODE,
                format!("Failed to create shard: {e}"),
                None::<()>,
            )
        })?;
        Ok(Arc::new(ShardStarknetApiImpl { api: shard.starknet_api.clone() }))
    }

    fn schedule_shard(&self, shard_id: ContractAddress) {
        if let Some(shard) = self.registry.get(&shard_id) {
            self.scheduler.schedule(shard);
        }
    }

    fn shard_ids(&self) -> Vec<ContractAddress> {
        self.registry.shard_ids()
    }

    fn chain_id(&self) -> Felt {
        self.chain_spec.id().id()
    }
}

/// Wraps a concrete `StarknetApi<TxPool, NoPendingBlockProvider, DbProviderFactory>` and
/// implements `ShardStarknetApi` by delegating to the `StarknetApiServer` /
/// `StarknetWriteApiServer` / `StarknetTraceApiServer` trait methods.
struct ShardStarknetApiImpl {
    api: StarknetApi<TxPool, NoPendingBlockProvider, DbProviderFactory>,
}

#[async_trait]
impl ShardStarknetApi for ShardStarknetApiImpl {
    async fn get_block_with_tx_hashes(
        &self,
        block_id: BlockIdOrTag,
    ) -> jsonrpsee::core::RpcResult<GetBlockWithTxHashesResponse> {
        StarknetApiServer::get_block_with_tx_hashes(&self.api, block_id).await
    }

    async fn get_block_with_txs(
        &self,
        block_id: BlockIdOrTag,
    ) -> jsonrpsee::core::RpcResult<MaybePreConfirmedBlock> {
        StarknetApiServer::get_block_with_txs(&self.api, block_id).await
    }

    async fn get_storage_at(
        &self,
        contract_address: ContractAddress,
        key: StorageKey,
        block_id: BlockIdOrTag,
    ) -> jsonrpsee::core::RpcResult<Felt> {
        StarknetApiServer::get_storage_at(&self.api, contract_address, key, block_id).await
    }

    async fn get_nonce(
        &self,
        block_id: BlockIdOrTag,
        contract_address: ContractAddress,
    ) -> jsonrpsee::core::RpcResult<Nonce> {
        StarknetApiServer::get_nonce(&self.api, block_id, contract_address).await
    }

    async fn get_transaction_by_hash(
        &self,
        tx_hash: TxHash,
    ) -> jsonrpsee::core::RpcResult<RpcTxWithHash> {
        StarknetApiServer::get_transaction_by_hash(&self.api, tx_hash).await
    }

    async fn get_transaction_receipt(
        &self,
        tx_hash: TxHash,
    ) -> jsonrpsee::core::RpcResult<TxReceiptWithBlockInfo> {
        StarknetApiServer::get_transaction_receipt(&self.api, tx_hash).await
    }

    async fn get_transaction_status(
        &self,
        tx_hash: TxHash,
    ) -> jsonrpsee::core::RpcResult<TxStatus> {
        StarknetApiServer::get_transaction_status(&self.api, tx_hash).await
    }

    async fn call(
        &self,
        request: FunctionCall,
        block_id: BlockIdOrTag,
    ) -> jsonrpsee::core::RpcResult<CallResponse> {
        StarknetApiServer::call(&self.api, request, block_id).await
    }

    async fn get_events(
        &self,
        filter: EventFilterWithPage,
    ) -> jsonrpsee::core::RpcResult<GetEventsResponse> {
        StarknetApiServer::get_events(&self.api, filter).await
    }

    async fn estimate_fee(
        &self,
        request: Vec<BroadcastedTx>,
        simulation_flags: Vec<EstimateFeeSimulationFlag>,
        block_id: BlockIdOrTag,
    ) -> jsonrpsee::core::RpcResult<Vec<FeeEstimate>> {
        StarknetApiServer::estimate_fee(&self.api, request, simulation_flags, block_id).await
    }

    async fn block_hash_and_number(
        &self,
    ) -> jsonrpsee::core::RpcResult<BlockHashAndNumberResponse> {
        StarknetApiServer::block_hash_and_number(&self.api).await
    }

    async fn block_number(&self) -> jsonrpsee::core::RpcResult<BlockNumberResponse> {
        StarknetApiServer::block_number(&self.api).await
    }

    async fn get_state_update(
        &self,
        block_id: BlockIdOrTag,
    ) -> jsonrpsee::core::RpcResult<StateUpdate> {
        StarknetApiServer::get_state_update(&self.api, block_id).await
    }

    async fn add_invoke_transaction(
        &self,
        tx: BroadcastedInvokeTx,
    ) -> jsonrpsee::core::RpcResult<AddInvokeTransactionResponse> {
        StarknetWriteApiServer::add_invoke_transaction(&self.api, tx).await
    }

    async fn add_declare_transaction(
        &self,
        tx: BroadcastedDeclareTx,
    ) -> jsonrpsee::core::RpcResult<AddDeclareTransactionResponse> {
        StarknetWriteApiServer::add_declare_transaction(&self.api, tx).await
    }

    async fn add_deploy_account_transaction(
        &self,
        tx: BroadcastedDeployAccountTx,
    ) -> jsonrpsee::core::RpcResult<AddDeployAccountTransactionResponse> {
        StarknetWriteApiServer::add_deploy_account_transaction(&self.api, tx).await
    }

    async fn trace_transaction(&self, tx_hash: TxHash) -> jsonrpsee::core::RpcResult<TxTrace> {
        StarknetTraceApiServer::trace_transaction(&self.api, tx_hash).await
    }
}
