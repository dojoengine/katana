#[starknet::interface]
pub trait IVrfConsumerMock<TContractState> {
    fn dice(ref self: TContractState) -> u8;
    fn dice_with_salt(ref self: TContractState) -> u8;

    fn not_consuming(ref self: TContractState);

    fn get_dice_value(self: @TContractState) -> u8;

    // admin
    fn set_vrf_provider(ref self: TContractState, new_vrf_provider: starknet::ContractAddress);
}


#[starknet::contract]
pub mod VrfConsumer {
    use cartridge_vrf::Source;
    use cartridge_vrf::vrf_consumer::vrf_consumer_component::VrfConsumerComponent;
    use stark_vrf::ecvrf::ECVRFImpl;
    use starknet::storage::{StoragePointerReadAccess, StoragePointerWriteAccess};
    use starknet::{ContractAddress, get_caller_address};

    component!(path: VrfConsumerComponent, storage: vrf_consumer, event: VrfConsumerEvent);

    #[abi(embed_v0)]
    impl VrfConsumerImpl = VrfConsumerComponent::VrfConsumerImpl<ContractState>;

    impl VrfConsumerInternalImpl = VrfConsumerComponent::InternalImpl<ContractState>;

    #[storage]
    pub struct Storage {
        #[substorage(v0)]
        vrf_consumer: VrfConsumerComponent::Storage,
        dice_value: u8,
    }

    #[event]
    #[derive(Drop, starknet::Event)]
    pub enum Event {
        #[flat]
        VrfConsumerEvent: VrfConsumerComponent::Event,
    }

    #[constructor]
    fn constructor(ref self: ContractState, vrf_provider: ContractAddress) {
        self.vrf_consumer.initializer(vrf_provider);
    }

    #[abi(embed_v0)]
    impl ConsumerImpl of super::IVrfConsumerMock<ContractState> {
        fn get_dice_value(self: @ContractState) -> u8 {
            self.dice_value.read()
        }


        fn dice(ref self: ContractState) -> u8 {
            let player_id = get_caller_address();
            let random: u256 = self.vrf_consumer.consume_random(Source::Nonce(player_id)).into();

            let dice_value = ((random % 6) + 1).try_into().unwrap();
            self.dice_value.write(dice_value);
            println!("dice_value: {}", dice_value);

            dice_value
        }

        fn dice_with_salt(ref self: ContractState) -> u8 {
            let random: u256 = self.vrf_consumer.consume_random(Source::Salt('salt')).into();

            let dice_value = ((random % 6) + 1).try_into().unwrap();
            self.dice_value.write(dice_value);
            println!("dice_value: {}", dice_value);

            dice_value
        }

        fn not_consuming(ref self: ContractState) {
            let _player_id = get_caller_address();
            // do the nothing
        }

        fn set_vrf_provider(ref self: ContractState, new_vrf_provider: ContractAddress) {
            // should be restricted
            self.vrf_consumer.set_vrf_provider(new_vrf_provider);
        }
    }

    #[generate_trait]
    impl ConsumerInternal of InternalTrait {}
}
