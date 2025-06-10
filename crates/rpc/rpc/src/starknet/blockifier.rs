use std::sync::Arc;

use katana_executor::implementation::blockifier::cache::ClassCache;
use katana_executor::implementation::blockifier::state::CachedState;
use katana_executor::implementation::blockifier::utils::{self, block_context_from_envs};
use katana_executor::{
    EntryPointCall, ExecutionError, ExecutionFlags, ExecutionResult, ResultAndStates,
};
use katana_primitives::env::{BlockEnv, CfgEnv};
use katana_primitives::fee::{self};
use katana_primitives::transaction::ExecutableTxWithHash;
use katana_primitives::Felt;
use katana_provider::traits::state::StateProvider;
use katana_rpc_types::FeeEstimate;
use starknet::core::types::PriceUnit;

#[tracing::instrument(level = "trace", target = "rpc", skip_all, fields(total_txs = transactions.len()))]
pub fn simulate(
    state: impl StateProvider,
    block_env: BlockEnv,
    cfg_env: CfgEnv,
    transactions: Vec<ExecutableTxWithHash>,
    flags: ExecutionFlags,
) -> Vec<ResultAndStates> {
    let block_context = Arc::new(block_context_from_envs(&block_env, &cfg_env));
    let state = CachedState::new(state, ClassCache::global().clone());
    let mut results = Vec::with_capacity(transactions.len());

    state.with_mut_cached_state(|state| {
        for tx in transactions {
            // Safe to unwrap here because the only way the call to `transact` can return an error
            // is when bouncer is `Some`.
            let result = utils::transact(state, &block_context, &flags, tx, None).unwrap();
            let simulated_result = ResultAndStates { result, states: Default::default() };

            results.push(simulated_result);
        }
    });

    results
}

#[tracing::instrument(level = "trace", target = "rpc", skip_all, fields(total_txs = transactions.len()))]
pub fn estimate_fees(
    state: impl StateProvider,
    block_env: BlockEnv,
    cfg_env: CfgEnv,
    transactions: Vec<ExecutableTxWithHash>,
    flags: ExecutionFlags,
) -> Vec<Result<FeeEstimate, ExecutionError>> {
    let flags = flags.with_fee(false);
    let block_context = block_context_from_envs(&block_env, &cfg_env);
    let state = CachedState::new(state, ClassCache::global().clone());

    let mut results = Vec::with_capacity(transactions.len());
    state.with_mut_cached_state(|state| {
        for tx in transactions {
            // Safe to unwrap here because the only way the call to `transact` can return an error
            // is when bouncer is `Some`.
            let result = utils::transact(state, &block_context, &flags, tx, None).unwrap();
            let estimated_result = match result {
                ExecutionResult::Failed { error } => Err(error),
                ExecutionResult::Success { receipt, .. } => {
                    let fee = receipt.fee();
                    let resources = receipt.resources_used();

                    let unit = match fee.unit {
                        fee::PriceUnit::Wei => PriceUnit::Wei,
                        fee::PriceUnit::Fri => PriceUnit::Fri,
                    };

                    Ok(FeeEstimate {
                        unit,
                        overall_fee: fee.overall_fee.into(),
                        l2_gas_price: fee.l2_gas_price.into(),
                        l1_gas_price: fee.l1_gas_price.into(),
                        l2_gas_consumed: resources.gas.l2_gas.into(),
                        l1_gas_consumed: resources.gas.l1_gas.into(),
                        l1_data_gas_price: fee.l1_data_gas_price.into(),
                        l1_data_gas_consumed: resources.gas.l1_data_gas.into(),
                    })
                }
            };

            results.push(estimated_result);
        }
    });

    results
}

#[tracing::instrument(level = "trace", target = "rpc", skip_all)]
pub fn call<P: StateProvider>(
    state: P,
    block_env: BlockEnv,
    cfg_env: CfgEnv,
    call: EntryPointCall,
    max_call_gas: u64,
) -> Result<Vec<Felt>, ExecutionError> {
    let block_context = Arc::new(block_context_from_envs(&block_env, &cfg_env));
    let state = CachedState::new(state, ClassCache::global().clone());

    state.with_mut_cached_state(|state| {
        katana_executor::implementation::blockifier::call::execute_call(
            call,
            state,
            block_context,
            max_call_gas,
        )
    })
}
