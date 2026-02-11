pub use cartridge::controller::*;
use katana_contracts::avnu::AvnuForwarder;
use katana_contracts::vrf::{CartridgeVrfAccount, CartridgeVrfConsumer, CartridgeVrfProvider};
use katana_genesis::Genesis;

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
    genesis.classes.insert(CartridgeVrfProvider::HASH, CartridgeVrfProvider::CLASS.clone().into());
}

pub fn add_avnu_forwarder_class(genesis: &mut Genesis) {
    genesis.classes.insert(AvnuForwarder::HASH, AvnuForwarder::CLASS.clone().into());
}

pub fn add_vrf_account_class(genesis: &mut Genesis) {
    genesis.classes.insert(CartridgeVrfAccount::HASH, CartridgeVrfAccount::CLASS.clone().into());
}

pub fn add_vrf_consumer_class(genesis: &mut Genesis) {
    genesis.classes.insert(CartridgeVrfConsumer::HASH, CartridgeVrfConsumer::CLASS.clone().into());
}
