use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

use anyhow::Result;
use katana_chain_spec::ChainSpec;
use katana_core::backend::Backend;
use katana_core::env::BlockContextGenerator;
use katana_executor::ExecutorFactory;
use katana_gas_price_oracle::GasPriceOracle;
use katana_pool::ordering::FiFo;
use katana_pool::validation::stateful::TxValidator;
use katana_pool::TxPool;
use katana_primitives::env::BlockEnv;
use katana_primitives::transaction::TxHash;
use katana_primitives::ContractAddress;
use katana_provider::api::env::BlockEnvProvider;
use katana_provider::api::state::{StateFactoryProvider, StateProvider};
use katana_provider::{DbProviderFactory, ProviderFactory};
use katana_rpc_server::starknet::{PendingBlockProvider, StarknetApi, StarknetApiConfig};
use katana_rpc_types::{
    PreConfirmedBlockWithReceipts, PreConfirmedBlockWithTxHashes, PreConfirmedBlockWithTxs,
    PreConfirmedStateUpdate, RpcTxWithHash, TxReceiptWithBlockInfo, TxTrace,
};
use katana_tasks::TaskSpawner;
use parking_lot::{Mutex, RwLock};

type StarknetApiResult<T> = Result<T, katana_rpc_api::error::starknet::StarknetApiError>;

/// A shard identifier, corresponding to a contract address.
pub type ShardId = ContractAddress;

/// The state of a shard in the scheduler.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShardState {
    /// Shard has no pending transactions and is not scheduled.
    Idle = 0,
    /// Shard has been enqueued in the scheduler but is not yet being processed.
    Pending = 1,
    /// Shard is actively being processed by a worker.
    Running = 2,
}

impl From<u8> for ShardState {
    fn from(v: u8) -> Self {
        match v {
            0 => ShardState::Idle,
            1 => ShardState::Pending,
            2 => ShardState::Running,
            _ => ShardState::Idle,
        }
    }
}

/// A trivial `PendingBlockProvider` that always returns `None`.
///
/// Since shards don't have a block producer, there's no pending block —
/// everything is immediately committed to storage by the worker.
#[derive(Debug)]
pub struct NoPendingBlockProvider;

impl PendingBlockProvider for NoPendingBlockProvider {
    fn pending_state(&self) -> StarknetApiResult<Option<Box<dyn StateProvider>>> {
        Ok(None)
    }

    fn get_pending_state_update(&self) -> StarknetApiResult<Option<PreConfirmedStateUpdate>> {
        Ok(None)
    }

    fn get_pending_block_with_txs(&self) -> StarknetApiResult<Option<PreConfirmedBlockWithTxs>> {
        Ok(None)
    }

    fn get_pending_block_with_receipts(
        &self,
    ) -> StarknetApiResult<Option<PreConfirmedBlockWithReceipts>> {
        Ok(None)
    }

    fn get_pending_block_with_tx_hashes(
        &self,
    ) -> StarknetApiResult<Option<PreConfirmedBlockWithTxHashes>> {
        Ok(None)
    }

    fn get_pending_transaction(&self, _hash: TxHash) -> StarknetApiResult<Option<RpcTxWithHash>> {
        Ok(None)
    }

    fn get_pending_receipt(
        &self,
        _hash: TxHash,
    ) -> StarknetApiResult<Option<TxReceiptWithBlockInfo>> {
        Ok(None)
    }

    fn get_pending_trace(&self, _hash: TxHash) -> StarknetApiResult<Option<TxTrace>> {
        Ok(None)
    }

    fn get_pending_transaction_by_index(
        &self,
        _index: katana_primitives::transaction::TxNumber,
    ) -> StarknetApiResult<Option<RpcTxWithHash>> {
        Ok(None)
    }
}

/// Per-contract shard with isolated storage, pool, and RPC handler.
#[derive(Debug)]
pub struct Shard {
    pub id: ShardId,
    pub db: katana_db::Db,
    pub provider: DbProviderFactory,
    pub pool: TxPool,
    pub backend: Arc<Backend<DbProviderFactory>>,
    pub block_env: Arc<RwLock<BlockEnv>>,
    pub starknet_api: StarknetApi<TxPool, NoPendingBlockProvider, DbProviderFactory>,
    state: AtomicU8,
}

impl Shard {
    /// Create a new shard with isolated in-memory storage and an initial execution block context.
    pub fn new(
        id: ShardId,
        chain_spec: Arc<ChainSpec>,
        executor_factory: Arc<dyn ExecutorFactory>,
        gas_oracle: GasPriceOracle,
        starknet_api_config: StarknetApiConfig,
        task_spawner: TaskSpawner,
        initial_block_env: BlockEnv,
    ) -> Result<Self> {
        // Per-shard in-memory database
        let db = katana_db::Db::in_memory()?;
        let provider = DbProviderFactory::new(db.clone());

        // Per-shard backend
        let backend = Arc::new(Backend {
            gas_oracle: gas_oracle.clone(),
            storage: provider.clone(),
            executor_factory,
            block_context_generator: parking_lot::RwLock::new(BlockContextGenerator::default()),
            chain_spec: chain_spec.clone(),
        });

        // Initialize genesis state for this shard
        backend.init_genesis(false)?;

        // Build per-shard transaction pool
        // Create validator from latest state
        let db_provider = provider.provider();

        let block_env = db_provider.block_env_at(0.into())?.unwrap_or_default();
        let latest_state = db_provider.latest()?;

        let execution_flags = backend.executor_factory.execution_flags().clone();
        let cfg_env = backend.executor_factory.overrides().cloned();
        let permit = Arc::new(Mutex::new(()));
        let validator = TxValidator::new(
            latest_state,
            execution_flags,
            cfg_env,
            block_env,
            permit,
            chain_spec.clone(),
        );

        let pool = TxPool::new(validator, FiFo::new());

        // Initialize per-shard block env from the latest base-chain context.
        let block_env = Arc::new(RwLock::new(initial_block_env));

        // Build per-shard StarknetApi
        let starknet_api = StarknetApi::new(
            chain_spec,
            pool.clone(),
            task_spawner,
            NoPendingBlockProvider,
            gas_oracle,
            starknet_api_config,
            provider.clone(),
        );

        Ok(Self {
            id,
            db,
            provider,
            pool,
            backend,
            block_env,
            starknet_api,
            state: AtomicU8::new(ShardState::Idle as u8),
        })
    }

    /// Get the current state of this shard.
    pub fn state(&self) -> ShardState {
        ShardState::from(self.state.load(Ordering::SeqCst))
    }

    /// Set the shard state.
    pub fn set_state(&self, state: ShardState) {
        self.state.store(state as u8, Ordering::SeqCst);
    }

    /// Attempt to transition from `expected` to `new` state.
    /// Returns `true` if the transition succeeded.
    pub fn compare_exchange_state(&self, expected: ShardState, new: ShardState) -> bool {
        self.state
            .compare_exchange(expected as u8, new as u8, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    }
}
