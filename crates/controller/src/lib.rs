pub use account_sdk::artifacts::CONTROLLERS;
use katana_primitives::genesis::Genesis;
use katana_primitives::utils::class::parse_sierra_class;

pub fn add_controller_classes(genesis: &mut Genesis) {
    genesis.classes.extend(
        CONTROLLERS.iter().map(|(_, v)| (v.hash, parse_sierra_class(v.content).unwrap().into())),
    );
}

pub fn add_vrf_provider_class(genesis: &mut Genesis) {
    let vrf_provider_class =
        include_str!("../artifacts/cartridge_vrf_VrfProvider.contract_class.json");
    let class = parse_sierra_class(vrf_provider_class).unwrap();
    genesis.classes.insert(
        class.class_hash().expect("Failed to compute class hash for VRF provider class"),
        class.into(),
    );
}
