//! Settlement configuration.
//!
//! Settlement is orthogonal to chain identity — it describes *where* and *how*
//! a node settles, not what the chain is — so these types live outside the
//! [`ChainSpec`](crate::ChainSpec) and are routed directly into the node's
//! settlement config.
//!
//! [`SettlementConfig`] splits into two halves with different consumers:
//!
//! - [`SettlementLayer`] (the `layer` field) — *where* settlement happens: the settlement chain's
//!   id, RPC, and core contract. Needed by anything that reads the settlement chain, including the
//!   messaging collector on a node that does not itself settle.
//! - [`SettlementRuntime`] (the optional `runtime` field) — *whether and how* this node actively
//!   settles: the settlement account + key, prover inputs, and batching. Only the embedded
//!   settlement service needs it.
//!
//! So `runtime.is_none()` marks a node that follows/relays the settlement chain
//! without settling, and the all-or-nothing operational fields are grouped
//! behind a single `Option` instead of several loose ones.

use katana_primitives::block::BlockNumber;
use katana_primitives::chain::ChainId;
use katana_primitives::{eth, ContractAddress, Felt};
use serde::{Deserialize, Serialize};
use url::Url;

/// A node's settlement configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct SettlementConfig {
    /// Where settlement happens — the settlement chain and core contract.
    pub layer: SettlementLayer,

    /// How this node actively settles, if it does. `None` for nodes that read
    /// the settlement chain (e.g. for messaging) but do not settle.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<SettlementRuntime>,
}

/// Runtime inputs for the node's embedded settlement service.
///
/// These complement the [`SettlementLayer`]: everything here is only needed by
/// the node that actively settles, not by nodes that merely follow the chain.
/// The settlement account's private key is held in plaintext — the settlement
/// key lives with the node operator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct SettlementRuntime {
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

    /// Maximum seconds between settlements while blocks are pending.
    ///
    /// A batch is settled when it reaches `batch_size` blocks or when this many
    /// seconds have elapsed since its first pending block — whichever comes first.
    #[serde(default = "default_settlement_idle_flush_secs")]
    pub idle_flush_secs: u64,
}

fn default_settlement_batch_size() -> usize {
    10
}

fn default_settlement_idle_flush_secs() -> u64 {
    120
}

/// The settlement chain a node settles to and the core contract it settles through.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SettlementLayer {
    Ethereum {
        // The id of the settlement chain.
        id: eth::ChainId,

        // url for ethereum rpc provider
        rpc_url: Url,

        // - The core appchain contract used to settlement
        core_contract: eth::Address,

        // the block at which the core contract was deployed
        block: alloy_primitives::BlockNumber,
    },

    Starknet {
        // The id of the settlement chain.
        id: ChainId,

        // url for starknet rpc provider
        rpc_url: Url,

        // - The core appchain contract used to settlement
        core_contract: ContractAddress,

        // the block at which the core contract was deployed
        block: BlockNumber,

        /// The proof system the core contract was initialized for. Determines which fields of
        /// Piltover's `program_info` are meaningful and validated at startup.
        #[serde(default)]
        proof_kind: SettlementProofKind,
    },

    Sovereign {
        // Once Katana can sync from data availability layer, we can add the details of the data
        // availability layer to the chain spec for Katana to sync from it.
    },
}

/// The proof system a Starknet settlement contract was initialized for.
///
/// Validity-proof chains have meaningful program hashes (SNOS, layout-bridge, bootloader) on the
/// core contract. TEE chains do not — only the SNOS config hash is validated against the chain's
/// own id and fee token.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SettlementProofKind {
    #[default]
    ValidityProof,
    Tee,
}
