//! Server implementation for the Starknet JSON-RPC API.

use std::fmt::Debug;
use std::future::Future;
use std::sync::Arc;

use katana_chain_spec::ChainSpec;
use katana_core::utils::get_current_timestamp;
use katana_executor::{ExecutionResult, ResultAndStates};
use katana_gas_price_oracle::GasPriceOracle;
use katana_pool::TransactionPool;
use katana_primitives::block::{BlockHashOrNumber, BlockIdOrTag, FinalityStatus, GasPrices};
use katana_primitives::class::{ClassHash, CompiledClass};
use katana_primitives::contract::{ContractAddress, Nonce, StorageKey, StorageValue};
use katana_primitives::env::BlockEnv;
use katana_primitives::event::MaybeForkedContinuationToken;
use katana_primitives::execution::TypedTransactionExecutionInfo;
use katana_primitives::transaction::{ExecutableTx, ExecutableTxWithHash, TxHash, TxNumber};
use katana_primitives::Felt;
use katana_provider::api::block::{
    BlockHashProvider, BlockIdReader, BlockNumberProvider, BlockProvider,
};
use katana_provider::api::contract::ContractClassProvider;
use katana_provider::api::env::BlockEnvProvider;
use katana_provider::api::state::{StateFactoryProvider, StateProvider, StateRootProvider};
use katana_provider::api::transaction::{
    ReceiptProvider, TransactionProvider, TransactionStatusProvider, TransactionTraceProvider,
    TransactionsProviderExt,
};
use katana_provider::api::ProviderError;
use katana_provider::{ProviderFactory, ProviderRO};
use katana_rpc_api::error::starknet::{
    CompilationErrorData, PageSizeTooBigData, ProofLimitExceededData, StarknetApiError,
};
use katana_rpc_types::block::{
    BlockHashAndNumberResponse, BlockNumberResponse, GetBlockWithReceiptsResponse,
    GetBlockWithTxHashesResponse, MaybePreConfirmedBlock,
};
use katana_rpc_types::class::Class;
use katana_rpc_types::event::{EventFilterWithPage, GetEventsResponse, ResultPageRequest};
use katana_rpc_types::list::{
    ContinuationToken as ListContinuationToken, GetBlocksRequest, GetBlocksResponse,
    GetTransactionsRequest, GetTransactionsResponse, TransactionListItem,
};
use katana_rpc_types::receipt::TxReceiptWithBlockInfo;
use katana_rpc_types::state_update::StateUpdate;
use katana_rpc_types::transaction::RpcTxWithHash;
use katana_rpc_types::trie::{
    ClassesProof, ContractLeafData, ContractStorageKeys, ContractStorageProofs, ContractsProof,
    GetStorageProofResponse, GlobalRoots, Nodes,
};
use katana_rpc_types::{
    to_rpc_fee_estimate, BroadcastedTx, BroadcastedTxWithChainId, CallResponse,
    ConfirmedBlockIdOrTag, FeeEstimate, FunctionCall, SimulatedTransactions, SimulationFlag,
    TxStatus, TxTrace, TxTraceWithHash,
};
use katana_rpc_types_builder::{BlockBuilder, ReceiptBuilder};
use katana_tasks::{Result as TaskResult, TaskSpawner};

use crate::permit::Permits;
use crate::utils::events::{Cursor, EventBlockId};
use crate::{utils, DEFAULT_ESTIMATE_FEE_MAX_CONCURRENT_REQUESTS};

mod blockifier;
mod config;
mod list;
mod pending;
mod read;
mod trace;
mod write;

#[cfg(feature = "cartridge")]
pub use config::CartridgePaymasterConfig;
pub use config::StarknetApiConfig;
pub use pending::PendingBlockProvider;

pub type StarknetApiResult<T> = Result<T, StarknetApiError>;

/// Handler for the Starknet JSON-RPC server.
///
/// This struct implements all the JSON-RPC traits required to serve the Starknet API (ie,
/// [read](katana_rpc_api::starknet::StarknetApi),
/// [write](katana_rpc_api::starknet::StarknetWriteApi), and
/// [trace](katana_rpc_api::starknet::StarknetTraceApi) APIs.
#[derive(Debug)]
pub struct StarknetApi<Pool, PP, PF>
where
    Pool: TransactionPool,
    PP: PendingBlockProvider,
    PF: ProviderFactory,
{
    inner: Arc<StarknetApiInner<Pool, PP, PF>>,
}

#[derive(Debug)]
struct StarknetApiInner<Pool, PP, PF>
where
    Pool: TransactionPool,
    PP: PendingBlockProvider,
    PF: ProviderFactory,
{
    pool: Pool,
    chain_spec: Arc<ChainSpec>,
    gas_oracle: GasPriceOracle,
    storage: PF,
    task_spawner: TaskSpawner,
    estimate_fee_permit: Permits,
    pending_block_provider: PP,
    config: StarknetApiConfig,
}

