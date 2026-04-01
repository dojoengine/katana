use starknet::ContractAddress;

#[starknet::interface]
pub trait ISimple<T> {
    fn roll_dice_with_nonce(ref self: T);
    fn roll_dice_with_salt(ref self: T);
    fn get_value(self: @T) -> felt252;
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
    }

    const VRF_PROVIDER_ADDRESS: starknet::ContractAddress =
        0x15f542e25a4ce31481f986888c179b6e57412be340b8095f72f75a328fbb27b
        .try_into()
        .unwrap();

    #[abi(embed_v0)]
    impl SimpleImpl of super::ISimple<ContractState> {
        fn get_value(self: @ContractState) -> felt252 {
            self.value.read()
        }

        fn roll_dice_with_nonce(ref self: ContractState) {
            let vrf_provider = super::IVrfProviderDispatcher {
                contract_address: VRF_PROVIDER_ADDRESS,
            };

            let player_id = get_caller_address();

            let value = vrf_provider.consume_random(Source::Nonce(player_id));
            self.value.write(value);
        }

        fn roll_dice_with_salt(ref self: ContractState) {
            let vrf_provider = super::IVrfProviderDispatcher {
                contract_address: VRF_PROVIDER_ADDRESS,
            };

            let value = vrf_provider.consume_random(Source::Salt(42));
            self.value.write(value);
        }
    }
}
