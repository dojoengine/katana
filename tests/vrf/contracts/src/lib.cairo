use starknet::ContractAddress;

#[starknet::interface]
pub trait ISimple<T> {
    fn get(self: @T) -> felt252;
    fn set_with_nonce(ref self: T);
    fn set_with_salt(ref self: T);
}

#[starknet::interface]
trait IVrfProvider<T> {
    fn request_random(self: @T, caller: ContractAddress, source: Source);
    fn consume_random(ref self: T, source: Source) -> felt252;
}

#[derive(Drop, Copy, Clone, Serde)]
pub enum Source {
    Nonce: ContractAddress,
    Salt: felt252,
}

#[starknet::contract]
mod Simple {
    use starknet::get_caller_address;
    use starknet::storage::{StoragePointerReadAccess, StoragePointerWriteAccess};
    use super::*;


    #[storage]
    struct Storage {
        value: felt252,
        vrf_provider_address: ContractAddress,
    }

    #[constructor]
    fn constructor(ref self: ContractState, vrf_provider: ContractAddress) {
        self.vrf_provider_address.write(vrf_provider);
    }

    #[abi(embed_v0)]
    impl SimpleImpl of super::ISimple<ContractState> {
        fn get(self: @ContractState) -> felt252 {
            self.value.read()
        }

        fn set_with_nonce(ref self: ContractState) {
            let vrf_provider = super::IVrfProviderDispatcher {
                contract_address: self.vrf_provider_address.read(),
            };

            let player_id = get_caller_address();

            let value = vrf_provider.consume_random(Source::Nonce(player_id));
            self.value.write(value);
        }

        fn set_with_salt(ref self: ContractState) {
            let vrf_provider = super::IVrfProviderDispatcher {
                contract_address: self.vrf_provider_address.read(),
            };

            let value = vrf_provider.consume_random(Source::Salt(42));
            self.value.write(value);
        }
    }
}