impl<Pool, PP, PF> StarknetApi<Pool, PP, PF>
where
    Pool: TransactionPool,
    PP: PendingBlockProvider,
    PF: ProviderFactory,
{
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        chain_spec: Arc<ChainSpec>,
        pool: Pool,
        task_spawner: TaskSpawner,
        pending_block_provider: PP,
        gas_oracle: GasPriceOracle,
        config: StarknetApiConfig,
        storage2: PF,
    ) -> Self {
        Self::new_inner(
            chain_spec,
            pool,
            task_spawner,
            config,
            pending_block_provider,
            gas_oracle,
            storage2,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn new_inner(
        chain_spec: Arc<ChainSpec>,
        pool: Pool,
        task_spawner: TaskSpawner,
        config: StarknetApiConfig,
        pending_block_provider: PP,
        gas_oracle: GasPriceOracle,
        storage2: PF,
    ) -> Self {
        let total_permits = config
            .max_concurrent_estimate_fee_requests
            .unwrap_or(DEFAULT_ESTIMATE_FEE_MAX_CONCURRENT_REQUESTS);
        let estimate_fee_permit = Permits::new(total_permits);

        let inner = StarknetApiInner {
            chain_spec,
            pool,
            task_spawner,
            estimate_fee_permit,
            config,
            pending_block_provider,
            gas_oracle,
            storage: storage2,
        };

        Self { inner: Arc::new(inner) }
    }

    pub fn pool(&self) -> &Pool {
        &self.inner.pool
    }

    pub fn storage(&self) -> &PF {
        &self.inner.storage
    }

    pub fn estimate_fee_permit(&self) -> &Permits {
        &self.inner.estimate_fee_permit
    }

    pub fn config(&self) -> &StarknetApiConfig {
        &self.inner.config
    }

    /// Returns the chain ID.
    pub fn chain_id(&self) -> Felt {
        self.inner.chain_spec.id().id()
    }
}

impl<Pool, PP, PF> StarknetApi<Pool, PP, PF>
where
    Pool: TransactionPool + 'static,
    PP: PendingBlockProvider,
    PF: ProviderFactory,
{
    /// Spawns an async function that is mostly CPU-bound blocking task onto the manager's blocking
    /// pool.
    async fn on_cpu_blocking_task<T, F>(&self, func: T) -> StarknetApiResult<F::Output>
    where
        T: FnOnce(Self) -> F,
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        use tokio::runtime::Builder;

        let this = self.clone();
        let future = func(this);
        let span = tracing::Span::current();

        let task = move || {
            let _enter = span.enter();
            Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("failed to build tokio runtime")
                .block_on(future)
        };

        match self.inner.task_spawner.cpu_bound().spawn(task).await {
            TaskResult::Ok(result) => Ok(result),
            TaskResult::Err(err) => {
                Err(StarknetApiError::unexpected(format!("internal task execution failed: {err}")))
            }
        }
    }

    pub async fn on_io_blocking_task<F, R>(&self, func: F) -> StarknetApiResult<R>
    where
        F: FnOnce(Self) -> R + Send + 'static,
        R: Send + 'static,
    {
        let this = self.clone();
        let span = tracing::Span::current();
        match self
            .inner
            .task_spawner
            .spawn_blocking(move || {
                let _enter = span.enter();
                func(this)
            })
            .await
        {
            TaskResult::Ok(result) => Ok(result),
            TaskResult::Err(err) => {
                Err(StarknetApiError::unexpected(format!("internal task execution failed: {err}")))
            }
        }
    }
}

