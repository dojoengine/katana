// SPDX-License-Identifier: MIT
// OpenZeppelin Contracts for Cairo v2.0.0 (account/src/account.cairo)

use stark_vrf::ecvrf::{ECVRFImpl, Point, Proof};
use starknet::ContractAddress;
use starknet::account::Call;
use crate::{PublicKey, Source};


#[starknet::interface]
pub trait IVrfAccount<TContractState> {
    fn request_random(self: @TContractState, caller: ContractAddress, source: Source);
    fn submit_random(ref self: TContractState, seed: felt252, proof: Proof);
    fn consume_random(ref self: TContractState, source: Source) -> felt252;

    fn get_consume_count(self: @TContractState) -> u32;
    fn is_vrf_call(self: @TContractState) -> bool;

    fn get_vrf_public_key(self: @TContractState) -> PublicKey;
    fn set_vrf_public_key(ref self: TContractState, new_pubkey: PublicKey);
}

#[starknet::interface]
pub trait ISRC6Mutable<TState> {
    fn __execute__(ref self: TState, calls: Array<Call>);
    fn __validate__(ref self: TState, calls: Array<Call>) -> felt252;
    fn is_valid_signature(self: @TState, hash: felt252, signature: Array<felt252>) -> felt252;
}

#[starknet::interface]
pub trait AccountABIMutable<TState> {
    // ISRC6
    fn __execute__(ref self: TState, calls: Array<Call>);
    fn __validate__(ref self: TState, calls: Array<Call>) -> felt252;
    fn is_valid_signature(self: @TState, hash: felt252, signature: Array<felt252>) -> felt252;

    // ISRC5
    fn supports_interface(self: @TState, interface_id: felt252) -> bool;

    // IDeclarer
    fn __validate_declare__(self: @TState, class_hash: felt252) -> felt252;

    // IDeployable
    fn __validate_deploy__(
        self: @TState, class_hash: felt252, contract_address_salt: felt252, public_key: felt252,
    ) -> felt252;

    // IPublicKey
    fn get_public_key(self: @TState) -> felt252;
    fn set_public_key(ref self: TState, new_public_key: felt252, signature: Span<felt252>);
}

pub const SUBMIT_RANDOM: felt252 = selector!("submit_random");

/// # Account Component
///
/// The Account component enables contracts to behave as accounts.
///
///

#[starknet::component]
pub mod VrfAccountComponent {
    use core::hash::{HashStateExTrait, HashStateTrait};
    use core::num::traits::Zero;
    use core::poseidon::{PoseidonTrait, poseidon_hash_span};
    use openzeppelin::account::interface;
    use openzeppelin::account::utils::{
        execute_single_call, is_tx_version_valid, is_valid_stark_signature,
    };
    use openzeppelin::introspection::src5::SRC5Component;
    use openzeppelin::introspection::src5::SRC5Component::{
        InternalTrait as SRC5InternalTrait, SRC5Impl,
    };
    use stark_vrf::Proof;
    use starknet::account::Call;
    use starknet::storage::{
        Map, StorageMapReadAccess, StorageMapWriteAccess, StoragePointerReadAccess,
        StoragePointerWriteAccess,
    };
    use starknet::{get_caller_address, get_contract_address};
    use super::*;

    #[storage]
    pub struct Storage {
        // vrf public key
        pub VrfProvider_pubkey: PublicKey,
        // wallet -> nonce
        pub VrfProvider_nonces: Map<ContractAddress, felt252>,
        // seed -> random
        pub VrfProvider_random: Map<felt252, felt252>,
        // seed -> consume_random call count
        pub VrfProvider_consume_count: Option<u32>,
        // account public key
        pub Account_public_key: felt252,
    }

    #[event]
    #[derive(Drop, starknet::Event)]
    pub enum Event {
        OwnerAdded: OwnerAdded,
        OwnerRemoved: OwnerRemoved,
        SubmitRandom: SubmitRandom,
    }

    #[derive(Drop, Debug, PartialEq, starknet::Event)]
    pub struct OwnerAdded {
        #[key]
        pub new_owner_guid: felt252,
    }

    #[derive(Drop, Debug, PartialEq, starknet::Event)]
    pub struct OwnerRemoved {
        #[key]
        pub removed_owner_guid: felt252,
    }

    #[derive(Drop, starknet::Event)]
    struct SubmitRandom {
        #[key]
        seed: felt252,
        proof: Proof,
    }

