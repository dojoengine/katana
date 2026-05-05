use snforge_std::fs::{FileParser, FileTrait};

#[derive(Drop, Serde)]
pub struct RootCerts {
    pub genoa_ark_hash_high: felt252,
    pub genoa_ark_hash_low: felt252,
    pub milan_ark_hash_high: felt252,
    pub milan_ark_hash_low: felt252,
}

pub fn load_root_certs() -> RootCerts {
    let file = FileTrait::new("../amd_root_certs.json");
    FileParser::<RootCerts>::parse_json(@file).expect('Failed to parse root_certs.json')
}

pub fn get_milan_root() -> u256 {
    let certs = load_root_certs();
    u256 {
        low: certs.milan_ark_hash_low.try_into().unwrap(),
        high: certs.milan_ark_hash_high.try_into().unwrap(),
    }
}

pub fn get_genoa_root() -> u256 {
    let certs = load_root_certs();
    u256 {
        low: certs.genoa_ark_hash_low.try_into().unwrap(),
        high: certs.genoa_ark_hash_high.try_into().unwrap(),
    }
}
