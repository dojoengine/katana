use std::str::FromStr;
use std::sync::Arc;

use blockifier::blockifier_versioned_constants::VersionedConstants;
use blockifier::bouncer::n_steps_to_sierra_gas;
use blockifier::context::{BlockContext, TransactionContext};
use blockifier::execution::call_info::CallInfo;
use blockifier::execution::entry_point::{
    CallEntryPoint, EntryPointExecutionContext, EntryPointExecutionResult, SierraGasRevertTracker,
};
use blockifier::execution::errors::{EntryPointExecutionError, PreExecutionError};
use blockifier::execution::stack_trace::{
    extract_trailing_cairo1_revert_trace, Cairo1RevertHeader,
};
use blockifier::execution::syscalls::hint_processor::ENTRYPOINT_NOT_FOUND_ERROR;
use blockifier::state::cached_state::CachedState;
use blockifier::state::state_api::StateReader;
use blockifier::transaction::objects::{DeprecatedTransactionInfo, TransactionInfo};
use cairo_vm::vm::runners::cairo_runner::RunResources;
use katana_primitives::execution::{FunctionCall, TrackedResource};
use katana_primitives::Felt;
use starknet_api::core::EntryPointSelector;
use starknet_api::execution_resources::GasAmount;
use starknet_api::transaction::fields::Calldata;

use super::utils::to_blk_address;
use crate::ExecutionError;

/// Perform a function call on a contract and retrieve the return values.
///
/// The `max_gas` is the maximum amount of Sierra gas to allocate for the call.
pub fn execute_call<S: StateReader>(
    request: FunctionCall,
    state: &mut CachedState<S>,
    block_context: Arc<BlockContext>,
    max_gas: u64,
) -> Result<Vec<Felt>, ExecutionError> {
    let res = execute_call_inner(request, state, block_context, max_gas)?;
    Ok(res.execution.retdata.0)
}

fn execute_call_inner<S: StateReader>(
    request: FunctionCall,
    state: &mut CachedState<S>,
    block_context: Arc<BlockContext>,
    max_sierra_gas: u64,
) -> EntryPointExecutionResult<CallInfo> {
    let call = CallEntryPoint {
        initial_gas: max_sierra_gas,
        calldata: Calldata(Arc::new(request.calldata)),
        storage_address: to_blk_address(request.contract_address),
        entry_point_selector: EntryPointSelector(request.entry_point_selector),
        ..Default::default()
    };

    // The run resources for a call execution will either be constraint ONLY by the block context
    // limits OR based on the tx fee and gas prices. As can be seen here, the upper bound will
    // always be limited the block max invoke steps https://github.com/dojoengine/blockifier/blob/5f58be8961ddf84022dd739a8ab254e32c435075/crates/blockifier/src/execution/entry_point.rs#L253
    // even if the it's max steps is derived from the tx fees.
    //
    // This if statement here determines how execution will be constraint to <https://github.com/dojoengine/blockifier/blob/5f58be8961ddf84022dd739a8ab254e32c435075/crates/blockifier/src/execution/entry_point.rs#L206-L208>.
    // This basically means, we can not set an arbitrary gas limit for this call without modifying
    // the block context. So, we just set the run resources here manually to bypass that.

    // The values for these parameters are essentially useless as we manually set the run resources
    // later anyway.
    let limit_steps_by_resources = true;

    let tx_context = Arc::new(TransactionContext {
        block_context: block_context.clone(),
        tx_info: TransactionInfo::Deprecated(DeprecatedTransactionInfo::default()),
    });

    let sierra_revert_tracker = SierraGasRevertTracker::new(GasAmount(max_sierra_gas));
    let mut ctx = EntryPointExecutionContext::new_invoke(
        tx_context,
        limit_steps_by_resources,
        sierra_revert_tracker,
    );

    let versioned_constants = block_context.versioned_constants();
    let l2_gas_per_step = versioned_constants.os_constants.gas_costs.base.step_gas_cost;

    // Convert the sierra gas to cairo steps for the run resources.
    //
    // If values can't fit in a usize, use the maximum. This is to mimic the behaviour of
    // blockifier.
    //
    // For reference: https://github.com/dojoengine/sequencer/blob/5d737b9c90a14bdf4483d759d1a1d4ce64aa9fd2/crates/blockifier/src/execution/entry_point.rs#L382C1-L451
    let n_steps = max_sierra_gas.saturating_div(l2_gas_per_step);
    let n_steps = n_steps.try_into().unwrap_or(usize::MAX);
    ctx.vm_run_resources = RunResources::new(n_steps);

    let mut remaining_gas = call.initial_gas;
    let call_info = call.execute(state, &mut ctx, &mut remaining_gas)?;

    // Calls return error as the return data (Ok variant) instead of returning as an
    // Err. Refer to: https://github.com/dojoengine/sequencer/blob/5d737b9c90a14bdf4483d759d1a1d4ce64aa9fd2/crates/blockifier/src/execution/execution_utils.rs#L74
    if call_info.execution.failed {
        match call_info.execution.retdata.0.as_slice() {
            [error_code] if *error_code == Felt::from_str(ENTRYPOINT_NOT_FOUND_ERROR).unwrap() => {
                let error = PreExecutionError::EntryPointNotFound(EntryPointSelector(
                    request.entry_point_selector,
                ));

                return Err(EntryPointExecutionError::PreExecutionError(error));
            }
            _ => {
                let error_trace =
                    extract_trailing_cairo1_revert_trace(&call_info, Cairo1RevertHeader::Execution);
                return Err(EntryPointExecutionError::ExecutionFailed { error_trace });
            }
        }
    }

    Ok(call_info)
}

