use katana_contracts::contracts;
use katana_genesis::constant::DEFAULT_STRK_FEE_TOKEN_ADDRESS;
use katana_genesis::Genesis;
use katana_primitives::block::{ExecutableBlock, GasPrices, PartialHeader};
use katana_primitives::chain::ChainId;
use katana_primitives::da::L1DataAvailabilityMode;
use katana_primitives::state::StateUpdatesWithClasses;
use katana_primitives::version::CURRENT_STARKNET_VERSION;
use katana_primitives::{ContractAddress, Felt};
use serde::{Deserialize, Serialize};

pub mod file;
pub mod utils;

pub use file::*;

use crate::fee_token::add_fee_token;
use crate::{FeeContracts, SettlementLayer};

/// The rollup chain specification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainSpec {
    /// The rollup network chain id.
    pub id: ChainId,

    /// The chain's genesis states.
    pub genesis: Genesis,

    /// The chain fee token contract.
    pub fee_contracts: FeeContracts,

    /// The chain's settlement layer configurations.
    pub settlement: SettlementLayer,

    /// Runtime configuration for the node's embedded TEE settlement service.
    ///
    /// When present (and the settlement layer is Starknet with
    /// [`SettlementProofKind::Tee`](crate::SettlementProofKind::Tee)), the node settles its own
    /// blocks to the settlement layer's core contract instead of relying on an external prover
    /// process.
    pub settlement_runtime: Option<TeeSettlementRuntime>,
}

/// Runtime inputs for the embedded TEE settlement service.
///
/// These complement the [`SettlementLayer`] (which carries the settlement chain RPC URL and core
/// contract address): everything here is only needed by the node that actively settles, not by
/// nodes that merely follow the chain.
///
/// Note that the chain spec file holds the settlement account's private key in plaintext — the
/// settlement key lives with the node operator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct TeeSettlementRuntime {
    /// Account on the settlement chain that submits `update_state` transactions.
    pub account_address: ContractAddress,

    /// Private key of the settlement account.
    pub account_private_key: Felt,

    /// AMD TEE registry contract on the settlement chain. Used to look up the trusted certificate
    /// prefix length when generating SP1 proofs.
    pub tee_registry: ContractAddress,

    /// SP1 prover-network private key.
    ///
    /// Required for real (SEV-SNP) attestation proving; unused with a mock attester, whose quotes
    /// can never be proven on the SP1 network.
    #[serde(default)]
    pub prover_key: Option<String>,

    /// Number of blocks settled per `update_state` transaction.
    #[serde(default = "default_settlement_batch_size")]
    pub batch_size: usize,

    /// Settle a partial batch after this many seconds without a new block.
    #[serde(default = "default_settlement_idle_flush_secs")]
    pub idle_flush_secs: u64,
}

fn default_settlement_batch_size() -> usize {
    10
}

fn default_settlement_idle_flush_secs() -> u64 {
    120
}

//////////////////////////////////////////////////////////////
// 	ChainSpec implementations
//////////////////////////////////////////////////////////////

impl ChainSpec {
    pub fn block(&self) -> ExecutableBlock {
        let header = PartialHeader {
            starknet_version: CURRENT_STARKNET_VERSION,
            number: self.genesis.number,
            timestamp: self.genesis.timestamp,
            parent_hash: self.genesis.parent_hash,
            l1_da_mode: L1DataAvailabilityMode::Calldata,
            l2_gas_prices: GasPrices::MIN,
            l1_gas_prices: self.genesis.gas_prices.clone(),
            l1_data_gas_prices: self.genesis.gas_prices.clone(),
            sequencer_address: self.genesis.sequencer_address,
        };

        let transactions = utils::GenesisTransactionsBuilder::new(self).build();

        ExecutableBlock { header, body: transactions }
    }

    /// Pre-allocated genesis state applied before the genesis block executes.
    ///
    /// Currently this holds the STRK fee token: declared and deployed at the canonical Starknet
    /// mainnet address (`DEFAULT_STRK_FEE_TOKEN_ADDRESS`) with the full initial supply credited to
    /// the genesis master account. This bypasses UDC because UDC-derived addresses can't land at
    /// the canonical mainnet address.
    ///
    /// The executor must see this state when processing the genesis transactions (the
    /// `transfer_balance` invokes from [`utils::GenesisTransactionsBuilder`] target this exact
    /// contract). Callers that drive genesis execution should overlay these state updates onto an
    /// empty state provider before running the block.
    pub fn state_updates(&self) -> StateUpdatesWithClasses {
        let mut states = StateUpdatesWithClasses::default();

        // Declare the legacy ERC20 class used by the fee token. It would otherwise be declared by
        // a genesis transaction; the pre-allocation pulls it forward into initial state.
        states
            .classes
            .entry(contracts::LegacyERC20::HASH)
            .or_insert_with(|| contracts::LegacyERC20::CLASS.clone());
        states.state_updates.deprecated_declared_classes.insert(contracts::LegacyERC20::HASH);

        // The genesis master account starts with the full ERC20 supply (matches the constructor
        // mint that the old UDC-deploy used). `GenesisTransactionsBuilder` then drains this into
        // the dev accounts via `transfer` invokes during block execution.
        let master = utils::master_account_address();
        let extra_balances = [(master, utils::ROLLUP_FEE_TOKEN_INITIAL_SUPPLY)];

        add_fee_token(
            &mut states,
            "Starknet Token",
            "STRK",
            18,
            DEFAULT_STRK_FEE_TOKEN_ADDRESS,
            contracts::LegacyERC20::HASH,
            &self.genesis.allocations,
            &extra_balances,
        );

        states
    }
}
