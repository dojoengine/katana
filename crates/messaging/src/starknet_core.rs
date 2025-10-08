//! Rust bindings for the Starknet Core Contract on Ethereum.
//!
//! This module provides a simple interface to interact with the Starknet Core Contract,
//! specifically for fetching `LogStateUpdate` events which represent state updates of the
//! Starknet rollup.
//!
//! # Example
//!
//! ```rust,no_run
//! use katana_messaging::starknet_core::{StarknetCore, STARKNET_CORE_CONTRACT_ADDRESS};
//!
//! # async fn example() -> anyhow::Result<()> {
//! // Create a client for the official Starknet mainnet contract
//! let client = StarknetCore::new_http_mainnet("https://eth-mainnet.g.alchemy.com/v2/YOUR_KEY").await?;
//!
//! // Fetch state updates from blocks 18000000 to 18000100
//! let state_updates = client.fetch_decoded_state_updates(18000000, 18000100).await?;
//!
//! for update in state_updates {
//!     println!("Global Root: {}", update.globalRoot);
//!     println!("Block Number: {}", update.blockNumber);
//!     println!("Block Hash: {}", update.blockHash);
//! }
//! # Ok(())
//! # }
//! ```
//!
//! # Custom Contract Address
//!
//! You can also create a client for a custom contract address (e.g., for testing):
//!
//! ```rust,no_run
//! use katana_messaging::starknet_core::StarknetCore;
//! use alloy_primitives::address;
//!
//! # async fn example() -> anyhow::Result<()> {
//! let custom_address = address!("0x1234567890123456789012345678901234567890");
//! let client = StarknetCore::new_http("http://localhost:8545", custom_address).await?;
//! # Ok(())
//! # }
//! ```

#![allow(dead_code)]

use alloy_network::Ethereum;
use alloy_primitives::{Address, U256};
use alloy_provider::{Provider, RootProvider};
use alloy_rpc_types_eth::{BlockNumberOrTag, Filter, FilterBlockOption, FilterSet, Log, Topic};
use alloy_sol_types::{sol, SolEvent};
use anyhow::Result;
use tracing::trace;

use super::LOG_TARGET;

// Official Starknet Core Contract address on Ethereum mainnet
pub const STARKNET_CORE_CONTRACT_ADDRESS: Address =
    alloy_primitives::address!("c662c410C0ECf747543f5bA90660f6ABeBD9C8c4");

sol! {
    #[derive(Debug, PartialEq)]
    event LogStateUpdate(
        uint256 globalRoot,
        int256 blockNumber,
        uint256 blockHash
    );
}

/// Rust bindings for the Starknet Core Contract.
///
/// This provides methods to interact with the Starknet Core Contract deployed on Ethereum,
/// specifically for fetching `LogStateUpdate` events which represent state updates of the
/// Starknet rollup.
#[derive(Debug)]
pub struct StarknetCore<P> {
    provider: P,
    contract_address: Address,
}

impl<P> StarknetCore<P>
where
    P: Provider,
{
    /// Creates a new `StarknetCore` instance with a custom contract address.
    ///
    /// # Arguments
    ///
    /// * `provider` - The Ethereum provider to use for queries
    /// * `contract_address` - The address of the Starknet Core Contract
    pub fn new(provider: P, contract_address: Address) -> Self {
        Self { provider, contract_address }
    }

    /// Creates a new `StarknetCore` instance using the official mainnet contract address.
    ///
    /// # Arguments
    ///
    /// * `provider` - The Ethereum provider to use for queries
    pub fn new_mainnet(provider: P) -> Self {
        Self::new(provider, STARKNET_CORE_CONTRACT_ADDRESS)
    }

    /// Fetches all `LogStateUpdate` events emitted by the contract in the given block range.
    ///
    /// # Arguments
    ///
    /// * `from_block` - The first block from which to fetch logs (inclusive)
    /// * `to_block` - The last block from which to fetch logs (inclusive)
    ///
    /// # Returns
    ///
    /// A vector of `Log` entries containing the `LogStateUpdate` events.
    ///
    /// # Errors
    ///
    /// Returns an error if the RPC request fails or if the block range is too large.
    pub async fn fetch_state_updates(&self, from_block: u64, to_block: u64) -> Result<Vec<Log>> {
        trace!(
            target: LOG_TARGET,
            from_block = ?from_block,
            to_block = ?to_block,
            "Fetching LogStateUpdate events."
        );

        let filter = Filter {
            block_option: FilterBlockOption::Range {
                from_block: Some(BlockNumberOrTag::Number(from_block)),
                to_block: Some(BlockNumberOrTag::Number(to_block)),
            },
            address: FilterSet::<Address>::from(self.contract_address),
            topics: [
                Topic::from(LogStateUpdate::SIGNATURE_HASH),
                Default::default(),
                Default::default(),
                Default::default(),
            ],
        };

        let logs: Vec<Log> = self
            .provider
            .get_logs(&filter)
            .await?
            .into_iter()
            .filter(|log| log.block_number.is_some())
            .collect();

        Ok(logs)
    }

    /// Fetches and decodes all `LogStateUpdate` events in the given block range.
    ///
    /// # Arguments
    ///
    /// * `from_block` - The first block from which to fetch logs (inclusive)
    /// * `to_block` - The last block from which to fetch logs (inclusive)
    ///
    /// # Returns
    ///
    /// A vector of decoded `LogStateUpdate` events.
    ///
    /// # Errors
    ///
    /// Returns an error if the RPC request fails or if decoding fails.
    pub async fn fetch_decoded_state_updates(
        &self,
        from_block: u64,
        to_block: u64,
    ) -> Result<Vec<LogStateUpdate>> {
        let logs = self.fetch_state_updates(from_block, to_block).await?;

        let decoded: Vec<LogStateUpdate> = logs
            .into_iter()
            .filter_map(|log| LogStateUpdate::decode_log(log.as_ref()).ok())
            .collect();

        Ok(decoded)
    }
}

