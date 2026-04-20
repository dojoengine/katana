use std::sync::Arc;

use katana_chain_spec::ChainSpec;
use katana_executor::blockifier::cache::ClassCache;
use katana_executor::blockifier::call::execute_call;
use katana_executor::blockifier::state::CachedState;
use katana_executor::blockifier::utils::{self, block_context_from_envs};
use katana_executor::error::ExecutionError;
use katana_executor::{ExecutionFlags, ExecutionResult, ResultAndStates};
use katana_primitives::env::{BlockEnv, VersionedConstantsOverrides};
use katana_primitives::transaction::ExecutableTxWithHash;
use katana_primitives::Felt;
use katana_provider::api::state::StateProvider;
use katana_rpc_api::error::starknet::{ContractErrorData, StarknetApiError};
use katana_rpc_types::{FeeEstimate, FunctionCall};

use crate::starknet::StarknetApiResult;

#[tracing::instrument(level = "trace", target = "rpc", skip_all, fields(total_txs = transactions.len()))]
pub fn simulate(
    chain_spec: &ChainSpec,
    state: impl StateProvider + 'static,
    block_env: BlockEnv,
    overrides: Option<&VersionedConstantsOverrides>,
    transactions: Vec<ExecutableTxWithHash>,
    flags: ExecutionFlags,
    class_cache: &ClassCache,
) -> Vec<ResultAndStates> {
    let block_context = Arc::new(block_context_from_envs(chain_spec, &block_env, overrides));
    let state = CachedState::new(state, class_cache.clone());
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

/// Estimates the fees for a list of transactions.
///
/// ## Errors
///
/// If one of the transactions reverts or fails due to any reason (e.g. validation failure or an
/// internal error), the function will fail with [`StarknetApiError::TransactionExecutionError`]
/// error.
///
/// This is in accordance with the Starknet RPC [specification] for the `starknet_estimateFee`
/// method.
///
/// [specification]: https://github.com/starkware-libs/starknet-specs/blob/c2e93098b9c2ca0423b7f4d15b201f52f22d8c36/api/starknet_api_openrpc.json#L623
#[tracing::instrument(level = "trace", target = "rpc", skip_all, fields(total_txs = transactions.len()))]
pub fn estimate_fees(
    chain_spec: &ChainSpec,
    state: impl StateProvider + 'static,
    block_env: BlockEnv,
    overrides: Option<&VersionedConstantsOverrides>,
    transactions: Vec<ExecutableTxWithHash>,
    flags: ExecutionFlags,
    class_cache: &ClassCache,
) -> StarknetApiResult<Vec<FeeEstimate>> {
    let flags = flags.with_fee(false);
    let block_context = block_context_from_envs(chain_spec, &block_env, overrides);
    let state = CachedState::new(state, class_cache.clone());

    state.with_mut_cached_state(|state| {
        let mut estimates = Vec::with_capacity(transactions.len());

        for (idx, tx) in transactions.into_iter().enumerate() {
            // Safe to unwrap here because the only way the call to `transact` can return an
            // error is when bouncer is `Some`.
            let result = utils::transact(state, &block_context, &flags, tx, None).unwrap();

            match result {
                ExecutionResult::Success { receipt, .. } => {
                    if let Some(reason) = receipt.revert_reason() {
                        // ideally, we would check for the contract deployment status before
                        // execution
                        return if is_undeployed_contract_error(reason) {
                            Err(StarknetApiError::ContractNotFound)
                        } else {
                            Err(StarknetApiError::transaction_execution_error(
                                idx as u64,
                                reason.to_string(),
                            ))
                        };
                    }

                    let fee = receipt.fee();
                    let resources = receipt.resources_used();

                    estimates.push(FeeEstimate {
                        overall_fee: fee.overall_fee,
                        l2_gas_price: fee.l2_gas_price,
                        l1_gas_price: fee.l1_gas_price,
                        l2_gas_consumed: resources.total_gas_consumed.l2_gas,
                        l1_gas_consumed: resources.total_gas_consumed.l1_gas,
                        l1_data_gas_price: fee.l1_data_gas_price,
                        l1_data_gas_consumed: resources.total_gas_consumed.l1_data_gas,
                    });
                }

                ExecutionResult::Failed { error } => {
                    let error = error.to_string();

                    return if is_undeployed_contract_error(&error) {
                        Err(StarknetApiError::ContractNotFound)
                    } else {
                        Err(StarknetApiError::transaction_execution_error(idx as u64, error))
                    };
                }
            };
        }

        Ok(estimates)
    })
}

#[tracing::instrument(level = "trace", target = "rpc", skip_all)]
pub fn call<P: StateProvider + 'static>(
    chain_spec: &ChainSpec,
    state: P,
    block_env: BlockEnv,
    overrides: Option<&VersionedConstantsOverrides>,
    call: FunctionCall,
    max_call_gas: u64,
    class_cache: &ClassCache,
) -> Result<Vec<Felt>, StarknetApiError> {
    let block_context = Arc::new(block_context_from_envs(chain_spec, &block_env, overrides));
    let state = CachedState::new(state, class_cache.clone());

    state
        .with_mut_cached_state(|state| execute_call(call, state, block_context, max_call_gas))
        .map_err(to_api_error)
}

