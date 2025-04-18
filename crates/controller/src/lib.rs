pub use account_sdk::artifacts::CONTROLLERS;
use katana_primitives::genesis::Genesis;
use katana_primitives::utils::class::parse_sierra_class;

pub fn add_controller_classes(genesis: &mut Genesis) {
    genesis.classes.extend(
        CONTROLLERS.iter().map(|(_, v)| (v.hash, parse_sierra_class(v.content).unwrap().into())),
    );
}
