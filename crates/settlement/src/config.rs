use std::time::Duration;

use katana_chain_spec::{self as chain_spec, SettlementLayer, SettlementProofKind};
use katana_primitives::chain::ChainId;
use katana_primitives::{ContractAddress, Felt};
use url::Url;

/// Configuration for the embedded settlement service.
///
/// The service settles to a Starknet chain via the Piltover core contract, so
/// the settlement-chain inputs are flat on this struct; only the proving
/// system is abstracted, via [`ProverConfig`]. Built from the node's
/// [`chain_spec::SettlementConfig`] via [`SettlementConfig::from_node_config`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettlementConfig {
    /// Account on the settlement chain that submits `update_state` transactions.
    pub account_address: ContractAddress,
    /// Private key of the settlement account.
    pub account_private_key: Felt,

    /// The settlement chain's id, as recorded in the rollup chain spec.
    pub chain_id: ChainId,
    /// Settlement chain JSON-RPC endpoint.
    pub rpc_url: Url,
    /// Piltover core contract on the settlement chain.
    pub core_contract: ContractAddress,

    /// Proving-system-specific configuration.
    pub prover: ProverConfig,

    /// Number of blocks settled per `update_state` transaction.
    pub batch_size: usize,
    /// Settle a partial batch after this long without a new block.
    pub idle_flush_interval: Duration,
}

/// Proving-system-specific settlement configuration.
///
/// One variant per supported proving system; mirrors
/// [`SettlementProofKind`] on the chain spec's settlement layer. A standard
/// validity-proof system (e.g. SNOS + STARK) would be a second variant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProverConfig {
    /// TEE attestation proving (AMD SEV-SNP + SP1 Groth16).
    Tee {
        /// AMD TEE registry contract on the settlement chain.
        tee_registry: ContractAddress,

        /// SP1 prover-network private key.
        ///
        /// Required for SEV-SNP attestation proving; unused with a mock attester.
        prover_key: Option<String>,
    },
}

impl SettlementConfig {
    /// Derives the embedded settlement service config from the node's settlement config.
    ///
    /// Returns `None` unless the node actively settles to a Starknet chain with TEE proofs (the
    /// only setup the embedded service supports today): the settlement layer must be
    /// [`SettlementLayer::Starknet`] with [`SettlementProofKind::Tee`] and a `runtime` must be
    /// present.
    pub fn from_node_config(config: &chain_spec::SettlementConfig) -> Option<Self> {
        let runtime = config.runtime.as_ref()?;

        let SettlementLayer::Starknet {
            id,
            rpc_url,
            core_contract,
            proof_kind: SettlementProofKind::Tee,
            ..
        } = &config.layer
        else {
            return None;
        };

        Some(Self {
            chain_id: *id,
            rpc_url: rpc_url.clone(),
            core_contract: *core_contract,
            account_address: runtime.account_address,
            account_private_key: runtime.account_private_key,
            batch_size: runtime.batch_size,
            idle_flush_interval: Duration::from_secs(runtime.idle_flush_secs),
            prover: ProverConfig::Tee {
                tee_registry: runtime.tee_registry,
                prover_key: runtime.prover_key.clone(),
            },
        })
    }
}
