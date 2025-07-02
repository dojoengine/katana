use std::sync::Arc;

// Re-export the blockifier crate.
pub use blockifier;
use blockifier::bouncer::{n_steps_to_sierra_gas, Bouncer, BouncerConfig, BouncerWeights};

pub mod cache;
pub mod call;
mod error;
pub mod state;
pub mod utils;

use blockifier::context::BlockContext;
use cache::ClassCache;
use katana_primitives::block::{ExecutableBlock, GasPrice as KatanaGasPrices, PartialHeader};
use katana_primitives::env::{BlockEnv, CfgEnv};
use katana_primitives::transaction::{ExecutableTx, ExecutableTxWithHash, TxWithHash};
use katana_primitives::version::StarknetVersion;
use katana_provider::traits::state::StateProvider;
use starknet_api::block::{
    BlockInfo, BlockNumber, BlockTimestamp, GasPriceVector, GasPrices, NonzeroGasPrice,
};
use tracing::info;

use self::state::CachedState;
use crate::{
    BlockExecutor, BlockLimits, ExecutionFlags, ExecutionOutput, ExecutionResult, ExecutionStats,
    ExecutorError, ExecutorFactory, ExecutorResult,
};

pub(crate) const LOG_TARGET: &str = "katana::executor::blockifier";

#[derive(Debug, Clone)]
pub struct BlockifierFactory {
    cfg: CfgEnv,
    flags: ExecutionFlags,
    limits: BlockLimits,

    class_cache: ClassCache,
}

impl BlockifierFactory {
    /// Create a new factory with the given configuration and simulation flags.
    pub fn new(
        cfg: CfgEnv,
        flags: ExecutionFlags,
        limits: BlockLimits,
        class_cache: ClassCache,
    ) -> Self {
        Self { cfg, flags, limits, class_cache }
    }
}

impl ExecutorFactory for BlockifierFactory {
    fn with_state<'a, P>(&self, state: P) -> Box<dyn BlockExecutor<'a> + 'a>
    where
        P: StateProvider + 'a,
    {
        self.with_state_and_block_env(state, BlockEnv::default())
    }

    fn with_state_and_block_env<'a, P>(
        &self,
        state: P,
        block_env: BlockEnv,
    ) -> Box<dyn BlockExecutor<'a> + 'a>
    where
        P: StateProvider + 'a,
    {
        let cfg_env = self.cfg.clone();
        let flags = self.flags.clone();
        let limits = self.limits.clone();
        Box::new(StarknetVMProcessor::new(
            Box::new(state),
            block_env,
            cfg_env,
            flags,
            limits,
            self.class_cache.clone(),
        ))
    }

    fn cfg(&self) -> &CfgEnv {
        &self.cfg
    }

    /// Returns the execution flags set by the factory.
    fn execution_flags(&self) -> &ExecutionFlags {
        &self.flags
    }
}

#[derive(Debug)]
pub struct StarknetVMProcessor<'a> {
    block_context: Arc<BlockContext>,
    state: CachedState<'a>,
    transactions: Vec<(TxWithHash, ExecutionResult)>,
    simulation_flags: ExecutionFlags,
    stats: ExecutionStats,
    bouncer: Bouncer,
    starknet_version: StarknetVersion,
}

impl<'a> StarknetVMProcessor<'a> {
    pub fn new(
        state: impl StateProvider + 'a,
        block_env: BlockEnv,
        cfg_env: CfgEnv,
        simulation_flags: ExecutionFlags,
        limits: BlockLimits,
        class_cache: ClassCache,
    ) -> Self {
        let transactions = Vec::new();
        let starknet_version: StarknetVersion = block_env.starknet_version;
        let block_context = Arc::new(utils::block_context_from_envs(&block_env, &cfg_env));

        let state = state::CachedState::new(state, class_cache);

        let mut block_max_capacity = BouncerWeights::max();

        // Initially, the primary reason why we introduced the cairo steps limit is to limit the
        // number of steps that needs to be proven during the prove generation process. As
        // of Starknet v0.13.4 update, a new type of resources is introduced, that is the L2 gas.
        // Which is supposed to pay for every L2-related resources (eg., computation, and
        // other blockchain-related resources such as tx payload, events emission, etc.)
        //
        // Now blockifier uses L2 gas as the primary resource for pricing the transactions. Hence,
        // we need to convert the cairo steps limit to L2 gas. Where 1 Cairo step = 100 L2
        // gas.
        //
        // To learn more about the L2 gas, refer to <https://community.starknet.io/t/starknet-v0-13-4-pre-release-notes/115257>.
        block_max_capacity.sierra_gas =
            n_steps_to_sierra_gas(limits.cairo_steps as usize, block_context.versioned_constants());

        let bouncer = Bouncer::new(BouncerConfig { block_max_capacity });

        Self {
            state,
            transactions,
            block_context,
            simulation_flags,
            stats: Default::default(),
            bouncer,
            starknet_version,
        }
    }

