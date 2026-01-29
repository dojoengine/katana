// SPDX-License-Identifier: MIT
// Compatible with OpenZeppelin Contracts for Cairo ^0.16.0

#[starknet::contract]
pub mod VrfProvider {
    use cartridge_vrf::vrf_provider::vrf_provider_component::VrfProviderComponent;
    use openzeppelin::access::ownable::OwnableComponent;
    use openzeppelin::upgrades::UpgradeableComponent;
    use openzeppelin::upgrades::interface::IUpgradeable;
    use starknet::{ClassHash, ContractAddress};
    use crate::PublicKey;

    component!(path: OwnableComponent, storage: ownable, event: OwnableEvent);
    component!(path: UpgradeableComponent, storage: upgradeable, event: UpgradeableEvent);
    component!(path: VrfProviderComponent, storage: vrf_provider, event: VrfProviderEvent);

    #[abi(embed_v0)]
    impl OwnableMixinImpl = OwnableComponent::OwnableMixinImpl<ContractState>;

    #[abi(embed_v0)]
    impl VrfProviderImpl = VrfProviderComponent::VrfProviderImpl<ContractState>;

    impl OwnableInternalImpl = OwnableComponent::InternalImpl<ContractState>;
    impl UpgradeableInternalImpl = UpgradeableComponent::InternalImpl<ContractState>;
    impl VrfProviderInternalImpl = VrfProviderComponent::InternalImpl<ContractState>;

    #[storage]
    pub struct Storage {
        #[substorage(v0)]
        ownable: OwnableComponent::Storage,
        #[substorage(v0)]
        upgradeable: UpgradeableComponent::Storage,
        #[substorage(v0)]
        vrf_provider: VrfProviderComponent::Storage,
    }

    #[event]
    #[derive(Drop, starknet::Event)]
    pub enum Event {
        #[flat]
        OwnableEvent: OwnableComponent::Event,
        #[flat]
        UpgradeableEvent: UpgradeableComponent::Event,
        #[flat]
        VrfProviderEvent: VrfProviderComponent::Event,
    }

    #[constructor]
    fn constructor(ref self: ContractState, owner: ContractAddress, pubkey: PublicKey) {
        self.ownable.initializer(owner);
        self.vrf_provider.initializer(pubkey);
    }

    #[abi(embed_v0)]
    impl UpgradeableImpl of IUpgradeable<ContractState> {
        fn upgrade(ref self: ContractState, new_class_hash: ClassHash) {
            self.ownable.assert_only_owner();
            self.upgradeable.upgrade(new_class_hash);
        }
    }
}
