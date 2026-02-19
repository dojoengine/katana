use katana_primitives::execution::Call;
use katana_primitives::Felt;
use starknet::macros::selector;

pub fn find_request_rand_call(calls: &[Call]) -> Option<(Call, usize)> {
    calls
        .iter()
        .position(|call| call.entry_point_selector == selector!("request_random"))
        .map(|position| (calls[position].clone(), position))
}
