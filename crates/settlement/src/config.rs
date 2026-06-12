use std::time::Duration;

use katana_chain_spec::{rollup, SettlementLayer, SettlementProofKind};
use katana_primitives::{ContractAddress, Felt};
use url::Url;

/// Configuration for the embedded TEE settlement service.
///
/// Combines the settlement chain endpoint from the rollup chain spec's
/// [`SettlementLayer`] with the runtime inputs from its `[settlement-runtime]`
/// section. See [`SettlementConfig::from_rollup_spec`].
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
    /// AMD TEE registry contract on the settlement chain.
    pub tee_registry: ContractAddress,
    /// SP1 prover-network private key. Required for SEV-SNP attestation proving.
    pub prover_key: Option<String>,
    /// Number of blocks settled per `update_state` transaction.
    pub batch_size: usize,
    /// Settle a partial batch after this long without a new block.
    pub idle_flush_interval: Duration,
}

impl SettlementConfig {
    /// Derives the settlement service config from a rollup chain spec.
    ///
    /// Returns `None` when the spec has no `[settlement-runtime]` section or when the settlement
    /// layer is not a Starknet chain settling with TEE proofs — the only mode the embedded
    /// service supports.
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
            tee_registry: runtime.tee_registry,
            prover_key: runtime.prover_key.clone(),
            batch_size: runtime.batch_size,
            idle_flush_interval: Duration::from_secs(runtime.idle_flush_secs),
        })
    }
}