impl<Pool, PP, PF> StarknetApi<Pool, PP, PF>
where
    Pool: TransactionPool + 'static,
    PP: PendingBlockProvider,
    PF: ProviderFactory,
    <PF as ProviderFactory>::Provider: ProviderRO,
{
    fn estimate_fee_with(
        &self,
        transactions: Vec<ExecutableTxWithHash>,
        block_id: BlockIdOrTag,
        flags: katana_executor::ExecutionFlags,
    ) -> StarknetApiResult<Vec<FeeEstimate>> {
        // get the state and block env at the specified block for execution
        let state = self.state(&block_id)?;
        let env = self.block_env_at(&block_id)?;
        let versioned_constant_overrides = self.inner.config.versioned_constant_overrides.as_ref();

        // do estimations
        blockifier::estimate_fees(
            self.inner.chain_spec.as_ref(),
            state,
            env,
            versioned_constant_overrides,
            transactions,
            flags,
        )
    }

    pub fn state(&self, block_id: &BlockIdOrTag) -> StarknetApiResult<Box<dyn StateProvider>> {
        let provider = self.storage().provider();

        let state = match block_id {
            BlockIdOrTag::PreConfirmed => {
                if let pending_state @ Some(..) =
                    self.inner.pending_block_provider.pending_state()?
                {
                    pending_state
                } else {
                    Some(provider.latest()?)
                }
            }

            BlockIdOrTag::L1Accepted => None,
            BlockIdOrTag::Latest => Some(provider.latest()?),
            BlockIdOrTag::Hash(hash) => provider.historical((*hash).into())?,
            BlockIdOrTag::Number(num) => provider.historical((*num).into())?,
        };

        state.ok_or(StarknetApiError::BlockNotFound)
    }

    fn block_env_at(&self, block_id: &BlockIdOrTag) -> StarknetApiResult<BlockEnv> {
        let provider = self.storage().provider();

        let env = match block_id {
            BlockIdOrTag::PreConfirmed => {
                if let Some(block) =
                    self.inner.pending_block_provider.get_pending_block_with_txs()?
                {
                    Some(BlockEnv {
                        number: block.block_number,
                        timestamp: block.timestamp,
                        l1_gas_prices: GasPrices {
                            eth: block.l1_gas_price.price_in_wei.try_into().unwrap(),
                            strk: block.l1_gas_price.price_in_fri.try_into().unwrap(),
                        },
                        l2_gas_prices: GasPrices {
                            eth: block.l2_gas_price.price_in_wei.try_into().unwrap(),
                            strk: block.l2_gas_price.price_in_fri.try_into().unwrap(),
                        },
                        l1_data_gas_prices: GasPrices {
                            eth: block.l1_data_gas_price.price_in_wei.try_into().unwrap(),
                            strk: block.l1_data_gas_price.price_in_fri.try_into().unwrap(),
                        },
                        starknet_version: block.starknet_version.try_into().unwrap(),
                        sequencer_address: block.sequencer_address,
                    })
                }
                // else, we create a new block env and update the values to reflect the current
                // state.
                else {
                    let num = provider.latest_number()?;
                    let mut env = provider.block_env_at(num.into())?.expect("missing block env");

                    env.number += 1;
                    env.timestamp = get_current_timestamp().as_secs() as u64;
                    env.l2_gas_prices = self.inner.gas_oracle.l2_gas_prices();
                    env.l1_gas_prices = self.inner.gas_oracle.l1_gas_prices();
                    env.l1_data_gas_prices = self.inner.gas_oracle.l1_data_gas_prices();

                    Some(env)
                }
            }

            BlockIdOrTag::L1Accepted => None,
            BlockIdOrTag::Latest => provider.block_env_at(provider.latest_number()?.into())?,
            BlockIdOrTag::Hash(hash) => provider.block_env_at((*hash).into())?,
            BlockIdOrTag::Number(num) => provider.block_env_at((*num).into())?,
        };

        env.ok_or(StarknetApiError::BlockNotFound)
    }

    pub async fn block_hash_and_number(&self) -> StarknetApiResult<BlockHashAndNumberResponse> {
        self.on_io_blocking_task(move |this| {
            let provider = this.storage().provider();
            let hash = provider.latest_hash()?;
            let number = provider.latest_number()?;
            Ok(BlockHashAndNumberResponse::new(hash, number))
        })
        .await?
    }

    pub async fn class_at_hash(
        &self,
        block_id: BlockIdOrTag,
        class_hash: ClassHash,
    ) -> StarknetApiResult<Class> {
        self.on_io_blocking_task(move |this| {
            let state = this.state(&block_id)?;

            let Some(class) = state.class(class_hash)? else {
                return Err(StarknetApiError::ClassHashNotFound);
            };

            Ok(Class::try_from(class).unwrap())
        })
        .await?
    }

    pub async fn class_hash_at_address(
        &self,
        block_id: BlockIdOrTag,
        contract_address: ContractAddress,
    ) -> StarknetApiResult<ClassHash> {
        self.on_io_blocking_task(move |this| {
            // Contract address 0x1 and 0x2 are special system contracts and does not
            // have a class.
            //
            // See https://docs.starknet.io/architecture-and-concepts/network-architecture/starknet-state/#address_0x1.
            if contract_address == ContractAddress::ONE || contract_address == ContractAddress::TWO
            {
                return Ok(ClassHash::ZERO);
            }

            let state = this.state(&block_id)?;
            let class_hash = state.class_hash_of_contract(contract_address)?;
            class_hash.ok_or(StarknetApiError::ContractNotFound)
        })
        .await?
    }

    pub async fn class_at_address(
        &self,
        block_id: BlockIdOrTag,
        contract_address: ContractAddress,
    ) -> StarknetApiResult<Class> {
        let hash = self.class_hash_at_address(block_id, contract_address).await?;
        let class = self.class_at_hash(block_id, hash).await?;
        Ok(class)
    }

    pub async fn compiled_class_at_hash(
        &self,
        class_hash: ClassHash,
    ) -> StarknetApiResult<CompiledClass> {
        let class = self
            .on_io_blocking_task(move |this| {
                let state = this.state(&BlockIdOrTag::Latest)?;
                state.class(class_hash)?.ok_or(StarknetApiError::ClassHashNotFound)
            })
            .await??;

        self.on_cpu_blocking_task(move |_| async move {
            class.compile().map_err(|e| {
                StarknetApiError::CompilationError(CompilationErrorData {
                    compilation_error: e.to_string(),
                })
            })
        })
        .await?
    }

    pub async fn call_contract(
        &self,
        request: FunctionCall,
        block_id: BlockIdOrTag,
    ) -> StarknetApiResult<CallResponse> {
        self.on_io_blocking_task(move |this| {
            // get the state and block env at the specified block for function call execution
            let state = this.state(&block_id)?;
            let env = this.block_env_at(&block_id)?;
            let cfg_env = this.inner.config.versioned_constant_overrides.as_ref();
            let max_call_gas = this.inner.config.max_call_gas.unwrap_or(1_000_000_000);

            let result = self::blockifier::call(
                this.inner.chain_spec.as_ref(),
                state,
                env,
                cfg_env,
                request,
                max_call_gas,
            )?;

            Ok(CallResponse { result })
        })
        .await?
    }

    pub async fn storage_at(
        &self,
        contract_address: ContractAddress,
        storage_key: StorageKey,
        block_id: BlockIdOrTag,
    ) -> StarknetApiResult<StorageValue> {
        self.on_io_blocking_task(move |this| {
            let state = this.state(&block_id)?;

            // Check that contract exist by checking the class hash of the contract,
            // unless its address 0x1 or 0x2 which are special system contracts and does not
            // have a class.
            //
            // See https://docs.starknet.io/architecture-and-concepts/network-architecture/starknet-state/#address_0x1.
            if contract_address != ContractAddress::ONE
                && contract_address != ContractAddress::TWO
                && state.class_hash_of_contract(contract_address)?.is_none()
            {
                return Err(StarknetApiError::ContractNotFound);
            }

            let value = state.storage(contract_address, storage_key)?;
            Ok(value.unwrap_or_default())
        })
        .await?
    }

    pub async fn block_tx_count(&self, block_id: BlockIdOrTag) -> StarknetApiResult<u64> {
        let count = self
            .on_io_blocking_task(move |this| {
                let provider = this.storage().provider();

                let block_id: BlockHashOrNumber = match block_id {
                    BlockIdOrTag::L1Accepted => return Ok(None),

                    BlockIdOrTag::PreConfirmed => {
                        if let Some(block) =
                            this.inner.pending_block_provider.get_pending_block_with_tx_hashes()?
                        {
                            return Ok(Some(block.transactions.len() as u64));
                        } else {
                            return Ok(None);
                        }
                    }
                    BlockIdOrTag::Latest => provider.latest_number()?.into(),
                    BlockIdOrTag::Number(num) => num.into(),
                    BlockIdOrTag::Hash(hash) => hash.into(),
                };

                let count = provider.transaction_count_by_block(block_id)?;
                Result::<_, StarknetApiError>::Ok(count)
            })
            .await??;

        if let Some(count) = count {
            Ok(count)
        } else {
            Err(StarknetApiError::BlockNotFound)
        }
    }

    pub async fn latest_block_number(&self) -> StarknetApiResult<BlockNumberResponse> {
        self.on_io_blocking_task(move |this| {
            let block_number = this.storage().provider().latest_number()?;
            Ok(BlockNumberResponse { block_number })
        })
        .await?
    }

    pub async fn nonce_at(
        &self,
        block_id: BlockIdOrTag,
        contract_address: ContractAddress,
    ) -> StarknetApiResult<Nonce> {
        self.on_io_blocking_task(move |this| {
            let pending_nonce = if matches!(block_id, BlockIdOrTag::PreConfirmed) {
                this.inner.pool.get_nonce(contract_address)
            } else {
                None
            };

            match pending_nonce {
                Some(pending_nonce) => Ok(pending_nonce),
                None => {
                    let state = this.state(&block_id)?;
                    state.nonce(contract_address)?.ok_or(StarknetApiError::ContractNotFound)
                }
            }
        })
        .await?
    }

    pub async fn transaction_by_block_id_and_index(
        &self,
        block_id: BlockIdOrTag,
        index: u64,
    ) -> StarknetApiResult<RpcTxWithHash> {
        let tx = self
            .on_io_blocking_task(move |this| {
                // TEMP: have to handle pending tag independently for now
                let tx = if BlockIdOrTag::PreConfirmed == block_id {
                    this.inner.pending_block_provider.get_pending_transaction_by_index(index)?
                } else {
                    let provider = this.storage().provider();

                    let block_num = provider
                        .convert_block_id(block_id)?
                        .map(BlockHashOrNumber::Num)
                        .ok_or(StarknetApiError::BlockNotFound)?;

                    provider
                        .transaction_by_block_and_idx(block_num, index)?
                        .map(RpcTxWithHash::from)
                };

                StarknetApiResult::Ok(tx)
            })
            .await??;

        if let Some(tx) = tx {
            Ok(tx)
        } else {
            Err(StarknetApiError::InvalidTxnIndex)
        }
    }

    pub async fn transaction(&self, hash: TxHash) -> StarknetApiResult<RpcTxWithHash> {
        let tx = self
            .on_io_blocking_task(move |this| {
                if let pending_tx @ Some(..) =
                    this.inner.pending_block_provider.get_pending_transaction(hash)?
                {
                    Result::<_, StarknetApiError>::Ok(pending_tx)
                } else {
                    let tx = this
                        .storage()
                        .provider()
                        .transaction_by_hash(hash)?
                        .map(RpcTxWithHash::from);

                    Result::<_, StarknetApiError>::Ok(tx)
                }
            })
            .await??;

        if let Some(tx) = tx {
            Ok(tx)
        } else {
            Err(StarknetApiError::TxnHashNotFound)
        }
    }

    pub async fn receipt(&self, hash: Felt) -> StarknetApiResult<TxReceiptWithBlockInfo> {
        let receipt = self
            .on_io_blocking_task(move |this| {
                if let pending_receipt @ Some(..) =
                    this.inner.pending_block_provider.get_pending_receipt(hash)?
                {
                    StarknetApiResult::Ok(pending_receipt)
                } else {
                    let provider = this.storage().provider();
                    StarknetApiResult::Ok(ReceiptBuilder::new(hash, provider).build()?)
                }
            })
            .await??;

        if let Some(receipt) = receipt {
            Ok(receipt)
        } else {
            Err(StarknetApiError::TxnHashNotFound)
        }
    }

    pub async fn transaction_status(&self, hash: TxHash) -> StarknetApiResult<TxStatus> {
        let status = self
            .on_io_blocking_task(move |this| {
                let provider = this.storage().provider();
                let status = provider.transaction_status(hash)?;

                if let Some(status) = status {
                    // TODO: this might not work once we allow querying for 'failed' transactions
                    // from the provider
                    let Some(receipt) = provider.receipt_by_hash(hash)? else {
                        let error = StarknetApiError::unexpected(
                            "Transaction hash exist, but the receipt is missing",
                        );
                        return Err(error);
                    };

                    let exec_status = if let Some(reason) = receipt.revert_reason() {
                        katana_rpc_types::ExecutionResult::Reverted { reason: reason.to_string() }
                    } else {
                        katana_rpc_types::ExecutionResult::Succeeded
                    };

                    let status = match status {
                        FinalityStatus::AcceptedOnL1 => TxStatus::AcceptedOnL1(exec_status),
                        FinalityStatus::AcceptedOnL2 => TxStatus::AcceptedOnL2(exec_status),
                        FinalityStatus::PreConfirmed => TxStatus::PreConfirmed(exec_status),
                    };

                    return Ok(Some(status));
                }

                // seach in the pending block if the transaction is not found
                if let Some(receipt) =
                    this.inner.pending_block_provider.get_pending_receipt(hash)?
                {
                    Ok(Some(TxStatus::PreConfirmed(receipt.receipt.execution_result().clone())))
                } else {
                    Ok(None)
                }
            })
            .await??;

        if let Some(status) = status {
            Ok(status)
        } else {
            let _ = self.inner.pool.get(hash).ok_or(StarknetApiError::TxnHashNotFound)?;
            Ok(TxStatus::Received)
        }
    }

    pub async fn block_with_txs(
        &self,
        block_id: BlockIdOrTag,
    ) -> StarknetApiResult<MaybePreConfirmedBlock> {
        let block = self
            .on_io_blocking_task(move |this| {
                let provider = this.storage().provider();

                if BlockIdOrTag::PreConfirmed == block_id {
                    if let Some(block) =
                        this.inner.pending_block_provider.get_pending_block_with_txs()?
                    {
                        return Ok(Some(MaybePreConfirmedBlock::PreConfirmed(block)));
                    }
                }

                if let Some(num) = provider.convert_block_id(block_id)? {
                    let block = katana_rpc_types_builder::BlockBuilder::new(num.into(), provider)
                        .build()?
                        .map(MaybePreConfirmedBlock::Confirmed);

                    StarknetApiResult::Ok(block)
                } else {
                    StarknetApiResult::Ok(None)
                }
            })
            .await??;

        if let Some(block) = block {
            Ok(block)
        } else {
            Err(StarknetApiError::BlockNotFound)
        }
    }

    pub async fn block_with_receipts(
        &self,
        block_id: BlockIdOrTag,
    ) -> StarknetApiResult<GetBlockWithReceiptsResponse> {
        let block = self
            .on_io_blocking_task(move |this| {
                let provider = this.storage().provider();

                if BlockIdOrTag::PreConfirmed == block_id {
                    if let Some(block) =
                        this.inner.pending_block_provider.get_pending_block_with_receipts()?
                    {
                        return Ok(Some(GetBlockWithReceiptsResponse::PreConfirmed(block)));
                    }
                }

                if let Some(num) = provider.convert_block_id(block_id)? {
                    let block = katana_rpc_types_builder::BlockBuilder::new(num.into(), provider)
                        .build_with_receipts()?
                        .map(GetBlockWithReceiptsResponse::Block);

                    StarknetApiResult::Ok(block)
                } else {
                    StarknetApiResult::Ok(None)
                }
            })
            .await??;

        if let Some(block) = block {
            Ok(block)
        } else {
            Err(StarknetApiError::BlockNotFound)
        }
    }

    pub async fn block_with_tx_hashes(
        &self,
        block_id: BlockIdOrTag,
    ) -> StarknetApiResult<GetBlockWithTxHashesResponse> {
        let block = self
            .on_io_blocking_task(move |this| {
                let provider = this.storage().provider();

                if BlockIdOrTag::PreConfirmed == block_id {
                    if let Some(block) =
                        this.inner.pending_block_provider.get_pending_block_with_tx_hashes()?
                    {
                        return Ok(Some(GetBlockWithTxHashesResponse::PreConfirmed(block)));
                    }
                }

                if let Some(num) = provider.convert_block_id(block_id)? {
                    let block = katana_rpc_types_builder::BlockBuilder::new(num.into(), provider)
                        .build_with_tx_hash()?
                        .map(GetBlockWithTxHashesResponse::Block);

                    StarknetApiResult::Ok(block)
                } else {
                    StarknetApiResult::Ok(None)
                }
            })
            .await??;

        if let Some(block) = block {
            Ok(block)
        } else {
            Err(StarknetApiError::BlockNotFound)
        }
    }

    pub async fn state_update(&self, block_id: BlockIdOrTag) -> StarknetApiResult<StateUpdate> {
        let state_update = self
            .on_io_blocking_task(move |this| {
                let provider = this.storage().provider();

                let block_id = match block_id {
                    BlockIdOrTag::Number(num) => BlockHashOrNumber::Num(num),
                    BlockIdOrTag::Hash(hash) => BlockHashOrNumber::Hash(hash),
                    BlockIdOrTag::Latest => provider.latest_number().map(BlockHashOrNumber::Num)?,

                    // TODO: Implement for L1 accepted and preconfirmed block id
                    BlockIdOrTag::L1Accepted => {
                        return Err(StarknetApiError::BlockNotFound);
                    }

                    BlockIdOrTag::PreConfirmed => {
                        if let Some(state_update) =
                            this.inner.pending_block_provider.get_pending_state_update()?
                        {
                            let state_update = StateUpdate::PreConfirmed(state_update);
                            return StarknetApiResult::Ok(Some(state_update));
                        } else {
                            return StarknetApiResult::Ok(None);
                        }
                    }
                };

                let state_update =
                    katana_rpc_types_builder::StateUpdateBuilder::new(block_id, provider)
                        .build()?
                        .map(StateUpdate::Confirmed);

                StarknetApiResult::Ok(state_update)
            })
            .await??;

        if let Some(state_update) = state_update {
            Ok(state_update)
        } else {
            Err(StarknetApiError::BlockNotFound)
        }
    }

    pub async fn events(
        &self,
        filter: EventFilterWithPage,
    ) -> StarknetApiResult<GetEventsResponse> {
        let EventFilterWithPage { event_filter, result_page_request } = filter;
        let ResultPageRequest { continuation_token, chunk_size } = result_page_request;

        if let Some(max_size) = self.inner.config.max_event_page_size {
            if chunk_size > max_size {
                return Err(StarknetApiError::PageSizeTooBig(PageSizeTooBigData {
                    requested: chunk_size,
                    max_allowed: max_size,
                }));
            }
        }

        self.on_io_blocking_task(move |this| {
            let from = match event_filter.from_block {
                Some(id) => id,
                None => BlockIdOrTag::Number(0),
            };

            let to = match event_filter.to_block {
                Some(id) => id,
                None => BlockIdOrTag::PreConfirmed,
            };

            let keys = event_filter.keys.filter(|keys| !(keys.len() == 1 && keys.is_empty()));
            let continuation_token = if let Some(token) = continuation_token {
                Some(MaybeForkedContinuationToken::parse(&token)?)
            } else {
                None
            };

            let events = this.events_inner(
                from,
                to,
                event_filter.address,
                keys,
                continuation_token,
                chunk_size,
            )?;

            Ok(events)
        })
        .await?
    }

    // TODO: should document more and possible find a simpler solution(?)
    fn events_inner(
        &self,
        from_block: BlockIdOrTag,
        to_block: BlockIdOrTag,
        address: Option<ContractAddress>,
        keys: Option<Vec<Vec<Felt>>>,
        continuation_token: Option<MaybeForkedContinuationToken>,
        chunk_size: u64,
    ) -> StarknetApiResult<GetEventsResponse> {
        let provider = self.storage().provider();

        let from = self.resolve_event_block_id_if_forked(from_block)?;
        let to = self.resolve_event_block_id_if_forked(to_block)?;

        // reserved buffer to fill up with events to avoid reallocations
        let mut events = Vec::with_capacity(chunk_size as usize);
        let filter = utils::events::Filter { address, keys: keys.clone() };

        match (from, to) {
            (EventBlockId::Num(from), EventBlockId::Num(to)) => {
                let from_after_forked_if_any = from;

                let cursor = continuation_token.and_then(|t| t.to_token().map(|t| t.into()));
                let block_range = from_after_forked_if_any..=to;

                let cursor = utils::events::fetch_events_at_blocks(
                    provider,
                    block_range,
                    &filter,
                    chunk_size,
                    cursor,
                    &mut events,
                )?;

                let continuation_token = cursor.map(|c| c.into_rpc_cursor().to_string());
                let events_page = GetEventsResponse { events, continuation_token };

                Ok(events_page)
            }

            (EventBlockId::Num(from), EventBlockId::Pending) => {
                let from_after_forked_if_any = from;

                let cursor = continuation_token.and_then(|t| t.to_token().map(|t| t.into()));
                let latest = provider.latest_number()?;
                let block_range = from_after_forked_if_any..=latest;

                let int_cursor = utils::events::fetch_events_at_blocks(
                    provider,
                    block_range,
                    &filter,
                    chunk_size,
                    cursor.clone(),
                    &mut events,
                )?;

                // if the internal cursor is Some, meaning the buffer is full and we havent
                // reached the latest block.
                if let Some(c) = int_cursor {
                    let continuation_token = Some(c.into_rpc_cursor().to_string());
                    return Ok(GetEventsResponse { events, continuation_token });
                }

                if let Some(block) =
                    self.inner.pending_block_provider.get_pending_block_with_receipts()?
                {
                    let cursor = utils::events::fetch_pending_events(
                        &block,
                        &filter,
                        chunk_size,
                        cursor,
                        &mut events,
                    )?;

                    let continuation_token = Some(cursor.into_rpc_cursor().to_string());
                    Ok(GetEventsResponse { events, continuation_token })
                } else {
                    let cursor = Cursor::new_block(latest + 1);
                    let continuation_token = Some(cursor.into_rpc_cursor().to_string());
                    Ok(GetEventsResponse { events, continuation_token })
                }
            }

            (EventBlockId::Pending, EventBlockId::Pending) => {
                if let Some(block) =
                    self.inner.pending_block_provider.get_pending_block_with_receipts()?
                {
                    let cursor = continuation_token.and_then(|t| t.to_token().map(|t| t.into()));
                    let new_cursor = utils::events::fetch_pending_events(
                        &block,
                        &filter,
                        chunk_size,
                        cursor,
                        &mut events,
                    )?;

                    let continuation_token = Some(new_cursor.into_rpc_cursor().to_string());
                    Ok(GetEventsResponse { events, continuation_token })
                } else {
                    let latest = provider.latest_number()?;
                    let new_cursor = Cursor::new_block(latest);

                    let continuation_token = Some(new_cursor.into_rpc_cursor().to_string());
                    Ok(GetEventsResponse { events, continuation_token })
                }
            }

            (EventBlockId::Pending, EventBlockId::Num(_)) => Err(StarknetApiError::unexpected(
                "Invalid block range; `from` block must be lower than `to`",
            )),
        }
    }

    // Determine the block number based on its Id. In the case where the block id is a hash, we need
    // to check if the block is in the forked client AND within the valid range (ie lower than
    // forked block).
    fn resolve_event_block_id_if_forked(
        &self,
        id: BlockIdOrTag,
    ) -> StarknetApiResult<EventBlockId> {
        let provider = &self.storage().provider();

        let id = match id {
            BlockIdOrTag::L1Accepted => EventBlockId::Pending,
            BlockIdOrTag::PreConfirmed => EventBlockId::Pending,
            BlockIdOrTag::Number(num) => EventBlockId::Num(num),

            BlockIdOrTag::Latest => {
                let num = provider.convert_block_id(id)?;
                EventBlockId::Num(num.ok_or(StarknetApiError::BlockNotFound)?)
            }

            BlockIdOrTag::Hash(..) => {
                // Check first if the block hash belongs to a local block.
                if let Some(num) = provider.convert_block_id(id)? {
                    EventBlockId::Num(num)
                } else {
                    return Err(StarknetApiError::BlockNotFound);
                }
            }
        };

        Ok(id)
    }

    pub async fn get_proofs(
        &self,
        block_id: BlockIdOrTag,
        class_hashes: Option<Vec<ClassHash>>,
        contract_addresses: Option<Vec<ContractAddress>>,
        contracts_storage_keys: Option<Vec<ContractStorageKeys>>,
    ) -> StarknetApiResult<GetStorageProofResponse> {
        self.on_io_blocking_task(move |this| {
            let provider = this.storage().provider();

            let Some(block_num) = provider.convert_block_id(block_id)? else {
                return Err(StarknetApiError::BlockNotFound);
            };

            // Check if the total number of keys requested exceeds the RPC limit.
            if let Some(limit) = this.inner.config.max_proof_keys {
                let total_keys = class_hashes.as_ref().map(|v| v.len()).unwrap_or(0)
                    + contract_addresses.as_ref().map(|v| v.len()).unwrap_or(0)
                    + contracts_storage_keys.as_ref().map(|v| v.len()).unwrap_or(0);

                let total_keys = total_keys as u64;
                if total_keys > limit {
                    return Err(StarknetApiError::ProofLimitExceeded(ProofLimitExceededData {
                        limit,
                        total: total_keys,
                    }));
                }
            }

            // TODO: the way we handle the block id is very clanky. change it!
            let state = this.state(&BlockIdOrTag::Number(block_num))?;
            let block_hash = provider
                .block_hash_by_num(block_num)?
                .ok_or(ProviderError::MissingBlockHeader(block_num))?;

            // --- Get classes proof (if any)

            let classes_proof = if let Some(classes) = class_hashes {
                let proofs = state.class_multiproof(classes)?;
                ClassesProof { nodes: proofs.into() }
            } else {
                ClassesProof::default()
            };

            // --- Get contracts proof (if any)

            let contracts_proof = if let Some(addresses) = contract_addresses {
                let proofs = state.contract_multiproof(addresses.clone())?;
                let mut contract_leaves_data = Vec::new();

                for address in addresses {
                    let nonce = state.nonce(address)?.unwrap_or_default();
                    let class_hash = state.class_hash_of_contract(address)?.unwrap_or_default();
                    let storage_root = state.storage_root(address)?.unwrap_or_default();
                    contract_leaves_data.push(ContractLeafData { storage_root, class_hash, nonce });
                }

                ContractsProof { nodes: proofs.into(), contract_leaves_data }
            } else {
                ContractsProof::default()
            };

            // --- Get contracts storage proof (if any)

            let contracts_storage_proofs = if let Some(contract_storage) = contracts_storage_keys {
                let mut nodes: Vec<Nodes> = Vec::new();

                for ContractStorageKeys { address, keys } in contract_storage {
                    let proofs = state.storage_multiproof(address, keys)?;
                    nodes.push(proofs.into());
                }

                ContractStorageProofs { nodes }
            } else {
                ContractStorageProofs::default()
            };

            let classes_tree_root = state.classes_root()?;
            let contracts_tree_root = state.contracts_root()?;
            let global_roots = GlobalRoots { block_hash, classes_tree_root, contracts_tree_root };

            Ok(GetStorageProofResponse {
                global_roots,
                classes_proof,
                contracts_proof,
                contracts_storage_proofs,
            })
        })
        .await?
    }

    pub async fn block_traces(
        &self,
        block_id: ConfirmedBlockIdOrTag,
    ) -> Result<Vec<TxTraceWithHash>, StarknetApiError> {
        self.on_io_blocking_task(move |this| {
            use StarknetApiError::BlockNotFound;

            let provider = &this.storage().provider();

            let block_id: BlockHashOrNumber = match block_id {
                ConfirmedBlockIdOrTag::L1Accepted => {
                    unimplemented!("l1 accepted block id")
                }
                ConfirmedBlockIdOrTag::Latest => provider.latest_number()?.into(),
                ConfirmedBlockIdOrTag::Number(num) => num.into(),
                ConfirmedBlockIdOrTag::Hash(hash) => hash.into(),
            };

            let indices = provider.block_body_indices(block_id)?.ok_or(BlockNotFound)?;
            let tx_hashes = provider.transaction_hashes_in_range(indices.into())?;

            let traces =
                provider.transaction_executions_by_block(block_id)?.ok_or(BlockNotFound)?;
            let traces = traces.into_iter().map(TxTrace::from);

            let result = tx_hashes
                .into_iter()
                .zip(traces)
                .map(|(h, r)| TxTraceWithHash { transaction_hash: h, trace_root: r })
                .collect::<Vec<_>>();

            Ok(result)
        })
        .await?
    }

    pub async fn trace(&self, tx_hash: TxHash) -> Result<TxTrace, StarknetApiError> {
        self.on_io_blocking_task(move |this| {
            // Check in the pending block first
            if let Some(pending_trace) =
                this.inner.pending_block_provider.get_pending_trace(tx_hash)?
            {
                Ok(pending_trace)
            } else {
                // If not found in pending block, fallback to the provider
                let trace = this
                    .storage()
                    .provider()
                    .transaction_execution(tx_hash)?
                    .ok_or(StarknetApiError::TxnHashNotFound)?;

                Ok(TxTrace::from(trace))
            }
        })
        .await?
    }
}

