use katana_primitives::contract::ContractAddress;
use katana_primitives::felt;
use lazy_static::lazy_static;

lazy_static! {

    // Predefined contract addresses

    pub static ref DEFAULT_SEQUENCER_ADDRESS: ContractAddress = ContractAddress(felt!("0x1"));

}