    pub mod Errors {
        pub const INVALID_CALLER: felt252 = 'Account: invalid caller';
        pub const INVALID_SIGNATURE: felt252 = 'Account: invalid signature';
        pub const INVALID_TX_VERSION: felt252 = 'Account: invalid tx version';
        pub const UNAUTHORIZED: felt252 = 'Account: unauthorized';
    }

    pub mod VrfErrors {
        pub const PUBKEY_ZERO: felt252 = 'VrfProvider: pubkey is zero';
        pub const INVALID_PROOF: felt252 = 'VrfProvider: invalid proof';
        pub const NOT_FULFILLED: felt252 = 'VrfProvider: not fulfilled';
        pub const NOT_CONSUMED: felt252 = 'VrfProvider: not consumed';
    }

    //
    // VRF
    //

    #[embeddable_as(VrfAccountImpl)]
    impl VrfAccount<
        TContractState,
        +HasComponent<TContractState>,
        +SRC5Component::HasComponent<TContractState>,
        +Drop<TContractState>,
    > of IVrfAccount<ComponentState<TContractState>> {
        fn request_random(
            self: @ComponentState<TContractState>, caller: ContractAddress, source: Source,
        ) { // noop
        }

        fn submit_random(ref self: ComponentState<TContractState>, seed: felt252, proof: Proof) {
            let pubkey: Point = self.get_vrf_public_key().into();
            let ecvrf = ECVRFImpl::new(pubkey);

            let random = ecvrf
                .verify(proof.clone(), array![seed].span())
                .expect(VrfErrors::INVALID_PROOF);

            self.VrfProvider_random.write(seed, random);
            self.VrfProvider_consume_count.write(Option::Some(0));

            self.emit(SubmitRandom { seed, proof });
        }

        fn consume_random(ref self: ComponentState<TContractState>, source: Source) -> felt252 {
            let tx_info = starknet::get_execution_info().tx_info.unbox();

            let seed = self._get_seed(source);
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
            assert(random != 0, VrfErrors::NOT_FULFILLED);

            self.VrfProvider_consume_count.write(Option::Some(consume_count + 1));

            poseidon_hash_span(array![random, consume_count.into()].span())
        }

        //
        //
        //

        fn get_consume_count(self: @ComponentState<TContractState>) -> u32 {
            self._get_consume_count()
        }

        fn is_vrf_call(self: @ComponentState<TContractState>) -> bool {
            self.VrfProvider_consume_count.read().is_some()
        }

        //
        //
        //

        fn get_vrf_public_key(self: @ComponentState<TContractState>) -> PublicKey {
            self.VrfProvider_pubkey.read()
        }

        fn set_vrf_public_key(ref self: ComponentState<TContractState>, new_pubkey: PublicKey) {
            self.assert_only_self();
            self.VrfProvider_pubkey.write(new_pubkey);
        }
    }

    //
    // VRF Internal
    //