impl<Pool, PoolTx, Pending, PF> StarknetApi<Pool, Pending, PF>
where
    Pool: TransactionPool<Transaction = PoolTx> + Send + Sync + 'static,
    PoolTx: From<BroadcastedTxWithChainId>,
    Pending: PendingBlockProvider,
    PF: ProviderFactory,
    <PF as ProviderFactory>::Provider: ProviderRO,
{
    pub async fn simulate_txs(
        &self,
        block_id: BlockIdOrTag,
        transactions: Vec<BroadcastedTx>,
        simulation_flags: Vec<SimulationFlag>,
    ) -> Result<Vec<SimulatedTransactions>, StarknetApiError> {
        self.on_cpu_blocking_task(move |this| async move {
            let chain = this.inner.chain_spec.id();

            let executables = transactions
                .into_iter()
                .map(|tx| {
                    let is_query = tx.is_query();
                    let tx = ExecutableTx::from(BroadcastedTxWithChainId { tx, chain });
                    ExecutableTxWithHash::new_query(tx, is_query)
                })
                .collect::<Vec<_>>();

            // If the node is run with transaction validation disabled, then we should not validate
            // even if the `SKIP_VALIDATE` flag is not set.
            let should_validate = !simulation_flags.contains(&SimulationFlag::SkipValidate)
                && this.inner.config.simulation_flags.account_validation();

            // If the node is run with fee charge disabled, then we should disable charing fees even
            // if the `SKIP_FEE_CHARGE` flag is not set.
            let should_charge_fee = !simulation_flags.contains(&SimulationFlag::SkipFeeCharge)
                && this.inner.config.simulation_flags.fee();

            let flags = katana_executor::ExecutionFlags::new()
                .with_account_validation(should_validate)
                .with_fee(should_charge_fee)
                .with_nonce_check(false);

            // get the state and block env at the specified block for execution
            let state = this.state(&block_id)?;
            let env = this.block_env_at(&block_id)?;

            // use the blockifier utils function
            let chain_spec = this.inner.chain_spec.as_ref();
            let overrides = this.inner.config.versioned_constant_overrides.as_ref();
            let results =
                self::blockifier::simulate(chain_spec, state, env, overrides, executables, flags);

            let mut simulated = Vec::with_capacity(results.len());
            for (i, ResultAndStates { result, .. }) in results.into_iter().enumerate() {
                match result {
                    ExecutionResult::Success { trace, receipt } => {
                        let trace = TypedTransactionExecutionInfo::new(receipt.r#type(), trace);

                        let transaction_trace = TxTrace::from(trace);
                        let fee_estimation =
                            to_rpc_fee_estimate(receipt.resources_used(), receipt.fee());
                        let value = SimulatedTransactions { transaction_trace, fee_estimation };

                        simulated.push(value)
                    }

                    ExecutionResult::Failed { error } => {
                        return Err(StarknetApiError::transaction_execution_error(
                            i as u64,
                            error.to_string(),
                        ));
                    }
                }
            }

            Ok(simulated)
        })
        .await?
    }
}

/////////////////////////////////////////////////////
// `StarknetApiExt` Implementations
/////////////////////////////////////////////////////

impl<Pool, PP, PF> StarknetApi<Pool, PP, PF>
where
    Pool: TransactionPool + 'static,
    PP: PendingBlockProvider,
    PF: ProviderFactory,
    <PF as ProviderFactory>::Provider: ProviderRO,
{
    async fn blocks(&self, request: GetBlocksRequest) -> StarknetApiResult<GetBlocksResponse> {
        self.on_io_blocking_task(move |this| {
            let provider = this.storage().provider();

            // Parse continuation token to get starting point
            let start_from = if let Some(token_str) = request.result_page_request.continuation_token
            {
                // Parse the continuation token and extract the item number
                ListContinuationToken::parse(&token_str)
                    .map(|token| token.item_n)
                    .map_err(|_| StarknetApiError::InvalidContinuationToken)?
            } else {
                request.from
            };

            // `latest_number` returns the number of the latest block, and block number starts from
            // 0.
            //
            // Unlike for `StarknetApi::transactions` where we use
            // `TransactionsProviderExt::total_transactions` which returns the total
            // number of transactions overall, the block number here is a block index so we don't
            // need to subtract by 1.
            let last_block_idx = provider.latest_number()?;
            let chunk_size = request.result_page_request.chunk_size;

            // Determine the theoretical end of the range based on how many blocks we actually
            // have and the `to` field of this query. The range shouldn't exceed the total of
            // available blocks!
            //
            // If the `to` field is not provided, we assume the end of the range is the last
            // block.
            let max_block_end =
                request.to.map(|to| to.min(last_block_idx)).unwrap_or(last_block_idx);

            // Get the end of the range based solely on the chunk size.
            // We must respect the chunk size if the range is larger than the chunk size.
            //
            // Subtract by one because we're referring this as a block index.
            let chunked_end = start_from.saturating_add(chunk_size).saturating_sub(1);
            // But, it must not exceed the theoretical end of the range.
            let abs_end = chunked_end.min(max_block_end);

            // Unlike the transactiosn counterpart, we don't need to add by one here because the
            // range is inclusive.
            let block_range = start_from..=abs_end;
            let mut blocks = Vec::with_capacity(chunk_size as usize);

            for block_num in block_range {
                let block = BlockBuilder::new(block_num.into(), &provider)
                    .build_with_tx_hash()?
                    .expect("must exist");

                blocks.push(block);
            }

            // Calculate the next block index to fetch after this query's range.
            let next_block_idx = abs_end + 1;

            // Create a continuation token if we have still more blocks to fetch.
            //
            // `next_block_idx` is not included in this query, hence why we're using <=.
            let continuation_token = if next_block_idx <= max_block_end {
                Some(ListContinuationToken { item_n: next_block_idx }.to_string())
            } else {
                None
            };

            Ok(GetBlocksResponse { blocks, continuation_token })
        })
        .await?
    }

    // NOTE: The current implementation of this method doesn't support pending transactions.
    async fn transactions(
        &self,
        request: GetTransactionsRequest,
    ) -> StarknetApiResult<GetTransactionsResponse> {
        self.on_io_blocking_task(move |this| {
            let provider = this.storage().provider();

            // Resolve the starting point for this query.
            let start_from = if let Some(token_str) = request.result_page_request.continuation_token
            {
                ListContinuationToken::parse(&token_str)
                    .map(|token| token.item_n)
                    .map_err(|_| StarknetApiError::InvalidContinuationToken)?
            } else {
                request.from
            };

            let last_txn_idx = (provider.total_transactions()? as TxNumber).saturating_sub(1);
            let chunk_size = request.result_page_request.chunk_size;

            // Determine the theoretical end of the range based on how many transactions we actually
            // have and the `to` field of this query. The range shouldn't exceed the total of
            // available transactions!
            //
            // If the `to` field is not provided, we assume the end of the range is the last
            // transaction.
            let max_txn_end = request.to.map(|to| to.min(last_txn_idx)).unwrap_or(last_txn_idx);

            // Get the end of the range based solely on the chunk size.
            // We must respect the chunk size if the range is larger than the chunk size.
            //
            // Subtract by one because we're referring this as a transaction index.
            let chunked_end = start_from.saturating_add(chunk_size).saturating_sub(1);
            // But, it must not exceed the theoretical end of the range.
            let abs_end = chunked_end.min(max_txn_end);

            // Calculate the next transaction index to fetch after this query's range.
            let next_txn_idx = abs_end + 1;

            // We use `next_txn_idx` because the range is non-inclusive - we want to include the
            // transaction pointed by `abs_end`.
            let tx_range = start_from..next_txn_idx;
            let tx_hashes = provider.transaction_hashes_in_range(tx_range)?;

            let mut transactions: Vec<TransactionListItem> = Vec::with_capacity(tx_hashes.len());

            for hash in tx_hashes {
                let transaction =
                    provider.transaction_by_hash(hash)?.map(RpcTxWithHash::from).ok_or(
                        StarknetApiError::unexpected(format!("transaction is missing; {hash:#}")),
                    )?;

                let receipt = ReceiptBuilder::new(hash, &provider).build()?.ok_or(
                    StarknetApiError::unexpected(format!("transaction is missing; {hash:#}")),
                )?;

                transactions.push(TransactionListItem { transaction, receipt });
            }

            // Generate continuation token if there are more transactions
            let continuation_token = if next_txn_idx <= max_txn_end {
                // the token should point to the next transaction because `abs_end` is included in
                // this query.
                Some(ListContinuationToken { item_n: next_txn_idx }.to_string())
            } else {
                None
            };

            Ok(GetTransactionsResponse { transactions, continuation_token })
        })
        .await?
    }

    async fn total_transactions(&self) -> StarknetApiResult<TxNumber> {
        self.on_io_blocking_task(move |this| {
            let provider = this.storage().provider();
            let total = provider.total_transactions()? as TxNumber;
            Ok(total)
        })
        .await?
    }
}

impl<Pool, PP, PF> Clone for StarknetApi<Pool, PP, PF>
where
    Pool: TransactionPool,
    PP: PendingBlockProvider,
    PF: ProviderFactory,
{
    fn clone(&self) -> Self {
        Self { inner: Arc::clone(&self.inner) }
    }
}
