use cainome::cairo_serde::CairoSerde;
use katana_primitives::{ContractAddress, Felt};
use starknet::macros::selector;

use crate::rpc::types::{Call, OutsideExecution};

/// Encodes the given calls into a vector of Felt values (New encoding, cairo 1),
/// since controller accounts are Cairo 1 contracts.
pub fn encode_calls(calls: Vec<Call>) -> Vec<Felt> {
    let mut execute_calldata: Vec<Felt> = vec![calls.len().into()];
    for call in calls {
        execute_calldata.push(call.to.into());
        execute_calldata.push(call.selector);

        execute_calldata.push(call.calldata.len().into());
        execute_calldata.extend_from_slice(&call.calldata);
    }

    execute_calldata
}

/// Build the `execute_from_outside` call from the given outside execution and signature.
pub fn get_execute_from_outside_call(
    address: ContractAddress,
    outside_execution: OutsideExecution,
    signature: Vec<Felt>,
) -> Call {
    let entrypoint = match outside_execution {
        OutsideExecution::V2(_) => selector!("execute_from_outside_v2"),
        OutsideExecution::V3(_) => selector!("execute_from_outside_v3"),
    };

    let mut inner_calldata = OutsideExecution::cairo_serialize(&outside_execution);
    inner_calldata.extend(Vec::<Felt>::cairo_serialize(&signature));

    Call { to: address, selector: entrypoint, calldata: inner_calldata }
}
