use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use jsonrpsee::types::error::INVALID_PARAMS_CODE;
use jsonrpsee::types::ErrorObjectOwned;
use katana_chain_spec::ChainSpec;
use katana_executor::blockifier::cache::{ClassCache, Error as ClassCacheError};
use katana_executor::{
    ExecutionFlags, ExecutionOutput, ExecutionResult, Executor, ExecutorFactory, ExecutorResult,
};
use katana_gas_price_oracle::GasPriceOracle;
use katana_pool::TransactionPool;
use katana_primitives::block::ExecutableBlock;
use katana_primitives::da::DataAvailabilityMode;
use katana_primitives::env::{BlockEnv, VersionedConstantsOverrides};
use katana_primitives::fee::{AllResourceBoundsMapping, ResourceBoundsMapping, Tip};
use katana_primitives::transaction::{ExecutableTxWithHash, TxWithHash};
use katana_primitives::{address, ContractAddress, Felt};
use katana_provider::api::state::StateProvider;
use katana_provider::providers::EmptyStateProvider;
use katana_provider::DbProviderFactory;
use katana_rpc_api::shard::ShardApiServer;
use katana_rpc_server::shard::{ShardProvider, ShardRpc};
use katana_rpc_server::starknet::{StarknetApi, StarknetApiConfig};
use katana_rpc_types::broadcasted::BroadcastedInvokeTx;
use katana_sharding::pool::ShardPool;
use katana_sharding::runtime::Runtime;
use katana_sharding::shard::{NoPendingBlockProvider, Shard, ShardId, ShardState};
use katana_tasks::TaskManager;

#[derive(Debug)]
struct FakeExecutorFactory {
    flags: ExecutionFlags,
}

fn disabled_execution_flags() -> ExecutionFlags {
    ExecutionFlags::new().with_account_validation(false).with_fee(false).with_nonce_check(false)
}

impl FakeExecutorFactory {
    fn new() -> Self {
        Self { flags: disabled_execution_flags() }
    }
}

impl ExecutorFactory for FakeExecutorFactory {
    fn executor(&self, _state: Box<dyn StateProvider>, block_env: BlockEnv) -> Box<dyn Executor> {
        Box::new(FakeExecutor { block_env })
    }

    fn overrides(&self) -> Option<&VersionedConstantsOverrides> {
        None
    }

    fn execution_flags(&self) -> &ExecutionFlags {
        &self.flags
    }
}

#[derive(Debug)]
struct CountingExecutorFactory {
    flags: ExecutionFlags,
    executor_calls: Arc<AtomicUsize>,
}

impl CountingExecutorFactory {
    fn new(executor_calls: Arc<AtomicUsize>) -> Self {
        Self { flags: disabled_execution_flags(), executor_calls }
    }
}

impl ExecutorFactory for CountingExecutorFactory {
    fn executor(&self, _state: Box<dyn StateProvider>, block_env: BlockEnv) -> Box<dyn Executor> {
        self.executor_calls.fetch_add(1, Ordering::SeqCst);
        Box::new(FakeExecutor { block_env })
    }

    fn overrides(&self) -> Option<&VersionedConstantsOverrides> {
        None
    }

    fn execution_flags(&self) -> &ExecutionFlags {
        &self.flags
    }
}

#[derive(Debug)]
struct FakeExecutor {
    block_env: BlockEnv,
}

impl Executor for FakeExecutor {
    fn execute_block(&mut self, _block: ExecutableBlock) -> ExecutorResult<()> {
        Ok(())
    }

    fn execute_transactions(
        &mut self,
        transactions: Vec<ExecutableTxWithHash>,
    ) -> ExecutorResult<(usize, Option<katana_executor::error::ExecutorError>)> {
        Ok((transactions.len(), None))
    }

    fn take_execution_output(&mut self) -> ExecutorResult<ExecutionOutput> {
        Ok(ExecutionOutput::default())
    }

    fn state(&self) -> Box<dyn StateProvider> {
        Box::new(EmptyStateProvider)
    }

    fn transactions(&self) -> &[(TxWithHash, ExecutionResult)] {
        &[]
    }

    fn block_env(&self) -> BlockEnv {
        self.block_env.clone()
    }
}

