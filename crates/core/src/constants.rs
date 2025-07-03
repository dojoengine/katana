use katana_primitives::contract::ContractAddress;
use lazy_static::lazy_static;
use starknet::macros::felt;

lazy_static! {

    // Predefined contract addresses

    pub static ref DEFAULT_SEQUENCER_ADDRESS: ContractAddress = ContractAddress(felt!("0x1"));

}