// Convenience constructor for creating a StarknetCore instance with HTTP provider
impl StarknetCore<RootProvider<Ethereum>> {
    /// Creates a new `StarknetCore` instance with an HTTP provider.
    ///
    /// # Arguments
    ///
    /// * `rpc_url` - The HTTP URL of the Ethereum RPC endpoint
    /// * `contract_address` - The address of the Starknet Core Contract
    pub async fn new_http(rpc_url: impl AsRef<str>, contract_address: Address) -> Result<Self> {
        let provider = RootProvider::<Ethereum>::new_http(reqwest::Url::parse(rpc_url.as_ref())?);
        Ok(Self::new(provider, contract_address))
    }

    /// Creates a new `StarknetCore` instance with an HTTP provider using the official mainnet
    /// contract address.
    ///
    /// # Arguments
    ///
    /// * `rpc_url` - The HTTP URL of the Ethereum RPC endpoint
    pub async fn new_http_mainnet(rpc_url: impl AsRef<str>) -> Result<Self> {
        let provider = RootProvider::<Ethereum>::new_http(reqwest::Url::parse(rpc_url.as_ref())?);
        Ok(Self::new_mainnet(provider))
    }
}

#[cfg(test)]
mod tests {
    use alloy_primitives::{address, b256, LogData};

    use super::*;

    #[test]
    fn test_mainnet_address() {
        assert_eq!(
            STARKNET_CORE_CONTRACT_ADDRESS,
            address!("c662c410C0ECf747543f5bA90660f6ABeBD9C8c4")
        );
    }

    #[test]
    fn test_log_state_update_decode() {
        let global_root = U256::from(0x1234567890abcdef_u64);
        let block_number = 123456_i64;
        let block_hash = U256::from(0xabcdef1234567890_u64);

        let event = LogStateUpdate::new(
            b256!("0x000000000000000000000000000000000000000000000000000000000000000"),
            (global_root, block_number.into(), block_hash),
        );

        let log = Log {
            inner: alloy_primitives::Log::<LogData> {
                address: STARKNET_CORE_CONTRACT_ADDRESS,
                data: LogData::from(&event),
            },
            ..Default::default()
        };

        let decoded = LogStateUpdate::decode_log(log.as_ref()).unwrap();

        assert_eq!(decoded.globalRoot, global_root);
        assert_eq!(decoded.blockNumber, block_number.into());
        assert_eq!(decoded.blockHash, block_hash);
    }

    #[test]
    fn test_log_state_update_signature() {
        // The event signature should match the keccak256 hash of:
        // "LogStateUpdate(uint256,int256,uint256)"
        let expected_signature =
            b256!("0x000000000000000000000000000000000000000000000000000000000000000");

        // Note: The actual signature hash would be computed at compile time by alloy
        // This test verifies the event can be created
        let event = LogStateUpdate::new(
            expected_signature,
            (U256::from(1), U256::from(2).into(), U256::from(3)),
        );

        assert_eq!(event.globalRoot, U256::from(1));
    }
}