fn is_undeployed_contract_error(error: &str) -> bool {
    use std::sync::LazyLock;

    use regex::Regex;

    static RE: LazyLock<Regex> = LazyLock::new(|| {
        // Error message returned by blockifier when a contract is not deployed during execution
        Regex::new(r"Requested contract address 0x[a-fA-F0-9]+ is not deployed").unwrap()
    });

    RE.is_match(error)
}

// fn is_undeployed_contract_error_fast(error_msg: &str) -> bool {
//     // Check for the prefix and suffix around where the address would be
//     error_msg.contains("Requested contract address 0x") &&
//     error_msg.contains(" is not deployed")
// }

fn to_api_error(error: ExecutionError) -> StarknetApiError {
    match error {
        ExecutionError::EntryPointNotFound(..) => StarknetApiError::EntrypointNotFound,
        ExecutionError::ContractNotDeployed(..) => StarknetApiError::ContractNotFound,
        error => {
            StarknetApiError::ContractError(ContractErrorData { revert_error: error.to_string() })
        }
    }
}

#[cfg(test)]
mod tests {

    use katana_chain_spec::ChainSpec;
    use katana_executor::blockifier::cache::ClassCache;
    use katana_primitives::address;
    use katana_primitives::env::BlockEnv;
    use katana_provider::api::state::StateFactoryProvider;
    use katana_provider::test_utils::test_provider;
    use katana_provider::ProviderFactory;
    use katana_rpc_api::error::starknet::StarknetApiError;
    use katana_rpc_types::FunctionCall;
    use starknet::macros::selector;

    #[test]
    fn call_on_contract_not_deployed() {
        let provider_factory = test_provider();
        let provider = provider_factory.provider();

        let state = provider.latest().unwrap();

        let max_call_gas = 1_000_000_000;
        let block_env = BlockEnv::default();
        let class_cache = ClassCache::new().unwrap();

        let call = FunctionCall {
            calldata: Vec::new(),
            contract_address: address!("1337"),
            entry_point_selector: selector!("foo"),
        };

        let result = super::call(
            &ChainSpec::dev(),
            state,
            block_env,
            None,
            call,
            max_call_gas,
            &class_cache,
        );
        assert!(matches!(result, Err(StarknetApiError::ContractNotFound)));
    }

    #[test]
    fn call_on_entry_point_not_found() {
        let provider_factory = test_provider();
        let provider = provider_factory.provider();

        let state = provider.latest().unwrap();

        let max_call_gas = 1_000_000_000;
        let block_env = BlockEnv::default();
        let class_cache = ClassCache::new().unwrap();

        let call = FunctionCall {
            calldata: Vec::new(),
            contract_address: address!("0x1"),
            entry_point_selector: selector!("foobar"),
        };

        let result = super::call(
            &ChainSpec::dev(),
            state,
            block_env,
            None,
            call,
            max_call_gas,
            &class_cache,
        );
        assert!(matches!(result, Err(StarknetApiError::EntrypointNotFound)));
    }
}