/// Returns the Sierra gas consumed by the contract call execution.
///
/// Depending on the the Sierra compiler version of the class, the resources used for the contract
/// call execution may either be tracked as Cairo steps or Sierra gas. Note that `blockifier`
/// doesn't store the actual tracked resource consumed value in a single variable of the `CallInfo`
/// struct but as separate variables i.e., `gas_consumed` of [`CallExecution`] and `n_steps` of
/// [`ExecutionResources`] for Sierra gas and Cairo steps respectively.
///
/// For reference: https://github.com/dojoengine/sequencer/blob/5d737b9c90a14bdf4483d759d1a1d4ce64aa9fd2/crates/blockifier/src/execution/entry_point_execution.rs#L367-L433
///
/// [`CallExecution`]: blockifier::execution::call_info::CallExecution
/// [`ExecutionResources`]: cairo_vm::vm::runners::cairo_runner::ExecutionResources
pub fn get_call_sierra_gas_consumed(
    info: &CallInfo,
    versioned_constant: &VersionedConstants,
) -> u64 {
    match info.tracked_resource {
        // If the execution is tracked using Cairo steps, the sierra gas isn't counted
        // ie `info.execution.gas_consumed` is set to 0. So, we have to convert the cairo
        // steps to sierra gas.
        //
        // https://github.com/dojoengine/sequencer/blob/5d737b9c90a14bdf4483d759d1a1d4ce64aa9fd2/crates/blockifier/src/execution/entry_point_execution.rs#L475-L479
        TrackedResource::CairoSteps => {
            n_steps_to_sierra_gas(info.resources.n_steps, versioned_constant).0
        }

        TrackedResource::SierraGas => info.execution.gas_consumed,
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;
    use std::sync::Arc;

    use blockifier::context::BlockContext;
    use blockifier::execution::errors::EntryPointExecutionError;
    use blockifier::state::cached_state::{self};
    use katana_primitives::class::ContractClass;
    use katana_primitives::execution::FunctionCall;
    use katana_primitives::{address, felt};
    use katana_provider::api::contract::ContractClassWriter;
    use katana_provider::api::state::{StateFactoryProvider, StateWriter};
    use katana_provider::{test_utils, ProviderFactory};
    use starknet::macros::selector;

    use super::{execute_call_inner, get_call_sierra_gas_consumed};
    use crate::implementation::blockifier::state::StateProviderDb;

    #[test]
    fn max_steps() {
        // -------------------- Preparations -------------------------------

        let json = include_str!(
            "../../../../contracts/build/katana_test_contracts_CallTest.contract_class.json"
        );
        let class = ContractClass::from_str(json).unwrap();
        let class_hash = class.class_hash().unwrap();
        let casm_hash = class.clone().compile().unwrap().class_hash().unwrap();

        // Initialize provider with the test contract
        let storage = test_utils::test_provider();
        let provider = storage.provider_mut();

        // Declare test contract
        provider.set_class(class_hash, class).unwrap();
        provider.set_compiled_class_hash_of_class_hash(class_hash, casm_hash).unwrap();
        // Deploy test contract
        let address = address!("0x1337");
        provider.set_class_hash_of_contract(address, class_hash).unwrap();

        let state = provider.latest().unwrap();
        let state = StateProviderDb::new(state);

        // ---------------------------------------------------------------

        let mut state = cached_state::CachedState::new(state);
        let ctx = Arc::new(BlockContext::create_for_testing());

        let mut req = FunctionCall {
            calldata: Vec::new(),
            contract_address: address,
            entry_point_selector: selector!("bounded_call"),
        };

        let max_gas_1 = 1_000_000;
        {
            // ~900,000 gas
            req.calldata = vec![felt!("460")];
            let info = execute_call_inner(req.clone(), &mut state, ctx.clone(), max_gas_1).unwrap();
            let gas_consumed = get_call_sierra_gas_consumed(&info, ctx.versioned_constants());

            assert!(!info.execution.failed);
            assert!(max_gas_1 >= gas_consumed);

            req.calldata = vec![felt!("600")];
            let result = execute_call_inner(req.clone(), &mut state, ctx.clone(), max_gas_1);
            assert!(result.is_err(), "should fail due to out of run resources")
        }

        let max_gas_2 = 10_000_000;
        {
            // roughly equivalent to 9,000,000 gas
            req.calldata = vec![felt!("4600")];
            let info = execute_call_inner(req.clone(), &mut state, ctx.clone(), max_gas_2).unwrap();
            let gas_consumed = get_call_sierra_gas_consumed(&info, ctx.versioned_constants());

            assert!(!info.execution.failed);
            assert!(max_gas_2 >= gas_consumed);
            assert!(max_gas_1 < gas_consumed);

            req.calldata = vec![felt!("6000")];
            let result = execute_call_inner(req.clone(), &mut state, ctx.clone(), max_gas_2);
            assert!(result.is_err(), "should fail due to out of run resources")
        }

        let max_gas_3 = 100_000_000;
        {
            req.calldata = vec![felt!("47000")];
            let info = execute_call_inner(req.clone(), &mut state, ctx.clone(), max_gas_3).unwrap();
            let gas_consumed = get_call_sierra_gas_consumed(&info, ctx.versioned_constants());

            assert!(!info.execution.failed);
            assert!(max_gas_3 >= gas_consumed);
            assert!(max_gas_2 < gas_consumed);

            req.calldata = vec![felt!("60000")];
            let result = execute_call_inner(req.clone(), &mut state, ctx.clone(), max_gas_3);
            assert!(result.is_err(), "should fail due to out of run resources")
        }

        // Check that 'call' isn't bounded by the block context max invoke steps
        assert!(max_gas_3 > ctx.versioned_constants().invoke_tx_max_n_steps as u64);
    }

    #[test]
    fn call_with_panic() {
        // -------------------- Preparations -------------------------------

        let json = include_str!(
            "../../../../contracts/build/katana_test_contracts_CallTest.contract_class.json"
        );
        let class = ContractClass::from_str(json).unwrap();
        let class_hash = class.class_hash().unwrap();
        let casm_hash = class.clone().compile().unwrap().class_hash().unwrap();

        // Initialize provider with the test contract
        let storage = test_utils::test_provider();
        let provider = storage.provider_mut();

        // Declare test contract
        provider.set_class(class_hash, class).unwrap();
        provider.set_compiled_class_hash_of_class_hash(class_hash, casm_hash).unwrap();
        // Deploy test contract
        let address = address!("0x1337");
        provider.set_class_hash_of_contract(address, class_hash).unwrap();

        let state = provider.latest().unwrap();
        let state = StateProviderDb::new(state);

        // ---------------------------------------------------------------

        let mut state = cached_state::CachedState::new(state);
        let ctx = Arc::new(BlockContext::create_for_testing());

        let req = FunctionCall {
            calldata: Vec::new(),
            contract_address: address,
            entry_point_selector: selector!("call_with_panic"),
        };

        let err = execute_call_inner(req.clone(), &mut state, ctx.clone(), u64::MAX).unwrap_err();
        if let EntryPointExecutionError::ExecutionFailed { error_trace } = err {
            assert!(error_trace.to_string().contains("fail"));
        }
    }
}
