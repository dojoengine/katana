// # Block Producer State Machine
//
// The block producer is driven by `poll_next`, which runs a loop over four states.
// Two flags (`force_seal`, `block_full`) and the [`MiningMode`] policy together determine
// when a block should be sealed. The policy only controls timing; the producer owns the
// hard constraints.
//
// ## States
//
// ```text
//                        ┌──────┐
//                  ┌─────│ Idle │◄──────────────────────┐
//                  │     └──────┘                        │
//                  │         │                           │
//     force_mine & │  txs queued                         │
//     queue empty  │         │                           │
//                  │         ▼                           │
//                  │   ┌───────────┐     ┌──────┐       │
//                  │   │ Executing │────►│ Open │       │
//                  │   └───────────┘     └──────┘       │
//                  │         ▲              │  │         │
//                  │         │   more txs   │  │         │
//                  │         └──────────────┘  │         │
//                  │                    seal   │         │
//                  │                  trigger  │         │
//                  │                           ▼         │
//                  │                     ┌─────────┐     │
//                  └────────────────────►│ Sealing │─────┘
//                                        └─────────┘
// ```
//
// - **Idle**: No block is open. Waiting for transactions or a force-mine command.
//   Initial state, and the state after every successful seal.
//
// - **Executing** (intermediate): A batch of transactions is being executed on the
//   blocking thread pool. Transitions to Open when the execution future completes.
//
// - **Open**: A block is open with executed transactions. The block accepts more
//   transactions (→ Executing) or waits for a seal trigger (→ Sealing).
//
// - **Sealing** (intermediate): The block is being committed to storage on the
//   blocking thread pool. Transitions to Idle when the seal future completes.
//
// ## Seal trigger
//
// ```text
//   should_seal = force_seal          ← ForceMine RPC command
//               | block_full          ← executor reported limits exhausted
//               | mode.poll_seal(cx)  ← policy timing (instant/interval/on-demand)
// ```
//
// The policy decides timing. The producer enforces hard constraints (`force_seal`,
// `block_full`) regardless of the policy.
//
// ## Transitions
//
// ### Idle
//
// ```text
//   should_seal && queued.empty()  →  Sealing    (force mine with no txs)
//   !queued.empty()                →  Executing   (opens a new block)
//   otherwise                      →  park
// ```
//
// If should_seal is true but there are queued txs, they are executed first
// (→ Executing → Open), and the seal happens from the Open state.
//
// ### Executing
//
// ```text
//   future pending               →  park
//   future ready, Ok(outcome)    →  Open
//     • leftovers pushed to front of queued
//     • if block_full: set block_full flag
//   future ready, Err            →  yield error
// ```
//
// ### Open
//
// ```text
//   should_seal       →  Sealing
//   !queued.empty()   →  Executing   (execute more txs into the current block)
//   otherwise         →  park
// ```
//
// ### Sealing
//
// ```text
//   future pending               →  park
//   future ready, Ok(outcome)    →  Idle
//     • remove mined txs from pool
//     • create new executor for next block
//     • update validator
//     • mode.on_sealed()
//     • yield Ok(outcome)
//   future ready, Err            →  yield error
// ```
//
// ## Block lifecycle
//
// ```text
//   Idle ──► Executing ──► Open ──► Sealing ──► Idle
//               ▲            │
//               └────────────┘
//             (more queued txs)
// ```
//
// A block is opened implicitly when the first batch of transactions enters execution.
// It remains open while more transactions are executed into it. When a seal trigger
// fires (policy timing, block full, or force mine), the block is sealed and the
// producer returns to idle.
//
// ## Mining modes
//
// | Mode       | poll_seal returns true when...    | Typical flow                               |
// |------------|----------------------------------|--------------------------------------------|
// | Instant    | on_txs_received was called       | Idle → Executing → Open → Sealing → Idle  |
// | Interval   | timer fires (started on first tx)| Idle → Executing → Open → ... → Sealing   |
// | OnDemand   | never                            | Idle → Executing → Open → (force) Sealing |
//
// In all modes, `block_full` and `force_seal` bypass the policy and trigger sealing
// unconditionally.

