pub use cartridge::controller::*;
use katana_genesis::Genesis;
use katana_primitives::utils::class::parse_sierra_class;

pub fn add_controller_classes(genesis: &mut Genesis) {
    genesis.classes.insert(ControllerV104::HASH, ControllerV104::CLASS.clone().into());
    genesis.classes.insert(ControllerV105::HASH, ControllerV105::CLASS.clone().into());
    genesis.classes.insert(ControllerV106::HASH, ControllerV106::CLASS.clone().into());
    genesis.classes.insert(ControllerV107::HASH, ControllerV107::CLASS.clone().into());
    genesis.classes.insert(ControllerV108::HASH, ControllerV108::CLASS.clone().into());
    genesis.classes.insert(ControllerV109::HASH, ControllerV109::CLASS.clone().into());
    genesis.classes.insert(ControllerLatest::HASH, ControllerLatest::CLASS.clone().into());
}

pub fn add_vrf_provider_class(genesis: &mut Genesis) {
    let vrf_provider_class =
        include_str!("../classes/cartridge_vrf_VrfProvider.contract_class.json");
    let class = parse_sierra_class(vrf_provider_class).unwrap();
    genesis.classes.insert(
        class.class_hash().expect("Failed to compute class hash for VRF provider class"),
        class.into(),
    );
}