    fn fill_block_env_from_header(&mut self, header: &PartialHeader) {
        let number = BlockNumber(header.number);
        let timestamp = BlockTimestamp(header.timestamp);

        let eth_l2_gas_price = NonzeroGasPrice::new(header.l2_gas_prices.eth.get().into())
            .unwrap_or(NonzeroGasPrice::MIN);
        let strk_l2_gas_price = NonzeroGasPrice::new(header.l2_gas_prices.strk.get().into())
            .unwrap_or(NonzeroGasPrice::MIN);
        let eth_l1_gas_price = NonzeroGasPrice::new(header.l1_gas_prices.eth.get().into())
            .unwrap_or(NonzeroGasPrice::MIN);
        let strk_l1_gas_price = NonzeroGasPrice::new(header.l1_gas_prices.strk.get().into())
            .unwrap_or(NonzeroGasPrice::MIN);
        let eth_l1_data_gas_price =
            NonzeroGasPrice::new(header.l1_data_gas_prices.eth.get().into())
                .unwrap_or(NonzeroGasPrice::MIN);
        let strk_l1_data_gas_price =
            NonzeroGasPrice::new(header.l1_data_gas_prices.strk.get().into())
                .unwrap_or(NonzeroGasPrice::MIN);

        // TODO: @kariy, not sure here if we should add some functions to alter it
        // instead of cloning. Or did I miss a function?
        // https://github.com/starkware-libs/blockifier/blob/a6200402ab635d8a8e175f7f135be5914c960007/crates/blockifier/src/context.rs#L23
        let versioned_constants = self.block_context.versioned_constants().clone();
        let chain_info = self.block_context.chain_info().clone();
        let block_info = BlockInfo {
            block_number: number,
            block_timestamp: timestamp,
            sequencer_address: utils::to_blk_address(header.sequencer_address),
            gas_prices: GasPrices {
                eth_gas_prices: GasPriceVector {
                    l2_gas_price: eth_l2_gas_price,
                    l1_gas_price: eth_l1_gas_price,
                    l1_data_gas_price: eth_l1_data_gas_price,
                },
                strk_gas_prices: GasPriceVector {
                    l2_gas_price: strk_l2_gas_price,
                    l1_gas_price: strk_l1_gas_price,
                    l1_data_gas_price: strk_l1_data_gas_price,
                },
            },
            use_kzg_da: false,
        };

        self.block_context = Arc::new(BlockContext::new(
            block_info,
            chain_info,
            versioned_constants,
            Default::default(),
        ));
    }
}

impl<'a> BlockExecutor<'a> for StarknetVMProcessor<'a> {
    fn execute_block(&mut self, block: ExecutableBlock) -> ExecutorResult<()> {
        self.fill_block_env_from_header(&block.header);
        self.execute_transactions(block.body)?;
        Ok(())
    }

    fn execute_transactions(
        &mut self,
        transactions: Vec<ExecutableTxWithHash>,
    ) -> ExecutorResult<(usize, Option<ExecutorError>)> {
        let block_context = &self.block_context;
        let flags = &self.simulation_flags;
        let mut state = self.state.inner.lock();

        let mut total_executed = 0;
        for exec_tx in transactions {
            // Collect class artifacts if its a declare tx
            let class_decl_artifacts = if let ExecutableTx::Declare(tx) = exec_tx.as_ref() {
                let class_hash = tx.class_hash();
                Some((class_hash, tx.class.clone()))
            } else {
                None
            };

            let tx = TxWithHash::from(&exec_tx);
            let hash = tx.hash;
            let result = utils::transact(
                &mut state.cached_state,
                block_context,
                flags,
                exec_tx,
                Some(&mut self.bouncer),
            );

            match result {
                Ok(exec_result) => {
                    match &exec_result {
                        ExecutionResult::Success { receipt, trace } => {
                            if let Some(reason) = receipt.revert_reason() {
                                info!(target: LOG_TARGET, hash = format!("{hash:#x}"), %reason, "Transaction reverted.");
                            }

                            if let Some((class_hash, class)) = class_decl_artifacts {
                                state.declared_classes.insert(class_hash, class.as_ref().clone());
                            }

                            crate::utils::log_resources(&trace.receipt.resources);
                        }

                        ExecutionResult::Failed { error } => {
                            info!(target: LOG_TARGET, hash = format!("{hash:#x}"), %error, "Executing transaction.");
                        }
                    }

                    total_executed += 1;
                    self.transactions.push((tx, exec_result));
                }

                Err(e @ ExecutorError::LimitsExhausted) => return Ok((total_executed, Some(e))),
                Err(e) => return Err(e),
            };
        }

        Ok((total_executed, None))
    }

    fn take_execution_output(&mut self) -> ExecutorResult<ExecutionOutput> {
        let states = utils::state_update_from_cached_state(&self.state);
        let transactions = std::mem::take(&mut self.transactions);
        let stats = std::mem::take(&mut self.stats);
        Ok(ExecutionOutput { stats, states, transactions })
    }

    fn state(&self) -> Box<dyn StateProvider + 'a> {
        Box::new(self.state.clone())
    }

    fn transactions(&self) -> &[(TxWithHash, ExecutionResult)] {
        &self.transactions
    }

    fn block_env(&self) -> BlockEnv {
        let l2_gas_prices = unsafe {
            KatanaGasPrices::new_unchecked(
                self.block_context.block_info().gas_prices.eth_gas_prices.l2_gas_price.get().0,
                self.block_context.block_info().gas_prices.strk_gas_prices.l2_gas_price.get().0,
            )
        };

        let l1_gas_prices = unsafe {
            KatanaGasPrices::new_unchecked(
                self.block_context.block_info().gas_prices.eth_gas_prices.l1_gas_price.get().0,
                self.block_context.block_info().gas_prices.strk_gas_prices.l1_gas_price.get().0,
            )
        };

        let l1_data_gas_prices = unsafe {
            KatanaGasPrices::new_unchecked(
                self.block_context.block_info().gas_prices.eth_gas_prices.l1_data_gas_price.get().0,
                self.block_context
                    .block_info()
                    .gas_prices
                    .strk_gas_prices
                    .l1_data_gas_price
                    .get()
                    .0,
            )
        };

        BlockEnv {
            l2_gas_prices,
            l1_gas_prices,
            l1_data_gas_prices,
            number: self.block_context.block_info().block_number.0,
            timestamp: self.block_context.block_info().block_timestamp.0,
            sequencer_address: utils::to_address(self.block_context.block_info().sequencer_address),
            starknet_version: self.starknet_version,
        }
    }
}
