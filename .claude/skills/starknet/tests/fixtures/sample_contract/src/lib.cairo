#[starknet::interface]
pub trait ISampleContract<TContractState> {
    fn get_value(self: @TContractState) -> felt252;
    fn set_value(ref self: TContractState, value: felt252);
}

#[starknet::contract]
pub mod SampleContract {
    use starknet::storage::*;

    #[storage]
    pub struct Storage {
        value: felt252,
    }

    #[abi(embed_v0)]
    pub impl SampleContractImpl of super::ISampleContract<ContractState> {
        fn get_value(self: @ContractState) -> felt252 {
            self.value.read()
        }

        fn set_value(ref self: ContractState, value: felt252) {
            self.value.write(value);
        }
    }
}
