use std::sync::Arc;

use katana_executor::implementation::blockifier::blockifier::state::cached_state::{
    self, MutRefState,
};
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
    for tx in transactions {
        // Safe to unwrap here because the only way the call to `transact` can return an error
        // is when bouncer is `Some`.
        let result = state.with_cached_state(|cached_state| {
            let mut state = cached_state::CachedState::new(MutRefState::new(cached_state));
            utils::transact(&mut state, &block_context, &flags, tx, None).unwrap()
        });

        results.push(ResultAndStates { result, states: Default::default() });
    }

    results
}

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
    for tx in transactions {
        // Safe to unwrap here because the only way the call to `transact` can return an error
        // is when bouncer is `Some`.
        let res = state.with_cached_state(|cached_state| {
            let mut state = cached_state::CachedState::new(MutRefState::new(cached_state));
            utils::transact(&mut state, &block_context, &flags, tx, None).unwrap()
        });

        let result = match res {
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
            ExecutionResult::Failed { error } => Err(error),
        };

        results.push(result);
    }

    results
}

pub fn call<P: StateProvider>(
    state: P,
    block_env: BlockEnv,
    cfg_env: CfgEnv,
    call: EntryPointCall,
    max_call_gas: u64,
) -> Result<Vec<Felt>, ExecutionError> {
    let block_context = Arc::new(block_context_from_envs(&block_env, &cfg_env));
    let state = CachedState::new(state, ClassCache::global().clone());

    state.with_cached_state(|cached_state| {
        katana_executor::implementation::blockifier::call::execute_call(
            call,
            MutRefState::new(cached_state),
            block_context,
            max_call_gas,
        )
    })
}