use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use futures::stream::StreamExt;
use futures::FutureExt;
use katana_executor::{ExecutionStats, Executor};
use katana_pool::validation::stateful::TxValidator;
use katana_pool::{PendingTransactions, TransactionPool, TxPool};
use katana_primitives::block::BlockHash;
use katana_primitives::transaction::{ExecutableTxWithHash, TxHash};
use katana_provider::api::block::BlockNumberProvider;
use katana_provider::api::env::BlockEnvProvider;
use katana_provider::api::state::{StateFactoryProvider, StateProvider};
use katana_provider::{ProviderError, ProviderFactory, ProviderRO, ProviderRW};
use katana_tasks::{CpuBlockingTaskPool, Result as TaskResult};
use parking_lot::{Mutex, RwLock};
use tokio::time::{interval_at, Instant, Interval};
use tracing::{info, trace};

use crate::backend::Backend;

#[cfg(test)]
#[path = "block_producer_tests.rs"]
mod tests;

pub(crate) const LOG_TARGET: &str = "miner";

#[derive(Debug, thiserror::Error)]
pub enum BlockProductionError {
    #[error(transparent)]
    Provider(#[from] ProviderError),

    #[error("transaction execution task cancelled")]
    ExecutionTaskCancelled,

    #[error("transaction execution error: {0}")]
    TransactionExecutionError(#[from] katana_executor::error::ExecutorError),

    #[error("inconsistent state updates: {0}")]
    InconsistentState(String),
}

impl BlockProductionError {
    /// Returns `true` if the error is caused by block limit being exhausted.
    pub fn is_block_limit_exhausted(&self) -> bool {
        matches!(
            self,
            Self::TransactionExecutionError(katana_executor::error::ExecutorError::LimitsExhausted)
        )
    }
}

#[derive(Debug, Clone)]
pub struct MinedBlockOutcome {
    pub block_hash: BlockHash,
    pub block_number: u64,
    pub txs: Vec<TxHash>,
    pub stats: ExecutionStats,
}

type ServiceFuture<T> = Pin<Box<dyn Future<Output = TaskResult<T>> + Send>>;

type BlockProductionResult = Result<MinedBlockOutcome, BlockProductionError>;
type BlockProductionFuture = ServiceFuture<BlockProductionResult>;
type TxExecutionFuture = ServiceFuture<Result<TxExecutionOutcome, BlockProductionError>>;
type PoolPendingTransactions =
    PendingTransactions<ExecutableTxWithHash, katana_pool::ordering::FiFo<ExecutableTxWithHash>>;

#[derive(Debug, Clone, derive_more::Deref)]
pub struct PendingExecutor(#[deref] Arc<RwLock<Box<dyn Executor>>>);

impl PendingExecutor {
    fn new(executor: Box<dyn Executor>) -> Self {
        Self(Arc::new(RwLock::new(executor)))
    }
}

// --- Mining mode ---

/// Defines the block production lifecycle: when a block should be sealed.
///
/// The mining mode is only concerned with timing — it tells the block producer *when* to seal.
/// Hard constraints like block-full and force-mine are handled by the producer directly.
trait MiningMode: Send + Sync {
    /// Notify that new transactions have been received.
    fn on_txs_received(&mut self);

    /// Poll whether the current block should be sealed.
    ///
    /// Once this returns `true`, it must continue returning `true` until
    /// [`on_sealed`](Self::on_sealed) is called.
    fn poll_seal(&mut self, cx: &mut Context<'_>) -> bool;

    /// Called after a block is sealed. Resets internal state for the next block.
    fn on_sealed(&mut self);
}

/// Seals a block as soon as there are transactions.
#[derive(Debug, Default)]
struct InstantMode {
    should_seal: bool,
}

impl MiningMode for InstantMode {
    fn on_txs_received(&mut self) {
        self.should_seal = true;
    }

    fn poll_seal(&mut self, _cx: &mut Context<'_>) -> bool {
        self.should_seal
    }

    fn on_sealed(&mut self) {
        self.should_seal = false;
    }
}

/// Never seals on its own. Blocks are only sealed via force mine or when the block is full.
#[derive(Debug, Default)]
struct OnDemandMode;

impl MiningMode for OnDemandMode {
    fn on_txs_received(&mut self) {}

    fn poll_seal(&mut self, _cx: &mut Context<'_>) -> bool {
        false
    }

    fn on_sealed(&mut self) {}
}

/// Seals a block after a timer expires. The timer starts when the first transaction is received.
#[derive(Debug)]
struct IntervalMode {
    block_time: Duration,
    timer: Option<Interval>,
    should_seal: bool,
}

impl IntervalMode {
    fn new(block_time_ms: u64) -> Self {
        Self { block_time: Duration::from_millis(block_time_ms), timer: None, should_seal: false }
    }
}

impl MiningMode for IntervalMode {
    fn on_txs_received(&mut self) {
        if self.timer.is_none() {
            let mut interval = interval_at(Instant::now() + self.block_time, self.block_time);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            self.timer = Some(interval);
        }
    }

    fn poll_seal(&mut self, cx: &mut Context<'_>) -> bool {
        if self.should_seal {
            return true;
        }

        if let Some(timer) = self.timer.as_mut() {
            if timer.poll_tick(cx).is_ready() {
                self.should_seal = true;
                return true;
            }
        }

        false
    }

    fn on_sealed(&mut self) {
        self.timer = None;
        self.should_seal = false;
    }
}

// --- Block producer ---

#[derive(Debug)]
struct TxExecutionOutcome {
    leftovers: Vec<ExecutableTxWithHash>,
    block_full: bool,
}

enum ProducerCommand {
    Queue(Vec<ExecutableTxWithHash>),
    ForceMine,
}

enum ProducerState {
    /// No block is open. Waiting for transactions or a force-mine command.
    Idle,
    /// A block is open with executed transactions. Waiting for more transactions or a seal trigger.
    Open,
    /// A batch of transactions is being executed on the blocking thread pool.
    Executing(TxExecutionFuture),
    /// The block is being sealed (committed to storage) on the blocking thread pool.
    Sealing(BlockProductionFuture),
}

#[allow(missing_debug_implementations)]
struct BlockProducerInner<PF>
where
    PF: ProviderFactory,
{
    backend: Arc<Backend<PF>>,
    mining_mode: Box<dyn MiningMode>,

    state: ProducerState,
    queued: VecDeque<ExecutableTxWithHash>,
    mailbox: VecDeque<ProducerCommand>,

    /// Set when a force mine command is received.
    force_seal: bool,
    /// Set when execution reports the block is full.
    block_full: bool,

    pool: Option<TxPool>,
    pool_pending_txs: Option<PoolPendingTransactions>,

    pending_executor: PendingExecutor,
    validator: TxValidator,
    permit: Arc<Mutex<()>>,

    blocking_task_spawner: CpuBlockingTaskPool,
    waker: Option<Waker>,
}

impl<PF> BlockProducerInner<PF>
where
    PF: ProviderFactory,
    <PF as ProviderFactory>::Provider: ProviderRO,
    <PF as ProviderFactory>::ProviderMut: ProviderRW,
{
    fn new(
        backend: Arc<Backend<PF>>,
        mining_mode: Box<dyn MiningMode>,
    ) -> Result<Self, BlockProductionError> {
        let permit = Arc::new(Mutex::new(()));

        let pending_executor = Self::create_executor_for_next_block(backend.as_ref())?;
        let validator = Self::create_validator(backend.as_ref(), &pending_executor, permit.clone());

        let blocking_task_spawner = CpuBlockingTaskPool::builder()
            .thread_name(|i| format!("block-producer-blocking-pool-{i}"))
            .build()
            .expect("failed to build task pool");

        Ok(Self {
            mining_mode,
            permit,
            validator,
            backend,
            pending_executor,
            blocking_task_spawner,
            state: ProducerState::Idle,
            queued: VecDeque::default(),
            mailbox: VecDeque::default(),
            force_seal: false,
            block_full: false,
            pool: None,
            pool_pending_txs: None,
            waker: None,
        })
    }

    fn create_executor_for_next_block(
        backend: &Backend<PF>,
    ) -> Result<PendingExecutor, BlockProductionError> {
        let provider = backend.storage.provider();

        let latest_num = provider.latest_number()?;
        let mut block_env = provider.block_env_at(latest_num.into())?.ok_or_else(|| {
            BlockProductionError::InconsistentState(format!(
                "missing block env for latest block {latest_num}"
            ))
        })?;
        backend.update_block_env(&mut block_env);

        let state = provider.latest()?;
        let executor = backend.executor_factory.executor(state, block_env);

        Ok(PendingExecutor::new(executor))
    }

    fn create_validator(
        backend: &Backend<PF>,
        pending_executor: &PendingExecutor,
        permit: Arc<Mutex<()>>,
    ) -> TxValidator {
        let cfg = backend.executor_factory.overrides();
        let flags = backend.executor_factory.execution_flags();

        let lock = pending_executor.read();
        let state = lock.state();
        let block_env = lock.block_env();

        TxValidator::new(
            state,
            flags.clone(),
            cfg.cloned(),
            block_env,
            permit,
            backend.chain_spec.clone(),
        )
    }

    fn update_validator(&self) {
        let lock = self.pending_executor.read();
        self.validator.update(lock.state(), lock.block_env());
    }

    fn wake(&self) {
        if let Some(waker) = self.waker.as_ref() {
            waker.wake_by_ref();
        }
    }

    fn enqueue(&mut self, txs: Vec<ExecutableTxWithHash>) {
        if txs.is_empty() {
            return;
        }

        self.mailbox.push_back(ProducerCommand::Queue(txs));
        self.wake();
    }

    fn attach_pool(&mut self, pool: TxPool) {
        self.pool_pending_txs = Some(pool.pending_transactions());
        self.pool = Some(pool);
        self.wake();
    }

    fn request_force_mine(&mut self) {
        self.mailbox.push_back(ProducerCommand::ForceMine);
        self.wake();
    }

    fn drain_mailbox(&mut self) {
        while let Some(command) = self.mailbox.pop_front() {
            match command {
                ProducerCommand::Queue(txs) => {
                    self.queued.extend(txs);
                    self.mining_mode.on_txs_received();
                }

                ProducerCommand::ForceMine => {
                    trace!(target: LOG_TARGET, "Scheduling force block mining.");
                    self.force_seal = true;
                }
            }
        }
    }

    fn drain_pool_transactions(&mut self, cx: &mut Context<'_>) {
        let Some(pending_txs) = self.pool_pending_txs.as_mut() else {
            return;
        };

        let mut collected = Vec::new();
        while let Poll::Ready(Some(tx)) = pending_txs.poll_next_unpin(cx) {
            collected.push(tx.tx.as_ref().clone());
        }

        if !collected.is_empty() {
            self.queued.extend(collected);
            self.mining_mode.on_txs_received();
        }
    }

    fn remove_mined_transactions(&self, txs: &[TxHash]) {
        if let Some(pool) = self.pool.as_ref() {
            pool.remove_transactions(txs);
        }
    }

    fn start_execution(&mut self) {
        let transactions = self.queued.drain(..).collect::<Vec<_>>();
        let executor = self.pending_executor.clone();

        let fut =
            self.blocking_task_spawner.spawn(|| Self::execute_transactions(executor, transactions));

        self.state = ProducerState::Executing(Box::pin(fut));
    }

    fn start_sealing(&mut self) {
        self.force_seal = false;
        self.block_full = false;

        let executor = self.pending_executor.clone();
        let permit = self.permit.clone();
        let backend = self.backend.clone();

        let fut = self.blocking_task_spawner.spawn(|| Self::seal_block(permit, executor, backend));

        self.state = ProducerState::Sealing(Box::pin(fut));
    }

    fn on_execution_finished(&mut self, outcome: TxExecutionOutcome) {
        if !outcome.leftovers.is_empty() {
            for tx in outcome.leftovers.into_iter().rev() {
                self.queued.push_front(tx);
            }
        }

        if outcome.block_full {
            info!(target: LOG_TARGET, "Block has reached capacity.");
            self.block_full = true;
        }
    }

    fn on_sealed(&mut self) -> Result<(), BlockProductionError> {
        self.pending_executor = Self::create_executor_for_next_block(self.backend.as_ref())?;
        self.update_validator();
        self.mining_mode.on_sealed();
        Ok(())
    }

    fn execute_transactions(
        executor: PendingExecutor,
        mut transactions: Vec<ExecutableTxWithHash>,
    ) -> Result<TxExecutionOutcome, BlockProductionError> {
        let executor = &mut executor.write();

        let (total_executed, limit_error) = executor.execute_transactions(transactions.clone())?;
        let leftovers =
            if limit_error.is_some() { transactions.split_off(total_executed) } else { Vec::new() };

        Ok(TxExecutionOutcome { leftovers, block_full: limit_error.is_some() })
    }

    fn seal_block(
        permit: Arc<Mutex<()>>,
        executor: PendingExecutor,
        backend: Arc<Backend<PF>>,
    ) -> Result<MinedBlockOutcome, BlockProductionError> {
        let _permit = permit.lock();
        let executor = &mut executor.write();

        trace!(target: LOG_TARGET, "Creating new block.");

        let block_env = executor.block_env();
        let execution_output = executor.take_execution_output()?;
        let outcome = backend.do_mine_block(&block_env, execution_output)?;

        trace!(target: LOG_TARGET, block_number = %outcome.block_number, "Created new block.");

        Ok(outcome)
    }

    fn poll_next(&mut self, cx: &mut Context<'_>) -> Poll<Option<BlockProductionResult>> {
        self.waker = Some(cx.waker().clone());

        loop {
            self.drain_mailbox();
            self.drain_pool_transactions(cx);

            let should_seal =
                self.force_seal || self.block_full || self.mining_mode.poll_seal(cx);

            match std::mem::replace(&mut self.state, ProducerState::Idle) {
                // Idle: no block is open.
                //   - should_seal with empty queue → seal an empty block (force mine)
                //   - queued txs → start executing (opens a new block)
                //   - otherwise → park
                ProducerState::Idle => {
                    if should_seal && self.queued.is_empty() {
                        self.start_sealing();
                        continue;
                    }

                    if !self.queued.is_empty() {
                        self.start_execution();
                        continue;
                    }

                    return Poll::Pending;
                }

                // Open: block is open with executed transactions.
                //   - should_seal → seal the block
                //   - more queued txs → execute them into the current block
                //   - otherwise → park (wait for more txs or seal trigger)
                ProducerState::Open => {
                    if should_seal {
                        self.start_sealing();
                        continue;
                    }

                    if !self.queued.is_empty() {
                        self.start_execution();
                        continue;
                    }

                    self.state = ProducerState::Open;
                    return Poll::Pending;
                }

                // Executing: waiting for tx execution to complete.
                //   - future ready → transition to Open, continue loop
                //   - future pending → park
                ProducerState::Executing(mut execution) => match execution.poll_unpin(cx) {
                    Poll::Ready(res) => {
                        self.state = ProducerState::Open;

                        match res {
                            TaskResult::Ok(Ok(outcome)) => {
                                self.on_execution_finished(outcome);
                                continue;
                            }

                            TaskResult::Ok(Err(err)) => {
                                return Poll::Ready(Some(Err(err)));
                            }

                            TaskResult::Err(err) => {
                                if err.is_cancelled() {
                                    return Poll::Ready(Some(Err(
                                        BlockProductionError::ExecutionTaskCancelled,
                                    )));
                                }

                                std::panic::resume_unwind(err.into_panic());
                            }
                        }
                    }

                    Poll::Pending => {
                        self.state = ProducerState::Executing(execution);
                        return Poll::Pending;
                    }
                },

                // Sealing: waiting for block seal to complete.
                //   - future ready → transition to Idle, yield outcome
                //   - future pending → park
                ProducerState::Sealing(mut sealing) => match sealing.poll_unpin(cx) {
                    Poll::Ready(res) => {
                        // state is already Idle from the mem::replace

                        match res {
                            TaskResult::Ok(Ok(outcome)) => {
                                self.remove_mined_transactions(&outcome.txs);

                                if let Err(err) = self.on_sealed() {
                                    return Poll::Ready(Some(Err(err)));
                                }

                                info!(
                                    target: LOG_TARGET,
                                    block_number = %outcome.block_number,
                                    "Mined block."
                                );

                                return Poll::Ready(Some(Ok(outcome)));
                            }

                            TaskResult::Ok(Err(err)) => {
                                return Poll::Ready(Some(Err(err)));
                            }

                            TaskResult::Err(err) => {
                                if err.is_cancelled() {
                                    return Poll::Ready(Some(Err(
                                        BlockProductionError::ExecutionTaskCancelled,
                                    )));
                                }

                                std::panic::resume_unwind(err.into_panic());
                            }
                        }
                    }

                    Poll::Pending => {
                        self.state = ProducerState::Sealing(sealing);
                        return Poll::Pending;
                    }
                },
            }
        }
    }
}

/// The type responsible for block production.
#[must_use = "BlockProducer does nothing unless polled"]
pub struct BlockProducer<PF>
where
    PF: ProviderFactory,
{
    inner: Arc<RwLock<BlockProducerInner<PF>>>,
}

impl<PF> BlockProducer<PF>
where
    PF: ProviderFactory,
    <PF as ProviderFactory>::Provider: ProviderRO,
    <PF as ProviderFactory>::ProviderMut: ProviderRW,
{
    /// Creates a block producer that mines a new block every `interval` milliseconds.
    pub fn interval(backend: Arc<Backend<PF>>, interval: u64) -> Self {
        let inner = BlockProducerInner::new(backend, Box::new(IntervalMode::new(interval)))
            .expect("failed to create interval block producer");
        Self { inner: Arc::new(RwLock::new(inner)) }
    }

    /// Creates a new block producer that only mines blocks via `katana_generateBlock`.
    pub fn on_demand(backend: Arc<Backend<PF>>) -> Self {
        let inner = BlockProducerInner::new(backend, Box::new(OnDemandMode))
            .expect("failed to create on-demand block producer");
        Self { inner: Arc::new(RwLock::new(inner)) }
    }

    /// Creates a block producer that mines a new block as soon as there are ready transactions.
    pub fn instant(backend: Arc<Backend<PF>>) -> Self {
        let inner = BlockProducerInner::new(backend, Box::new(InstantMode::default()))
            .expect("failed to create instant block producer");
        Self { inner: Arc::new(RwLock::new(inner)) }
    }

    pub(super) fn queue(&self, transactions: Vec<ExecutableTxWithHash>) {
        self.inner.write().enqueue(transactions);
    }

    pub(super) fn attach_pool(&self, pool: TxPool) {
        self.inner.write().attach_pool(pool);
    }

    pub fn validator(&self) -> TxValidator {
        self.inner.read().validator.clone()
    }

    /// Returns the current pending executor state.
    pub fn pending_executor(&self) -> Option<PendingExecutor> {
        Some(self.inner.read().pending_executor.clone())
    }

    /// Returns the current pending state.
    pub fn pending_state(&self) -> Option<Box<dyn StateProvider>> {
        self.pending_executor().map(|executor| executor.read().state())
    }

    /// Returns true if there are pending transactions in the current block.
    pub fn has_pending_transactions(&self) -> bool {
        self.pending_executor()
            .map(|executor| !executor.read().transactions().is_empty())
            .unwrap_or(false)
    }

    /// Handler for the `katana_generateBlock` RPC method.
    pub fn force_mine(&self) {
        self.inner.write().request_force_mine();
    }

    pub(super) fn poll_next(&self, cx: &mut Context<'_>) -> Poll<Option<BlockProductionResult>> {
        self.inner.write().poll_next(cx)
    }
}

impl<PF> Clone for BlockProducer<PF>
where
    PF: ProviderFactory,
{
    fn clone(&self) -> Self {
        Self { inner: self.inner.clone() }
    }
}

impl<PF> std::fmt::Debug for BlockProducer<PF>
where
    PF: ProviderFactory,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BlockProducer").field("inner", &"..").finish()
    }
}
