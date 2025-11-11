use crate::block::{BlockNumber, GasPrices};
use crate::contract::ContractAddress;
use crate::version::StarknetVersion;

/// Block environment values.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BlockEnv {
    /// The current block height.
    pub number: BlockNumber,
    /// The timestamp in seconds since the UNIX epoch.
    pub timestamp: u64,
    /// The L2 gas prices.
    pub l2_gas_prices: GasPrices,
    /// The L1 gas prices.
    pub l1_gas_prices: GasPrices,
    /// The L1 data gas prices.
    pub l1_data_gas_prices: GasPrices,
    /// The contract address of the sequencer.
    pub sequencer_address: ContractAddress,
    /// The version of the Starknet protocol.
    pub starknet_version: StarknetVersion,
}

/// The chain configuration values.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VersionedConstantsOverrides {
    /// The maximum number of steps allowed for an invoke transaction.
    pub invoke_tx_max_n_steps: Option<u32>,
    /// The maximum number of steps allowed for transaction validation.
    pub validate_max_n_steps: Option<u32>,
    /// The maximum recursion depth allowed.
    pub max_recursion_depth: Option<usize>,
}

/// The contract addresses of the tokens used for the fees.
#[derive(Debug, Clone, Default)]
pub struct FeeTokenAddressses {
    /// The contract address of the `STRK` token.
    pub strk: ContractAddress,
    /// The contract address of the `ETH` token.
    pub eth: ContractAddress,
}