#[derive(Clone)]
struct SingleShardProvider {
    shard_id: ShardId,
    chain_id: Felt,
    api: StarknetApi<ShardPool, NoPendingBlockProvider, DbProviderFactory>,
}

impl ShardProvider for SingleShardProvider {
    type Api = StarknetApi<ShardPool, NoPendingBlockProvider, DbProviderFactory>;

    fn starknet_api(&self, shard_id: ContractAddress) -> Result<Self::Api, ErrorObjectOwned> {
        if shard_id == self.shard_id {
            Ok(self.api.clone())
        } else {
            Err(ErrorObjectOwned::owned(
                INVALID_PARAMS_CODE,
                format!("unknown shard id: {shard_id}"),
                None::<()>,
            ))
        }
    }

    fn shard_ids(&self) -> Vec<ContractAddress> {
        vec![self.shard_id]
    }

    fn chain_id(&self) -> Felt {
        self.chain_id
    }
}

#[derive(Clone)]
struct MultiShardProvider {
    chain_id: Felt,
    apis: Arc<HashMap<ShardId, StarknetApi<ShardPool, NoPendingBlockProvider, DbProviderFactory>>>,
}

impl ShardProvider for MultiShardProvider {
    type Api = StarknetApi<ShardPool, NoPendingBlockProvider, DbProviderFactory>;

    fn starknet_api(&self, shard_id: ContractAddress) -> Result<Self::Api, ErrorObjectOwned> {
        self.apis.get(&shard_id).cloned().ok_or_else(|| {
            ErrorObjectOwned::owned(
                INVALID_PARAMS_CODE,
                format!("unknown shard id: {shard_id}"),
                None::<()>,
            )
        })
    }

    fn shard_ids(&self) -> Vec<ContractAddress> {
        self.apis.keys().copied().collect()
    }

    fn chain_id(&self) -> Felt {
        self.chain_id
    }
}

fn build_invoke_transaction(sender: ContractAddress) -> BroadcastedInvokeTx {
    BroadcastedInvokeTx {
        sender_address: sender,
        calldata: vec![],
        signature: vec![],
        nonce: Felt::ZERO,
        paymaster_data: vec![],
        tip: Tip::default(),
        account_deployment_data: vec![],
        resource_bounds: ResourceBoundsMapping::All(AllResourceBoundsMapping::default()),
        fee_data_availability_mode: DataAvailabilityMode::L1,
        nonce_data_availability_mode: DataAvailabilityMode::L1,
        is_query: false,
    }
}

fn starknet_api_config() -> StarknetApiConfig {
    StarknetApiConfig {
        max_event_page_size: None,
        max_proof_keys: None,
        max_call_gas: None,
        max_concurrent_estimate_fee_requests: None,
        simulation_flags: disabled_execution_flags(),
        versioned_constant_overrides: None,
    }
}

