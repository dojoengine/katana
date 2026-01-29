use stark_vrf::ecvrf::{ECVRFImpl, Point, Proof};
use starknet::ContractAddress;
use crate::{PublicKey, Source};

#[starknet::interface]
pub trait IVrfProvider<TContractState> {
    fn request_random(self: @TContractState, caller: ContractAddress, source: Source);
    fn submit_random(ref self: TContractState, seed: felt252, proof: Proof);
    fn consume_random(ref self: TContractState, source: Source) -> felt252;
    fn assert_consumed(ref self: TContractState, seed: felt252);

    fn get_consume_count(self: @TContractState) -> u32;
    fn is_vrf_call(self: @TContractState) -> bool;

    fn get_public_key(self: @TContractState) -> PublicKey;
    fn set_public_key(ref self: TContractState, new_pubkey: PublicKey);
}


#[starknet::component]
pub mod VrfProviderComponent {
    use core::poseidon::poseidon_hash_span;
    use openzeppelin::access::ownable::OwnableComponent;
    use openzeppelin::access::ownable::OwnableComponent::InternalImpl as OwnableInternalImpl;
    use starknet::storage::{
        Map, StorageMapReadAccess, StorageMapWriteAccess, StoragePointerReadAccess,
        StoragePointerWriteAccess,
    };
    use starknet::{ContractAddress, TxInfo, get_caller_address};
    use super::*;

    #[storage]
    pub struct Storage {
        VrfProvider_pubkey: PublicKey,
        // wallet -> nonce
        VrfProvider_nonces: Map<ContractAddress, felt252>,
        // seed -> random
        VrfProvider_random: Map<felt252, felt252>,
        // seed -> consume_random call count
        VrfProvider_consume_count: Option<u32>,
    }

    #[derive(Drop, starknet::Event)]
    struct PublicKeyChanged {
        pubkey: PublicKey,
    }

    #[derive(Drop, starknet::Event)]
    struct SubmitRandom {
        #[key]
        seed: felt252,
        proof: Proof,
    }

    #[derive(Drop, starknet::Event)]
    #[event]
    pub enum Event {
        PublicKeyChanged: PublicKeyChanged,
        SubmitRandom: SubmitRandom,
    }

    pub mod Errors {
        pub const PUBKEY_ZERO: felt252 = 'VrfProvider: pubkey is zero';
        pub const INVALID_PROOF: felt252 = 'VrfProvider: invalid proof';
        pub const NOT_FULFILLED: felt252 = 'VrfProvider: not fulfilled';
        pub const NOT_CONSUMED: felt252 = 'VrfProvider: not consumed';
    }

    #[embeddable_as(VrfProviderImpl)]
    impl VrfProvider<
        TContractState,
        +Drop<TContractState>,
        +HasComponent<TContractState>,
        impl Owner: OwnableComponent::HasComponent<TContractState>,
    > of super::IVrfProvider<ComponentState<TContractState>> {
        fn request_random(
            self: @ComponentState<TContractState>, caller: ContractAddress, source: Source,
        ) {}

        fn submit_random(ref self: ComponentState<TContractState>, seed: felt252, proof: Proof) {
            let pubkey: Point = self.get_public_key().into();
            let ecvrf = ECVRFImpl::new(pubkey);

            let random = ecvrf
                .verify(proof.clone(), array![seed].span())
                .expect(Errors::INVALID_PROOF);

            self.VrfProvider_random.write(seed, random);
            self.VrfProvider_consume_count.write(Option::Some(0));

            self.emit(SubmitRandom { seed, proof });
        }

        fn consume_random(ref self: ComponentState<TContractState>, source: Source) -> felt252 {
            let tx_info = starknet::get_execution_info().tx_info.unbox();

            let seed = self.get_seed(source, tx_info);
            let consume_count = self.get_consume_count();

            // Always return 0 during fee estimation to avoid leaking vrf info.
            if tx_info.max_fee == 0
                && *tx_info.resource_bounds.at(0).max_amount == 0
                && *tx_info.resource_bounds.at(1).max_amount == 0
                && *tx_info.resource_bounds.at(2).max_amount == 0 {
                // simulate called
                self.VrfProvider_consume_count.write(Option::Some(consume_count + 1));

                return 0;
            }

            let random = self.VrfProvider_random.read(seed);
            assert(random != 0, Errors::NOT_FULFILLED);

            self.VrfProvider_consume_count.write(Option::Some(consume_count + 1));

            poseidon_hash_span(array![random, consume_count.into()].span())
        }

        fn get_consume_count(self: @ComponentState<TContractState>) -> u32 {
            self._get_consume_count()
        }

        fn is_vrf_call(self: @ComponentState<TContractState>) -> bool {
            self.VrfProvider_consume_count.read().is_some()
        }

        fn assert_consumed(ref self: ComponentState<TContractState>, seed: felt252) {
            let consume_count = self.get_consume_count();
            assert(consume_count > 0, Errors::NOT_CONSUMED);

            self.VrfProvider_random.write(seed, 0);
            self.VrfProvider_consume_count.write(Option::None);
        }

        fn get_public_key(self: @ComponentState<TContractState>) -> PublicKey {
            self.VrfProvider_pubkey.read()
        }

        fn set_public_key(ref self: ComponentState<TContractState>, new_pubkey: PublicKey) {
            let mut ownable_component = get_dep_component_mut!(ref self, Owner);
            ownable_component.assert_only_owner();

            self._set_public_key(new_pubkey);
        }
    }

    #[generate_trait]
    pub impl InternalImpl<
        TContractState, +HasComponent<TContractState>,
    > of InternalTrait<TContractState> {
        fn initializer(ref self: ComponentState<TContractState>, pubkey: PublicKey) {
            self._set_public_key(pubkey);
        }

        fn _set_public_key(ref self: ComponentState<TContractState>, new_pubkey: PublicKey) {
            assert(new_pubkey.x != 0 && new_pubkey.y != 0, Errors::PUBKEY_ZERO);
            self.VrfProvider_pubkey.write(new_pubkey);

            self.emit(PublicKeyChanged { pubkey: new_pubkey })
        }

        fn _get_consume_count(self: @ComponentState<TContractState>) -> u32 {
            let count = self.VrfProvider_consume_count.read();
            count.unwrap_or(0)
        }

        fn get_seed(
            ref self: ComponentState<TContractState>, source: Source, tx_info: TxInfo,
        ) -> felt252 {
            let caller = get_caller_address();

            match source {
                Source::Nonce(addr) => {
                    let consume_count = self._get_consume_count();
                    // let nonce = self.VrfProvider_nonces.read(addr);
                    let nonce = if consume_count == 0 {
                        // only increment nonce on first consume_random
                        let nonce = self.VrfProvider_nonces.read(addr);
                        self.VrfProvider_nonces.write(addr, nonce + 1);
                        nonce
                    } else {
                        // return the nonce pre-incrementation
                        let nonce = self.VrfProvider_nonces.read(addr);
                        nonce - 1
                    };
                    poseidon_hash_span(
                        array![nonce, addr.into(), caller.into(), tx_info.chain_id].span(),
                    )
                },
                Source::Salt(salt) => {
                    poseidon_hash_span(array![salt, caller.into(), tx_info.chain_id].span())
                },
            }
        }
    }
}
