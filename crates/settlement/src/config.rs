use std::time::Duration;

use katana_chain_spec::{rollup, SettlementLayer, SettlementProofKind};
use katana_primitives::{ContractAddress, Felt};
use url::Url;

/// Configuration for the embedded settlement service.
///
/// The proof-system-agnostic parts (settlement chain endpoint, core contract, settlement
/// account, batching) live directly on this struct; everything specific to a proving system is
/// in [`ProverConfig`]. See [`SettlementConfig::from_rollup_spec`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettlementConfig {
    /// Settlement chain JSON-RPC endpoint.
    pub rpc_url: Url,
    /// Piltover core contract on the settlement chain.
    pub core_contract: ContractAddress,
    /// Account on the settlement chain that submits `update_state` transactions.
    pub account_address: ContractAddress,
    /// Private key of the settlement account.
    pub account_private_key: Felt,
    /// Number of blocks settled per `update_state` transaction.
    pub batch_size: usize,
    /// Settle a partial batch after this long without a new block.
    pub idle_flush_interval: Duration,
    /// Proving-system-specific configuration.
    pub prover: ProverConfig,
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
        /// SP1 prover-network private key. Required for SEV-SNP attestation proving; unused
        /// with a mock attester.
        prover_key: Option<String>,
    },
}

impl SettlementConfig {
    /// Derives the settlement service config from a rollup chain spec.
    ///
    /// Returns `None` when the spec has no `[settlement-runtime]` section or when the settlement
    /// layer is not a Starknet chain settling with TEE proofs — the only proving system the
    /// embedded service supports today.
    pub fn from_rollup_spec(spec: &rollup::ChainSpec) -> Option<Self> {
        let runtime = spec.settlement_runtime.as_ref()?;

        let SettlementLayer::Starknet {
            rpc_url,
            core_contract,
            proof_kind: SettlementProofKind::Tee,
            ..
        } = &spec.settlement
        else {
            return None;
        };

        Some(Self {
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