fn ensure_global_class_cache() {
    match ClassCache::try_global() {
        Ok(_) => {}
        Err(ClassCacheError::NotInitialized) => match ClassCache::builder().build_global() {
            Ok(_) | Err(ClassCacheError::AlreadyInitialized) => {}
            Err(err) => panic!("failed to initialize global class cache: {err}"),
        },
        Err(err) => panic!("unexpected class cache error: {err}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_schedules_shard_after_rpc_add_invoke() {
    ensure_global_class_cache();

    let task_manager = TaskManager::current();
    let task_spawner = task_manager.task_spawner();

    let runtime = Runtime::new(1, Duration::from_millis(10));
    let scheduler = runtime.handle().scheduler().clone();

    let chain_spec = Arc::new(ChainSpec::dev());
    let shard_id = address!("0x12345");
    let sender = address!("0x1");
    let chain_id = chain_spec.id().id();

    let executor_factory: Arc<dyn ExecutorFactory> = Arc::new(FakeExecutorFactory::new());

    let shard = Arc::new(
        Shard::new(
            shard_id,
            Arc::clone(&chain_spec),
            executor_factory,
            GasPriceOracle::create_for_testing(),
            starknet_api_config(),
            task_spawner,
            BlockEnv { number: 1, ..BlockEnv::default() },
            scheduler.clone(),
        )
        .expect("failed to create shard"),
    );

    scheduler.shard_registry().write().insert(shard_id, Arc::clone(&shard));

    let rpc =
        ShardRpc::new(SingleShardProvider { shard_id, chain_id, api: shard.starknet_api.clone() });

    let tx = build_invoke_transaction(sender);
    let add_response = ShardApiServer::add_invoke_transaction(&rpc, shard_id, tx)
        .await
        .expect("failed to add invoke transaction");
    let tx_hash = add_response.transaction_hash;

    assert!(shard.pool.contains(tx_hash), "tx should be present in shard pool after RPC add");
    assert_eq!(shard.pool.size(), 1, "exactly one tx should be pending in pool");
    assert_eq!(shard.state(), ShardState::Pending, "shard should be pending after scheduling");

    let scheduled = scheduler.next_task().expect("scheduler queue should contain the shard");
    assert_eq!(scheduled.id, shard_id, "wrong shard was scheduled");
    assert_eq!(scheduled.state(), ShardState::Pending, "scheduled shard state should be pending");

    runtime.shutdown_timeout(Duration::from_secs(2));
    task_manager.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_executes_all_shards_when_shard_count_exceeds_worker_count() {
    ensure_global_class_cache();

    let task_manager = TaskManager::current();
    let task_spawner = task_manager.task_spawner();

    let worker_count = 2usize;
    let shard_count = 5usize;

    let mut runtime = Runtime::new(worker_count, Duration::from_millis(0));
    let scheduler = runtime.handle().scheduler().clone();

    let chain_spec = Arc::new(ChainSpec::dev());
    let chain_id = chain_spec.id().id();

    let mut apis = HashMap::with_capacity(shard_count);
    let mut shards = Vec::with_capacity(shard_count);
    let mut executor_calls = Vec::with_capacity(shard_count);

    for i in 0..shard_count {
        let shard_id = ContractAddress::new(Felt::from(0x1000_u64 + i as u64));
        let calls = Arc::new(AtomicUsize::new(0));
        let executor_factory: Arc<dyn ExecutorFactory> =
            Arc::new(CountingExecutorFactory::new(Arc::clone(&calls)));

        let shard = Arc::new(
            Shard::new(
                shard_id,
                Arc::clone(&chain_spec),
                executor_factory,
                GasPriceOracle::create_for_testing(),
                starknet_api_config(),
                task_spawner.clone(),
                BlockEnv { number: 1, ..BlockEnv::default() },
                scheduler.clone(),
            )
            .expect("failed to create shard"),
        );

        scheduler.register_shard(Arc::clone(&shard));
        apis.insert(shard_id, shard.starknet_api.clone());
        shards.push(shard);
        executor_calls.push(calls);
    }

    let rpc = ShardRpc::new(MultiShardProvider { chain_id, apis: Arc::new(apis) });

    for (i, shard) in shards.iter().enumerate() {
        let sender = ContractAddress::new(Felt::from(0x2000_u64 + i as u64));
        let tx = build_invoke_transaction(sender);
        ShardApiServer::add_invoke_transaction(&rpc, shard.id, tx)
            .await
            .expect("failed to add invoke transaction");
    }

    assert_eq!(
        scheduler.queue_len(),
        shard_count,
        "all shards should be queued before workers start",
    );

    runtime.start();

    let wait = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let every_executor_called =
                executor_calls.iter().all(|calls| calls.load(Ordering::SeqCst) > 0);
            let queue_empty = scheduler.queue_len() == 0;
            let all_idle = shards.iter().all(|shard| shard.state() == ShardState::Idle);

            if every_executor_called && queue_empty && all_idle {
                break;
            }

            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;

    assert!(
        wait.is_ok(),
        "runtime did not process all scheduled shards in time; queue_len={}",
        scheduler.queue_len(),
    );

    for (i, calls) in executor_calls.iter().enumerate() {
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "executor factory for shard index {i} should be called exactly once",
        );
    }

    assert_eq!(scheduler.queue_len(), 0, "queue should be empty after all shards are executed");

    runtime.shutdown_timeout(Duration::from_secs(2));
    task_manager.shutdown().await;
}