    #[generate_trait]
    pub impl VrfInternalImpl<
        TContractState,
        +HasComponent<TContractState>,
        impl SRC5: SRC5Component::HasComponent<TContractState>,
        +Drop<TContractState>,
    > of VrfInternalTrait<TContractState> {
        fn _assert_consumed(ref self: ComponentState<TContractState>, seed: felt252) {
            let consume_count = self._get_consume_count();
            assert(consume_count > 0, VrfErrors::NOT_CONSUMED);

            self.VrfProvider_random.write(seed, 0);
            self.VrfProvider_consume_count.write(Option::None);
        }

        fn _get_consume_count(self: @ComponentState<TContractState>) -> u32 {
            let count = self.VrfProvider_consume_count.read();
            count.unwrap_or(0)
        }

        fn _get_seed(ref self: ComponentState<TContractState>, source: Source) -> felt252 {
            let tx_info = starknet::get_execution_info().tx_info.unbox();
            let caller = get_caller_address();

            match source {
                Source::Nonce(addr) => {
                    let consume_count = self._get_consume_count();
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

        fn _is_submit_random_call(ref self: ComponentState<TContractState>, call: @Call) -> bool {
            let this = get_contract_address();
            (*call.to == this) && (*call.selector == SUBMIT_RANDOM)
        }
    }

    //
    // External
    //

    #[embeddable_as(SRC6Impl)]
    impl SRC6<
        TContractState,
        +HasComponent<TContractState>,
        +SRC5Component::HasComponent<TContractState>,
        +Drop<TContractState>,
    > of ISRC6Mutable<ComponentState<TContractState>> {
        /// Executes a list of calls from the account.
        ///
        /// Requirements:
        ///
        /// - The transaction version must be greater than or equal to `MIN_TRANSACTION_VERSION`.
        /// - If the transaction is a simulation (version >= `QUERY_OFFSET`), it must be
        /// greater than or equal to `QUERY_OFFSET` + `MIN_TRANSACTION_VERSION`.
        fn __execute__(ref self: ComponentState<TContractState>, calls: Array<Call>) {
            // Avoid calls from other contracts
            // https://github.com/OpenZeppelin/cairo-contracts/issues/344
            let sender = starknet::get_caller_address();
            assert(sender.is_zero(), Errors::INVALID_CALLER);
            assert(is_tx_version_valid(), Errors::INVALID_TX_VERSION);

            let mut should_assert_consumed_seed = Option::None;
            for call in calls.span() {
                if self._is_submit_random_call(call) {
                    should_assert_consumed_seed = Option::Some(call.calldata.at(0));
                }
                execute_single_call(call);
            }

            if should_assert_consumed_seed.is_some() {
                let seed = *should_assert_consumed_seed.unwrap();
                self._assert_consumed(seed);
            }
        }

        /// Verifies the validity of the signature for the current transaction.
        /// This function is used by the protocol to verify `invoke` transactions.
        fn __validate__(ref self: ComponentState<TContractState>, calls: Array<Call>) -> felt252 {
            self.validate_transaction()
        }


        /// Verifies that the given signature is valid for the given hash.
        fn is_valid_signature(
            self: @ComponentState<TContractState>, hash: felt252, signature: Array<felt252>,
        ) -> felt252 {
            if self._is_valid_signature(hash, signature.span()) {
                starknet::VALIDATED
            } else {
                0
            }
        }
    }

    #[embeddable_as(DeclarerImpl)]
    impl Declarer<
        TContractState,
        +HasComponent<TContractState>,
        +SRC5Component::HasComponent<TContractState>,
        +Drop<TContractState>,
    > of interface::IDeclarer<ComponentState<TContractState>> {
        /// Verifies the validity of the signature for the current transaction.
        /// This function is used by the protocol to verify `declare` transactions.
        fn __validate_declare__(
            self: @ComponentState<TContractState>, class_hash: felt252,
        ) -> felt252 {
            self.validate_transaction()
        }
    }

    #[embeddable_as(DeployableImpl)]
    impl Deployable<
        TContractState,
        +HasComponent<TContractState>,
        +SRC5Component::HasComponent<TContractState>,
        +Drop<TContractState>,
    > of interface::IDeployable<ComponentState<TContractState>> {
        /// Verifies the validity of the signature for the current transaction.
        /// This function is used by the protocol to verify `deploy_account` transactions.
        fn __validate_deploy__(
            self: @ComponentState<TContractState>,
            class_hash: felt252,
            contract_address_salt: felt252,
            public_key: felt252,
        ) -> felt252 {
            self.validate_transaction()
        }
    }

    #[embeddable_as(PublicKeyImpl)]
    impl ImplPublicKey<
        TContractState,
        +HasComponent<TContractState>,
        +SRC5Component::HasComponent<TContractState>,
        +Drop<TContractState>,
    > of interface::IPublicKey<ComponentState<TContractState>> {
        /// Returns the current public key of the account.
        fn get_public_key(self: @ComponentState<TContractState>) -> felt252 {
            self.Account_public_key.read()
        }

        /// Sets the public key of the account to `new_public_key`.
        ///
        /// Requirements:
        ///
        /// - The caller must be the contract itself.
        /// - The signature must be valid for the new owner.
        ///
        /// Emits both an `OwnerRemoved` and an `OwnerAdded` event.
        fn set_public_key(
            ref self: ComponentState<TContractState>,
            new_public_key: felt252,
            signature: Span<felt252>,
        ) {
            self.assert_only_self();

            let current_owner = self.Account_public_key.read();
            self.assert_valid_new_owner(current_owner, new_public_key, signature);

            self.emit(OwnerRemoved { removed_owner_guid: current_owner });
            self._set_public_key(new_public_key);
        }
    }

    #[embeddable_as(AccountMixinImpl)]
    impl AccountMixin<
        TContractState,
        +HasComponent<TContractState>,
        impl SRC5: SRC5Component::HasComponent<TContractState>,
        +Drop<TContractState>,
    > of AccountABIMutable<ComponentState<TContractState>> {
        // ISRC6
        fn __execute__(ref self: ComponentState<TContractState>, calls: Array<Call>) {
            SRC6::__execute__(ref self, calls)
        }

        fn __validate__(ref self: ComponentState<TContractState>, calls: Array<Call>) -> felt252 {
            SRC6::__validate__(ref self, calls)
        }

        fn is_valid_signature(
            self: @ComponentState<TContractState>, hash: felt252, signature: Array<felt252>,
        ) -> felt252 {
            SRC6::is_valid_signature(self, hash, signature)
        }

        // IDeclarer
        fn __validate_declare__(
            self: @ComponentState<TContractState>, class_hash: felt252,
        ) -> felt252 {
            Declarer::__validate_declare__(self, class_hash)
        }

        // IDeployable
        fn __validate_deploy__(
            self: @ComponentState<TContractState>,
            class_hash: felt252,
            contract_address_salt: felt252,
            public_key: felt252,
        ) -> felt252 {
            Deployable::__validate_deploy__(self, class_hash, contract_address_salt, public_key)
        }

        // IPublicKey
        fn get_public_key(self: @ComponentState<TContractState>) -> felt252 {
            ImplPublicKey::get_public_key(self)
        }

        fn set_public_key(
            ref self: ComponentState<TContractState>,
            new_public_key: felt252,
            signature: Span<felt252>,
        ) {
            ImplPublicKey::set_public_key(ref self, new_public_key, signature);
        }

        // ISRC5
        fn supports_interface(
            self: @ComponentState<TContractState>, interface_id: felt252,
        ) -> bool {
            let src5 = get_dep_component!(self, SRC5);
            src5.supports_interface(interface_id)
        }
    }

    //
    // Internal
    //

    #[generate_trait]
    pub impl InternalImpl<
        TContractState,
        +HasComponent<TContractState>,
        impl SRC5: SRC5Component::HasComponent<TContractState>,
        +Drop<TContractState>,
    > of InternalTrait<TContractState> {
        /// Initializes the account with the given public key, and registers the ISRC6 interface ID.
        ///
        /// Emits an `OwnerAdded` event.
        fn initializer(ref self: ComponentState<TContractState>, public_key: felt252) {
            let mut src5_component = get_dep_component_mut!(ref self, SRC5);
            src5_component.register_interface(interface::ISRC6_ID);
            self._set_public_key(public_key);
        }

        /// Validates that the caller is the account itself. Otherwise it reverts.
        fn assert_only_self(self: @ComponentState<TContractState>) {
            let caller = starknet::get_caller_address();
            let self = starknet::get_contract_address();
            assert(self == caller, Errors::UNAUTHORIZED);
        }

        /// Validates that `new_owner` accepted the ownership of the contract.
        ///
        /// WARNING: This function assumes that `current_owner` is the current owner of the
        /// contract, and does not validate this assumption.
        ///
        /// Requirements:
        ///
        /// - The signature must be valid for the new owner.
        fn assert_valid_new_owner(
            self: @ComponentState<TContractState>,
            current_owner: felt252,
            new_owner: felt252,
            signature: Span<felt252>,
        ) {
            let message_hash = PoseidonTrait::new()
                .update_with('StarkNet Message')
                .update_with('accept_ownership')
                .update_with(starknet::get_contract_address())
                .update_with(current_owner)
                .finalize();

            let is_valid = is_valid_stark_signature(message_hash, new_owner, signature);
            assert(is_valid, Errors::INVALID_SIGNATURE);
        }

        /// Validates the signature for the current transaction.
        /// Returns the short string `VALID` if valid, otherwise it reverts.
        fn validate_transaction(self: @ComponentState<TContractState>) -> felt252 {
            let tx_info = starknet::get_tx_info().unbox();
            let tx_hash = tx_info.transaction_hash;
            let signature = tx_info.signature;

            assert(self._is_valid_signature(tx_hash, signature), Errors::INVALID_SIGNATURE);
            starknet::VALIDATED
        }

        /// Sets the public key without validating the caller.
        /// The usage of this method outside the `set_public_key` function is discouraged.
        ///
        /// Emits an `OwnerAdded` event.
        fn _set_public_key(ref self: ComponentState<TContractState>, new_public_key: felt252) {
            self.Account_public_key.write(new_public_key);
            self.emit(OwnerAdded { new_owner_guid: new_public_key });
        }

        /// Returns whether the given signature is valid for the given hash
        /// using the account's current public key.
        fn _is_valid_signature(
            self: @ComponentState<TContractState>, hash: felt252, signature: Span<felt252>,
        ) -> bool {
            let public_key = self.Account_public_key.read();
            is_valid_stark_signature(hash, public_key, signature)
        }
    }
}
