#[starknet::contract]
pub mod CallTest {
    #[storage]
    struct Storage { }

    #[external(v0)]
    fn bounded_call(self: @ContractState, iterations: u64) {
        let mut i = 0;
        loop {
            if i >= iterations {
                break;
            }
            i += 1;
        }
    }

    #[external(v0)]
    fn call_with_panic(self: @ContractState) {
	    assert(1 != 1, 'fail');
    }
}
