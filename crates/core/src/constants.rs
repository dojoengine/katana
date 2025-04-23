use std::num::NonZeroU128;

use katana_primitives::contract::ContractAddress;
use lazy_static::lazy_static;
use starknet::macros::felt;

// Default gas prices
pub const DEFAULT_ETH_L1_GAS_PRICE: NonZeroU128 = NonZeroU128::new(20 * u128::pow(10, 9)).unwrap(); // Given in units of Wei.
pub const DEFAULT_STRK_L1_GAS_PRICE: NonZeroU128 = NonZeroU128::new(20 * u128::pow(10, 9)).unwrap(); // Given in units of STRK.

// Default data gas prices
pub const DEFAULT_ETH_L1_DATA_GAS_PRICE: NonZeroU128 = NonZeroU128::new(u128::pow(10, 6)).unwrap(); // Given in units of Wei.
pub const DEFAULT_STRK_L1_DATA_GAS_PRICE: NonZeroU128 = NonZeroU128::new(u128::pow(10, 6)).unwrap(); // Given in units of STRK.

lazy_static! {

    // Predefined contract addresses

    pub static ref DEFAULT_SEQUENCER_ADDRESS: ContractAddress = ContractAddress(felt!("0x1"));

}
