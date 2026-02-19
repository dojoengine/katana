use cainome_cairo_serde::CairoSerde;
use katana_primitives::Felt;
use katana_rpc_types::outside_execution::Call;
use starknet::macros::selector;

/// Encodes the given calls into a vector of Felt values (New encoding, cairo 1),
/// since controller accounts are Cairo 1 contracts.
pub fn encode_calls(calls: Vec<Call>) -> Vec<Felt> {
    Vec::<Call>::cairo_serialize(&calls)
}

pub fn request_random_call(
    calls: &[Call],
) -> Option<(katana_rpc_types::outside_execution::Call, usize)> {
    calls
        .iter()
        .position(|call| call.selector == selector!("request_random"))
        .map(|position| (calls[position].clone(), position))
}
