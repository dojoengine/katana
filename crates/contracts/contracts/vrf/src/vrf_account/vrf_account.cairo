// SPDX-License-Identifier: MIT
// Compatible with OpenZeppelin Contracts for Cairo ^2.0.0

#[starknet::contract(account)]
mod VrfAccount {
    use openzeppelin::account::extensions::SRC9Component;
    use openzeppelin::introspection::src5::SRC5Component;
    use openzeppelin::upgrades::UpgradeableComponent;
    use openzeppelin::upgrades::interface::{IUpgradeAndCall, IUpgradeable};
    use starknet::ClassHash;
    use crate::vrf_account::vrf_account_component::VrfAccountComponent;

    component!(path: VrfAccountComponent, storage: vrf_provider, event: VrfProviderEvent);
    component!(path: SRC5Component, storage: src5, event: SRC5Event);
    component!(path: SRC9Component, storage: src9, event: SRC9Event);
    component!(path: UpgradeableComponent, storage: upgradeable, event: UpgradeableEvent);

    // External
    #[abi(embed_v0)]
    impl AccountMixinImpl = VrfAccountComponent::AccountMixinImpl<ContractState>;
    #[abi(embed_v0)]
    impl VrfAccountImpl = VrfAccountComponent::VrfAccountImpl<ContractState>;

    #[abi(embed_v0)]
    impl OutsideExecutionV2Impl =
        SRC9Component::OutsideExecutionV2Impl<ContractState>;

    // Internal
    impl AccountInternalImpl = VrfAccountComponent::InternalImpl<ContractState>;
    impl OutsideExecutionInternalImpl = SRC9Component::InternalImpl<ContractState>;
    impl UpgradeableInternalImpl = UpgradeableComponent::InternalImpl<ContractState>;

    #[storage]
    struct Storage {
        #[substorage(v0)]
        src5: SRC5Component::Storage,
        #[substorage(v0)]
        src9: SRC9Component::Storage,
        #[substorage(v0)]
        upgradeable: UpgradeableComponent::Storage,
        #[substorage(v0)]
        vrf_provider: VrfAccountComponent::Storage,
    }

    #[event]
    #[derive(Drop, starknet::Event)]
    enum Event {
        #[flat]
        SRC5Event: SRC5Component::Event,
        #[flat]
        SRC9Event: SRC9Component::Event,
        #[flat]
        UpgradeableEvent: UpgradeableComponent::Event,
        #[flat]
        VrfProviderEvent: VrfAccountComponent::Event,
    }

    #[constructor]
    fn constructor(ref self: ContractState, public_key: felt252) {
        self.vrf_provider.initializer(public_key);
        self.src9.initializer();
    }


    //
    // Initializer
    //

    #[generate_trait]
    #[abi(per_item)]
    impl ExternalImpl of ExternalTrait {
        #[external(v0)]
        fn initializer(ref self: ContractState, public_key: felt252) {
            self.vrf_provider.assert_only_self();

            self.vrf_provider.initializer(public_key);
            self.src9.initializer();
        }
    }

    //
    // Upgradeable
    //

    #[abi(embed_v0)]
    impl UpgradeableImpl of IUpgradeable<ContractState> {
        fn upgrade(ref self: ContractState, new_class_hash: ClassHash) {
            self.vrf_provider.assert_only_self();
            self.upgradeable.upgrade(new_class_hash);
        }
    }

    #[abi(embed_v0)]
    impl UpgradeAndCallImpl of IUpgradeAndCall<ContractState> {
        fn upgrade_and_call(
            ref self: ContractState,
            new_class_hash: ClassHash,
            selector: felt252,
            calldata: Span<felt252>,
        ) -> Span<felt252> {
            self.vrf_provider.assert_only_self();
            self.upgradeable.upgrade_and_call(new_class_hash, selector, calldata)
        }
    }
}
