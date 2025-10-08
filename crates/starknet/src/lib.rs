#![cfg_attr(not(test), warn(unused_crate_dependencies))]

//! Rust bindings for the Starknet Core Contract on Ethereum.
//!
//! This module provides a simple interface to interact with the Starknet Core Contract,
//! specifically for fetching `LogStateUpdate` and `LogMessageToL2` events which represent
//! state updates and L1->L2 messages of the Starknet rollup.
//!
//! # Contract Reference
//!
//! The Starknet Core Contract is the main settlement contract that for Starknet that handles state
//! updates and L1↔L2 messaging. See:
//! - Contract addresses: <https://docs.starknet.io/learn/cheatsheets/chain-info#important-addresses>
//! - Solidity implementation: <https://github.com/starkware-libs/cairo-lang/blob/66355d7d99f1962ff9ccba8d0dbacbce3bd79bf8/src/starkware/starknet/solidity/Starknet.sol#L4>
//!
//! # Example
//!
//! ```rust,no_run
//! use katana_messaging::starknet_core::{StarknetCore, STARKNET_CORE_CONTRACT_ADDRESS};
//!
//! # async fn example() -> anyhow::Result<()> {
//! // Create a client for the official Starknet mainnet contract
//! let client =
//!     StarknetCore::new_http_mainnet("https://eth-mainnet.g.alchemy.com/v2/YOUR_KEY").await?;
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
//! use alloy_primitives::address;
//! use katana_messaging::starknet_core::StarknetCore;
//!
//! # async fn example() -> anyhow::Result<()> {
//! let custom_address = address!("0x1234567890123456789012345678901234567890");
//! let client = StarknetCore::new_http("http://localhost:8545", custom_address).await?;
//! # Ok(())
//! # }
//! ```

use alloy_network::Ethereum;
use alloy_primitives::Address;
use alloy_provider::{Provider, RootProvider};
use alloy_rpc_types_eth::{BlockNumberOrTag, Filter, FilterBlockOption, FilterSet, Log, Topic};
use alloy_sol_types::{sol, SolEvent};
use anyhow::Result;

/// Official Starknet Core Contract address on Ethereum mainnet.
///
/// Source: <https://docs.starknet.io/learn/cheatsheets/chain-info#mainnet>
pub const STARKNET_CORE_CONTRACT_ADDRESS_MAINNET: Address =
    alloy_primitives::address!("c662c410C0ECf747543f5bA90660f6ABeBD9C8c4");

/// Starknet Core Contract address on Ethereum Sepolia testnet.
///
/// Source: <https://docs.starknet.io/learn/cheatsheets/chain-info#sepolia>
pub const STARKNET_CORE_CONTRACT_ADDRESS_SEPOLIA: Address =
    alloy_primitives::address!("E2Bb56ee936fd6433DC0F6e7e3b8365C906AA057");

sol! {
    #[derive(Debug, PartialEq)]
    event LogMessageToL2(
        address indexed from_address,
        uint256 indexed to_address,
        uint256 indexed selector,
        uint256[] payload,
        uint256 nonce,
        uint256 fee
    );

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
/// specifically for fetching `LogStateUpdate` and `LogMessageToL2` events which represent
/// state updates and L1->L2 messages of the Starknet rollup.
#[derive(Debug)]
pub struct StarknetCore<P> {
    provider: P,
    contract_address: Address,
}

impl<P> StarknetCore<P> {
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
        Self::new(provider, STARKNET_CORE_CONTRACT_ADDRESS_MAINNET)
    }

    /// Creates a new `StarknetCore` instance using the Sepolia testnet contract address.
    ///
    /// # Arguments
    ///
    /// * `provider` - The Ethereum provider to use for queries
    pub fn new_sepolia(provider: P) -> Self {
        Self::new(provider, STARKNET_CORE_CONTRACT_ADDRESS_SEPOLIA)
    }
}

impl<P: Provider> StarknetCore<P> {
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
    pub async fn fetch_state_updates(
        &self,
        from_block: u64,
        to_block: u64,
    ) -> Result<Vec<LogStateUpdate>> {
        let logs = self.fetch_raw_state_updates_logs(from_block, to_block).await?;

        let decoded: Vec<LogStateUpdate> = logs
            .into_iter()
            .map(|log| LogStateUpdate::decode_log(log.as_ref()).map(|l| l.data))
            .collect::<Result<_, _>>()?;

        Ok(decoded)
    }

    /// Fetches and decodes all `LogMessageToL2` events in the given block range.
    ///
    /// # Arguments
    ///
    /// * `from_block` - The first block from which to fetch logs (inclusive)
    /// * `to_block` - The last block from which to fetch logs (inclusive)
    ///
    /// # Returns
    ///
    /// A vector of decoded `LogMessageToL2` events.
    ///
    /// # Errors
    ///
    /// Returns an error if the RPC request fails or if decoding fails.
    pub async fn fetch_messages_to_l2(
        &self,
        from_block: u64,
        to_block: u64,
    ) -> Result<Vec<LogMessageToL2>> {
        let logs = self.fetch_raw_messages_to_l2_logs(from_block, to_block).await?;

        let decoded: Vec<LogMessageToL2> = logs
            .into_iter()
            .map(|log| LogMessageToL2::decode_log(log.as_ref()).map(|l| l.data))
            .collect::<Result<_, _>>()?;

        Ok(decoded)
    }

    /// Fetches all `LogMessageToL2` events emitted by the contract in the given block range.
    ///
    /// # Arguments
    ///
    /// * `from_block` - The first block from which to fetch logs (inclusive)
    /// * `to_block` - The last block from which to fetch logs (inclusive)
    ///
    /// # Returns
    ///
    /// A vector of `Log` entries containing the `LogMessageToL2` events.
    ///
    /// # Errors
    ///
    /// Returns an error if the RPC request fails or if the block range is too large.
    pub async fn fetch_raw_messages_to_l2_logs(
        &self,
        from_block: u64,
        to_block: u64,
    ) -> Result<Vec<Log>> {
        let filter = Filter {
            block_option: FilterBlockOption::Range {
                from_block: Some(BlockNumberOrTag::Number(from_block)),
                to_block: Some(BlockNumberOrTag::Number(to_block)),
            },
            address: FilterSet::<Address>::from(self.contract_address),
            topics: [
                Topic::from(LogMessageToL2::SIGNATURE_HASH),
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
    async fn fetch_raw_state_updates_logs(
        &self,
        from_block: u64,
        to_block: u64,
    ) -> Result<Vec<Log>> {
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

    /// Creates a new `StarknetCore` instance with an HTTP provider using the Sepolia testnet
    /// contract address.
    ///
    /// # Arguments
    ///
    /// * `rpc_url` - The HTTP URL of the Ethereum RPC endpoint
    pub async fn new_http_sepolia(rpc_url: impl AsRef<str>) -> Result<Self> {
        let provider = RootProvider::<Ethereum>::new_http(reqwest::Url::parse(rpc_url.as_ref())?);
        Ok(Self::new_sepolia(provider))
    }
}
