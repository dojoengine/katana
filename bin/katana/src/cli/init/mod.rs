//! Chain initialization commands for Katana.
//!
//! This module provides functionality to initialize new blockchain networks with Katana,
//! supporting both rollup and [sovereign] chain configurations. Currently, Katana only supports
//! deploying the rollup chain on top of the Starknet blockchain.
//!
//! # Overview
//!
//! The `init` command supports two distinct initialization modes:
//!
//! ## Rollup Mode (`katana init rollup`)
//!
//! Initializes a rollup chain that settles on an existing blockchain (right now only Starknet).
//!
//! **Interactive Usage:**
//!
//! ```bash
//! // Prompts for all required information when no flags are provided.
//! katana init rollup
//! ```
//!
//! **Explicit Usage:**
//!
//! ```bash
//! katana init rollup \
//!   --id my-rollup \
//!   --settlement-chain sepolia \
//!   --settlement-account-address 0x123... \
//!   --settlement-account-private-key 0x456...
//! ```
//!
//! ## Sovereign Mode (`katana init sovereign`)
//!
//! Initializes a sovereign chain that operates independently without settlement on another
//! blockchain. State updates and proofs are published to a Data Availability layer only.
//!
//! **Interactive Usage:**
//!
//! ```bash
//! // Prompts for all required information when no flags are provided.
//! katana init sovereign
//! ```
//!
//! **Explicit Usage:**
//!
//! ```bash
//! katana init sovereign --id my-sovereign-chain
//! ```
//!
//! # configuration output
//!
//! both modes generate chain specification files that can be used to start katana nodes:
//! - local configuration: `~/.config/katana/chains/`
//! - custom path: use `--output-path` to specify a directory
//!
//! [sovereign]: https://celestia.org/learn/intermediates/sovereign-rollups-an-introduction/

use std::path::PathBuf;
use std::str::FromStr;

use anyhow::Context;
use clap::{Args, Subcommand};
use deployment::DeploymentOutcome;
use katana_chain_spec::settlement_check::SettlementChainProvider;
use katana_chain_spec::{
    rollup, ChainSpec, FeeContracts, SettlementConfig, SettlementLayer, SettlementProofKind,
};
use katana_cli::chain_config::{self, ChainConfigDir};
use katana_cli::utils::ShortStringValueParser;
use katana_contracts::piltover::AppchainCoreContract;
use katana_genesis::allocation::DevAllocationsGenerator;
use katana_genesis::constant::{DEFAULT_PREFUNDED_ACCOUNT_BALANCE, DEFAULT_STRK_FEE_TOKEN_ADDRESS};
use katana_genesis::Genesis;
use katana_primitives::block::BlockNumber;
use katana_primitives::cairo::ShortString;
use katana_primitives::chain::ChainId;
use katana_primitives::{felt, ContractAddress, Felt, U256};
use starknet::accounts::{ExecutionEncoding, SingleOwnerAccount};
use starknet::providers::Provider;
use starknet::signers::SigningKey;
use url::Url;

pub mod deployment;
mod prompt;
#[cfg(feature = "init-slot")]
mod slot;

/// The mock AMD TEE registry (`mock_amd_tee_registry`) deployed on Starknet Sepolia by the
/// `cartridge-gg/piltover` project. It skips real SEV-SNP / SP1 Groth16 verification, so it pairs
/// with the mock prover (`saya-tee --mock-prove`). Used to prefill the TEE registry address when a
/// rollup is initialized with the Mock TEE proof on Sepolia.
const MOCK_TEE_REGISTRY_SEPOLIA: Felt =
    felt!("0x037189b1807f1358074b70b3dc8ab79167bbf72cff1296286052f6dfe31c8f15");

/// The canonical AMD TEE registry (`AMDTEERegistry`) deployed on Starknet Sepolia by the
/// `cartridge-gg/katana-tee` project. It performs real SEV-SNP / SP1 Groth16 attestation
/// verification, so it pairs with the real prover (`saya-tee`, non-mock). Used to prefill the TEE
/// registry address when a rollup is initialized with the AMD SEV-SNP + SP1 Groth16 proof on
/// Sepolia.
///
/// Source: <https://github.com/cartridge-gg/katana-tee/blob/39831d1854fac9b39ea56d9378ad8eeed6c6c193/deployments/sepolia.json#L10>
const AMD_TEE_REGISTRY_SEPOLIA: Felt =
    felt!("0x01258ed7b2d3435097f9290d100d706d7f9f65db2725609cd7697669cac3bc3a");

#[derive(Debug, Args)]
#[cfg_attr(test, derive(PartialEq, Eq))]
pub struct InitCommand {
    #[command(subcommand)]
    pub mode: InitMode,
}

/// initialization mode selection for different chain types.
#[derive(Debug, Subcommand)]
#[cfg_attr(test, derive(PartialEq, Eq))]
pub enum InitMode {
    #[command(about = "Initialize a rollup chain")]
    Rollup(Box<RollupArgs>),

    #[command(hide = true)]
    #[command(about = "Initialize a sovereign chain")]
    Sovereign(SovereignArgs),
}

/// Configuration arguments for rollup chain initialization.
///
/// Rollup chains settle their state and proofs on an existing Layer 1 blockchain.
/// This requires settlement layer connectivity, account management, and contract deployment.
///
/// ## Interactivity
///
/// If no arguments are provided, the command will prompt interactively for them.
#[derive(Debug, Args)]
#[cfg_attr(test, derive(PartialEq, Eq))]
pub struct RollupArgs {
    /// The id of the new chain to be initialized.
    ///
    /// An empty `Id` is not a allowed, since the chain id must be
    /// a valid ASCII string.
    #[arg(long)]
    #[arg(value_parser = ShortStringValueParser)]
    #[arg(requires_all = ["settlement_chain", "settlement_account", "settlement_account_private_key"])]
    id: Option<ShortString>,

    #[arg(
        long = "settlement-chain",
        help = "The settlement chain to be used, where the core contract is deployed."
    )]
    #[arg(long_help = "The settlement chain to be used, where the core contract is deployed.

Possible values:
  - `mainnet`, `sn_mainnet`: Starknet mainnet
  - `sepolia`, `sn_sepolia`: Starknet sepolia")]
    #[cfg_attr(
        feature = "init-custom-settlement-chain",
        arg(long_help = "The settlement chain to be used, where the core contract is deployed.

Possible values:
  - `mainnet`, `sn_mainnet`: Starknet mainnet
  - `sepolia`, `sn_sepolia`: Starknet sepolia
  - <URL>: Custom settlement chain URL (requires --settlement-facts-registry)

If a custom settlement chain is provided, setting a custom facts registry is required using
the `--settlement-facts-registry` option. Otherwise, setting a custom facts registry
with a known chain is a no-op for now.")
    )]
    #[arg(requires = "id")]
    settlement_chain: Option<SettlementChain>,
    /// The address of the settlement account to be used to configure the core contract.
    #[arg(long = "settlement-account-address")]
    #[arg(requires = "id")]
    settlement_account: Option<ContractAddress>,

    /// The private key of the settlement account to be used to configure the core contract.
    #[arg(long = "settlement-account-private-key")]
    #[arg(requires = "id")]
    settlement_account_private_key: Option<Felt>,

    /// The address of the settlement contract.
    /// If not provided, the contract will be deployed on the settlement chain using the provided
    /// settlement account.
    #[arg(long = "settlement-contract")]
    #[arg(requires_all = ["id", "settlement_contract_deployed_block"])]
    settlement_contract: Option<ContractAddress>,

    /// The block number of the settlement contract deployment.
    /// This value is required if the `settlement-contract` is provided, for Katana to
    /// know from which block the messages can be gathered from the settlement chain.
    #[arg(long = "settlement-contract-deployed-block")]
    #[arg(requires = "settlement_contract")]
    settlement_contract_deployed_block: Option<BlockNumber>,

    /// The address of the facts registry contract on the settlement chain.
    ///
    /// Required if a custom settlement chain is specified.
    #[arg(long = "settlement-facts-registry")]
    #[arg(conflicts_with = "tee")]
    settlement_facts_registry_contract: Option<ContractAddress>,

    /// Set up the Piltover core contract for TEE-proved settlement (Saya
    /// persistent-TEE mode) instead of STARK proofs. Piltover's fact-registry
    /// field is pointed at `--tee-registry-address` instead of the default
    /// Herodotus Atlantic registry.
    #[arg(long = "tee")]
    #[arg(requires_all = ["id", "tee_registry_address"])]
    tee: bool,

    /// Address of the IAMDTeeRegistry contract on the settlement chain.
    /// Required when --tee is set.
    #[arg(long = "tee-registry-address")]
    #[arg(requires = "tee")]
    tee_registry_address: Option<ContractAddress>,

    /// Specify the path of the directory where the configuration files will be stored at.
    #[arg(long)]
    output_path: Option<PathBuf>,

    #[cfg(feature = "init-slot")]
    #[command(flatten)]
    slot: slot::SlotArgs,
}

#[derive(Debug, Args)]
#[cfg_attr(test, derive(PartialEq, Eq))]
pub struct SovereignArgs {
    /// The id of the new chain to be initialized.
    ///
    /// An empty `Id` is not a allowed, since the chain id must be
    /// a valid ASCII string.
    #[arg(long)]
    #[arg(value_parser = ShortStringValueParser)]
    id: Option<ShortString>,

    /// Specify the path of the directory where the configuration files will be stored at.
    #[arg(long)]
    output_path: Option<PathBuf>,

    #[cfg(feature = "init-slot")]
    #[command(flatten)]
    slot: slot::SlotArgs,
}

impl InitCommand {
    /// Executes the initialization command based on the selected mode.
    ///
    /// Dispatches to the appropriate initialization logic for either rollup or sovereign chains.
    pub(crate) async fn execute(self) -> anyhow::Result<()> {
        match self.mode {
            InitMode::Rollup(args) => args.execute().await,
            InitMode::Sovereign(args) => args.execute().await,
        }
    }
}

impl RollupArgs {
    /// Executes rollup chain initialization with settlement layer integration.
    ///
    /// # Interactive Behavior
    ///
    /// Falls back to interactive prompts when no CLI flags are provided.
    pub(crate) async fn execute(self) -> anyhow::Result<()> {
        let output = if let Some(output) = self.configure_from_args().await {
            output?
        } else {
            prompt::prompt_rollup().await?
        };

        let proof_kind = if output.proof_impl.is_tee() {
            SettlementProofKind::Tee
        } else {
            SettlementProofKind::ValidityProof
        };

        let settlement = SettlementLayer::Starknet {
            rpc_url: output.rpc_url.clone(),
            id: ChainId::parse(&output.settlement_id)?,
            block: output.deployment_outcome.block_number,
            core_contract: output.deployment_outcome.contract_address,
            proof_kind,
        };

        let id = ChainId::parse(&output.id)?;

        // Rollups are Cartridge Controller–ready by default: seed 3 dev accounts because
        // the Cartridge paymaster sidecar reserves accounts 0/1/2 (relayer, gas tank,
        // estimate) — with fewer, its bootstrap aborts.
        #[cfg_attr(not(feature = "init-slot"), allow(unused_mut))]
        let mut genesis = generate_genesis(3);
        #[cfg(feature = "init-slot")]
        slot::add_paymasters_to_genesis(&mut genesis, &output.slot_paymasters.unwrap_or_default());
        // Declare the Controller account classes so a Cartridge Controller can be
        // deployed/used on this chain (same helper the `--dev` path uses).
        katana_slot_controller::add_controller_classes(&mut genesis);

        // STRK is pre-allocated by rollup::ChainSpec::state_updates at the canonical Starknet
        // mainnet address. ETH mirrors STRK on rollup — the on-disk config keeps only one address
        // (see FileFeeContract).
        let fee_contracts = FeeContracts {
            eth: DEFAULT_STRK_FEE_TOKEN_ADDRESS,
            strk: DEFAULT_STRK_FEE_TOKEN_ADDRESS,
        };

        let chain_spec = ChainSpec::Rollup(rollup::ChainSpec { id, genesis, fee_contracts });

        // Settlement is recorded alongside the chain spec, not inside it. `init` writes only the
        // settlement layer; the operator adds a `[settlement.runtime]` section to actively settle.
        let settlement = SettlementConfig { layer: settlement, runtime: None };

        let dir = match &self.output_path {
            Some(path) => ChainConfigDir::create(path)?,
            // Write to the local chain config directory by default if user
            // doesn't specify the output path
            None => ChainConfigDir::create_local(&chain_spec.id())?,
        };
        chain_config::write(&dir, &chain_spec, Some(&settlement))
            .context("failed to write chain spec file")?;

        // ----- Print initialization summary -----

        println!(
            r"
CHAIN
=====

| Chain ID        | {chain_id} ({chain_id_felt:#x})
| Config file     | {config_path}
| Genesis file    | {genesis_path}


SETTLEMENT LAYER
================

| Proof category  | {proof_category}
| Proof type      | {proof_implementation}
| Chain ID        | {settlement_id} ({settlement_id_felt:#x})
| RPC URL         | {rpc_url}
| Core contract   | {core_contract}
| Deployed block  | #{deployed_block}
| Fact registry   | {fact_registry:#066x}
| Config hash     | {config_hash:#066x}",
            chain_id = output.id,
            chain_id_felt = Felt::from(output.id),
            config_path = dir.config_path().display(),
            genesis_path = dir.genesis_path().display(),
            proof_category = output.proof_impl.category_label(),
            proof_implementation = output.proof_impl.implementation_label(),
            settlement_id = output.settlement_id,
            settlement_id_felt = Felt::from(output.settlement_id),
            rpc_url = output.rpc_url,
            core_contract = output.deployment_outcome.contract_address,
            deployed_block = output.deployment_outcome.block_number,
            fact_registry = output.effective_fact_registry,
            config_hash = output.deployment_outcome.config_hash,
        );

        // Only show the Piltover class hash when we actually declared/deployed it ourselves.
        // For --settlement-contract (user-supplied), check_program_info validates program info
        // but not the on-chain class hash, so printing it would risk misleading the operator.
        if output.deployment_outcome.class_declared {
            println!(
                "| Class hash      | {:#066x} (declared this run)",
                AppchainCoreContract::HASH
            );
        }
        println!();

        Ok(())
    }

    async fn configure_from_args(&self) -> Option<anyhow::Result<PersistentOutcome>> {
        if let Some(id) = self.id {
            // Check if all required settlement args are provided
            let Some(settlement_chain) = self.settlement_chain.clone() else {
                return None; // Fall back to prompting
            };
            let Some(settlement_account_address) = self.settlement_account else {
                return None; // Fall back to prompting
            };
            let Some(settlement_private_key) = self.settlement_account_private_key else {
                return None; // Fall back to prompting
            };

            let settlement_provider = match &settlement_chain {
                SettlementChain::Mainnet => {
                    let mut provider = SettlementChainProvider::sn_mainnet();
                    if let Some(fact_registry) = self.settlement_facts_registry_contract {
                        provider.set_fact_registry(*fact_registry);
                    }
                    provider
                }
                SettlementChain::Sepolia => {
                    let mut provider = SettlementChainProvider::sn_sepolia();
                    if let Some(fact_registry) = self.settlement_facts_registry_contract {
                        provider.set_fact_registry(*fact_registry);
                    }
                    provider
                }
                #[cfg(feature = "init-custom-settlement-chain")]
                SettlementChain::Custom(url) => {
                    // In TEE mode, --tee-registry-address is the fact registry. In ZK mode,
                    // a custom chain must provide --settlement-facts-registry explicitly.
                    if self.tee {
                        let tee_registry = self.tee_registry_address.expect("clap requires_all");
                        SettlementChainProvider::new(url.clone(), *tee_registry)
                    } else {
                        let Some(fact_registry) = self.settlement_facts_registry_contract else {
                            return Some(Err(anyhow::anyhow!(
                                "Specifying the facts registry contract (using \
                                 `--settlement-facts-registry`) is required when settling on a \
                                 custom chain"
                            )));
                        };
                        SettlementChainProvider::new(url.clone(), *fact_registry)
                    }
                }
            };

            let effective_fact_registry = resolve_effective_fact_registry(
                self.tee,
                self.tee_registry_address,
                self.settlement_facts_registry_contract,
                settlement_provider.fact_registry(),
            );

            let l1_chain_id = match settlement_provider.chain_id().await.with_context(|| {
                format!("failed to get chain id for settlement layer `{settlement_chain}`")
            }) {
                Ok(id) => id,
                Err(err) => return Some(Err(err)),
            };

            let chain_id = Felt::from(id);

            let deployment_outcome = if let Some(contract) = self.settlement_contract {
                let config_hash = match deployment::check_program_info(
                    chain_id,
                    contract,
                    &settlement_provider,
                    effective_fact_registry,
                    self.tee,
                )
                .await
                .with_context(|| "settlement contract validation failed.".to_string())
                {
                    Ok(hash) => hash,
                    Err(err) => return Some(Err(err)),
                };

                DeploymentOutcome {
                    contract_address: contract,
                    block_number: self
                        .settlement_contract_deployed_block
                        .expect("must exist at this point"),
                    class_declared: false,
                    config_hash,
                }
            }
            // If settlement contract is not provided, then we will deploy it.
            else {
                let account = SingleOwnerAccount::new(
                    settlement_provider.clone(),
                    SigningKey::from_secret_scalar(settlement_private_key).into(),
                    settlement_account_address.into(),
                    l1_chain_id,
                    ExecutionEncoding::New,
                );

                match deployment::deploy_settlement_contract(
                    account,
                    chain_id,
                    effective_fact_registry,
                    self.tee,
                )
                .await
                .with_context(|| "failed to deploy settlement contract".to_string())
                {
                    Ok(id) => id,
                    Err(err) => return Some(Err(err)),
                }
            };

            let proof_impl =
                if self.tee { ProofImpl::AmdSevSnpSp1Groth16 } else { ProofImpl::Stark };

            Some(Ok(PersistentOutcome {
                id,
                deployment_outcome,
                rpc_url: settlement_provider.url().clone(),
                settlement_id: ShortString::try_from(l1_chain_id).unwrap(),
                effective_fact_registry,
                proof_impl,
                #[cfg(feature = "init-slot")]
                slot_paymasters: self.slot.paymaster_accounts.clone(),
            }))
        } else {
            None
        }
    }
}

impl SovereignArgs {
    /// Executes sovereign chain initialization.
    pub(crate) async fn execute(self) -> anyhow::Result<()> {
        let output = if let Some(output) = self.configure_from_args() {
            output
        } else {
            prompt::prompt_sovereign().await?
        };

        let settlement = SettlementLayer::Sovereign {};
        let id = ChainId::parse(&output.id)?;

        #[cfg_attr(not(feature = "init-slot"), allow(unused_mut))]
        let mut genesis = generate_genesis(1);
        #[cfg(feature = "init-slot")]
        slot::add_paymasters_to_genesis(&mut genesis, &output.slot_paymasters.unwrap_or_default());

        // STRK is pre-allocated by rollup::ChainSpec::state_updates at the canonical Starknet
        // mainnet address. ETH mirrors STRK on rollup — the on-disk config keeps only one address
        // (see FileFeeContract).
        let fee_contracts = FeeContracts {
            eth: DEFAULT_STRK_FEE_TOKEN_ADDRESS,
            strk: DEFAULT_STRK_FEE_TOKEN_ADDRESS,
        };

        let chain_spec = ChainSpec::Rollup(rollup::ChainSpec { id, genesis, fee_contracts });

        let settlement = SettlementConfig { layer: settlement, runtime: None };

        let dir = match &self.output_path {
            Some(path) => ChainConfigDir::create(path)?,
            None => ChainConfigDir::create_local(&chain_spec.id())?,
        };
        chain_config::write(&dir, &chain_spec, Some(&settlement))
            .context("failed to write chain spec file")?;

        // ----- Print initialization summary -----

        println!(
            r"
CHAIN
=====

| Chain ID        | {chain_id} ({chain_id_felt:#x})
| Mode            | Sovereign
| Config file     | {config_path}
| Genesis file    | {genesis_path}
",
            chain_id = output.id,
            chain_id_felt = Felt::from(output.id),
            config_path = dir.config_path().display(),
            genesis_path = dir.genesis_path().display(),
        );

        Ok(())
    }

    fn configure_from_args(&self) -> Option<SovereignOutcome> {
        self.id.map(|id| SovereignOutcome {
            id,
            #[cfg(feature = "init-slot")]
            slot_paymasters: self.slot.paymaster_accounts.clone(),
        })
    }
}

#[derive(Debug)]
struct SovereignOutcome {
    /// The id of the new chain to be initialized.
    pub id: ShortString,

    #[cfg(feature = "init-slot")]
    pub slot_paymasters: Option<Vec<slot::PaymasterAccountArgs>>,
}

#[derive(Debug)]
struct PersistentOutcome {
    // the id of the new chain to be initialized.
    pub id: ShortString,

    // the chain id of the settlement layer.
    pub settlement_id: ShortString,

    // the rpc url for the settlement layer.
    pub rpc_url: Url,

    pub deployment_outcome: DeploymentOutcome,

    /// The fact registry address that was wired into the Piltover core contract via
    /// `set_facts_registry(...)`. In ZK mode this is the Herodotus Atlantic integrity contract;
    /// in TEE mode it is the `IAMDTeeRegistry` contract.
    pub effective_fact_registry: Felt,

    /// The proof implementation the chain was initialized with. Sourced from `--tee` for the
    /// CLI-flag path and from the interactive proof-mode + variant prompts for the prompt path.
    pub proof_impl: ProofImpl,

    #[cfg(feature = "init-slot")]
    pub slot_paymasters: Option<Vec<slot::PaymasterAccountArgs>>,
}

/// A specific proof implementation. Each variant belongs to one of the two top-level proof
/// categories — Validity Proof or TEE — exposed via [`ProofImpl::category_label`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ProofImpl {
    /// STARK proofs verified via Herodotus Atlantic on the settlement chain.
    Stark,
    /// AMD SEV-SNP attestations verified via SP1 Groth16 in the IAMDTeeRegistry contract.
    AmdSevSnpSp1Groth16,
    /// Mock TEE: attestations are accepted by a mock registry that skips real SEV-SNP / SP1
    /// Groth16 verification (pairs with the mock prover, `saya-tee --mock-prove`). On-chain this
    /// is configured identically to a real TEE rollup (`proof_kind = "tee"`); only the registry
    /// address differs.
    MockTee,
}

impl ProofImpl {
    pub(super) fn category_label(self) -> &'static str {
        match self {
            Self::Stark => "Validity Proof",
            Self::AmdSevSnpSp1Groth16 | Self::MockTee => "TEE",
        }
    }

    pub(super) fn implementation_label(self) -> &'static str {
        match self {
            Self::Stark => "STARK (Atlantic)",
            Self::AmdSevSnpSp1Groth16 => "AMD SEV-SNP + SP1 Groth16",
            Self::MockTee => "Mock (no attestation verification)",
        }
    }

    pub(super) fn is_tee(self) -> bool {
        matches!(self, Self::AmdSevSnpSp1Groth16 | Self::MockTee)
    }
}

/// Selects the fact-registry address that Piltover's `set_facts_registry(...)` will be wired to,
/// and that `check_program_info` will validate against.
///
/// Precedence:
/// 1. TEE mode (`--tee`): `--tee-registry-address`.
/// 2. ZK mode with `--settlement-facts-registry` override: that address.
/// 3. ZK mode default: the provider's built-in Herodotus Atlantic address.
///
/// Clap enforces `--tee` + `--settlement-facts-registry` as a mutual-exclusion conflict, so the
/// `(true, _, Some(..))` combination cannot occur in practice. The match still defines it — TEE
/// wins — so the helper is total for unit tests.
fn resolve_effective_fact_registry(
    tee: bool,
    tee_registry: Option<ContractAddress>,
    facts_override: Option<ContractAddress>,
    provider_default: Felt,
) -> Felt {
    match (tee, tee_registry, facts_override) {
        (true, Some(addr), _) => *addr,
        (false, _, Some(addr)) => *addr,
        _ => provider_default,
    }
}

fn generate_genesis(num_accounts: u16) -> Genesis {
    let accounts = DevAllocationsGenerator::new(num_accounts)
        .with_balance(U256::from(DEFAULT_PREFUNDED_ACCOUNT_BALANCE))
        .generate();
    let mut genesis = Genesis::default();
    genesis.extend_allocations(accounts.into_iter().map(|(k, v)| (k, v.into())));
    genesis
}

#[derive(Debug, thiserror::Error)]
#[error("Unsupported settlement chain: {id}")]
struct SettlementChainTryFromStrError {
    id: String,
}

/// Supported settlement chain options for rollup initialization.
#[derive(Debug, Clone, strum_macros::Display, PartialEq, Eq)]
enum SettlementChain {
    Mainnet,
    Sepolia,
    #[cfg(feature = "init-custom-settlement-chain")]
    Custom(Url),
}

impl std::str::FromStr for SettlementChain {
    type Err = SettlementChainTryFromStrError;

    fn from_str(s: &str) -> Result<SettlementChain, <Self as ::core::str::FromStr>::Err> {
        let id = s.to_lowercase();
        if &id == "sepolia" || &id == "sn_sepolia" {
            return Ok(SettlementChain::Sepolia);
        }

        if &id == "mainnet" || &id == "sn_mainnet" {
            return Ok(SettlementChain::Mainnet);
        }

        #[cfg(feature = "init-custom-settlement-chain")]
        if let Ok(url) = Url::parse(s) {
            return Ok(SettlementChain::Custom(url));
        };

        Err(SettlementChainTryFromStrError { id: s.to_string() })
    }
}

impl TryFrom<&str> for SettlementChain {
    type Error = SettlementChainTryFromStrError;

    fn try_from(s: &str) -> Result<SettlementChain, <Self as TryFrom<&str>>::Error> {
        SettlementChain::from_str(s)
    }
}

#[cfg(test)]
mod tests {
    use assert_matches::assert_matches;
    use clap::error::{ContextKind, ContextValue};
    use clap::Parser;
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case("sepolia", SettlementChain::Sepolia)]
    #[case("SEPOLIA", SettlementChain::Sepolia)]
    #[case("sn_sepolia", SettlementChain::Sepolia)]
    #[case("SN_SEPOLIA", SettlementChain::Sepolia)]
    #[case("mainnet", SettlementChain::Mainnet)]
    #[case("MAINNET", SettlementChain::Mainnet)]
    #[case("sn_mainnet", SettlementChain::Mainnet)]
    #[case("SN_MAINNET", SettlementChain::Mainnet)]
    fn test_chain_from_str(#[case] input: &str, #[case] expected: SettlementChain) {
        assert_matches!(SettlementChain::from_str(input), Ok(chain) if chain == expected);
    }

    #[test]
    fn invalid_chain() {
        assert!(SettlementChain::from_str("invalid_chain").is_err());
    }

    #[test]
    #[cfg(feature = "init-custom-settlement-chain")]
    fn custom_settlement_chain() {
        assert_matches!(
            SettlementChain::from_str("http://localhost:5050"),
            Ok(SettlementChain::Custom(actual_url)) => {
                assert_eq!(actual_url, Url::parse("http://localhost:5050").unwrap());
            }
        );
    }

    #[test]
    fn non_sovereign_requires_all_settlement_args() {
        #[derive(Parser)]
        struct Cli {
            #[command(flatten)]
            args: InitCommand,
        }

        // This should fail with the expected error message:-
        //
        // ```
        // error: the following required arguments were not provided:
        //   --settlement-chain <SETTLEMENT_CHAIN>
        //   --settlement-account-address <SETTLEMENT_ACCOUNT>
        //   --settlement-account-private-key <SETTLEMENT_ACCOUNT_PRIVATE_KEY>
        // ```
        match Cli::try_parse_from(["init", "rollup", "--id", "bruh"]) {
            Ok(..) => panic!("Expected parsing to fail with missing required arguments"),
            Err(err) => {
                if let ContextValue::Strings(values) = err.get(ContextKind::InvalidArg).unwrap() {
                    // Assert that the error message contains all the required arguments
                    assert!(values.contains(&"--settlement-chain <SETTLEMENT_CHAIN>".to_string()));
                    assert!(values.contains(
                        &"--settlement-account-address <SETTLEMENT_ACCOUNT>".to_string()
                    ));
                    assert!(values.contains(
                        &"--settlement-account-private-key <SETTLEMENT_ACCOUNT_PRIVATE_KEY>"
                            .to_string()
                    ));
                } else {
                    panic!("Expected InvalidArg context with Strings value");
                }
            }
        }
    }

    #[test]
    fn sovereign_does_not_require_settlement_args() {
        #[derive(Parser)]
        struct Cli {
            #[command(flatten)]
            args: InitCommand,
        }

        let result = Cli::parse_from(["init", "sovereign", "--id", "bruh"]);

        assert_matches!(result.args.mode, InitMode::Sovereign(config) => {
            assert_eq!(config.id, Some(ShortString::from_ascii("bruh")));
        });
    }

    #[test]
    fn cli_accept_custom_fact_registry() {
        #[derive(Parser)]
        struct Cli {
            #[command(flatten)]
            args: InitCommand,
        }

        let custom_settlement_fact_registry = "0x1234567890123456789012345678901234567890";
        let result = Cli::parse_from([
            "init",
            "rollup",
            "--id",
            "wot",
            "--settlement-chain",
            "sepolia",
            "--settlement-account-address",
            "0x1234567890123456789012345678901234567890",
            "--settlement-account-private-key",
            "0x1234567890123456789012345678901234567890",
            "--settlement-facts-registry",
            custom_settlement_fact_registry,
        ]);

        assert_matches!(result.args.mode, InitMode::Rollup(config) => {
            assert_eq!(
                config.settlement_facts_registry_contract,
                Some(ContractAddress::from_str(custom_settlement_fact_registry).unwrap())
            );
        });
    }

    #[test]
    fn cli_required_settlement_args_with_custom_fact_registry() {
        #[derive(Parser)]
        struct Cli {
            #[command(flatten)]
            args: InitCommand,
        }

        // This should fail with the expected error message:-
        //
        // ```
        // error: the following required arguments were not provided:
        //   --settlement-chain <SETTLEMENT_CHAIN>
        //   --settlement-account-address <SETTLEMENT_ACCOUNT>
        //   --settlement-account-private-key <SETTLEMENT_ACCOUNT_PRIVATE_KEY>
        // ```
        match Cli::try_parse_from([
            "init",
            "rollup",
            "--id",
            "wot",
            "--settlement-facts-registry",
            "0x1234567890123456789012345678901234567890",
        ]) {
            Ok(..) => panic!("Expected parsing to fail with missing required arguments"),
            Err(err) => {
                if let ContextValue::Strings(values) = err.get(ContextKind::InvalidArg).unwrap() {
                    // Assert that the error message contains all the required arguments
                    assert!(values.contains(&"--settlement-chain <SETTLEMENT_CHAIN>".to_string()));
                    assert!(values.contains(
                        &"--settlement-account-address <SETTLEMENT_ACCOUNT>".to_string()
                    ));
                    assert!(values.contains(
                        &"--settlement-account-private-key <SETTLEMENT_ACCOUNT_PRIVATE_KEY>"
                            .to_string()
                    ));
                } else {
                    panic!("Expected InvalidArg context with Strings value");
                }
            }
        }
    }

    #[cfg(feature = "init-custom-settlement-chain")]
    #[tokio::test]
    async fn cli_required_custom_fact_registry_for_custom_init_chain() {
        #[derive(Parser)]
        struct Cli {
            #[command(flatten)]
            args: InitCommand,
        }

        let result = Cli::parse_from([
            "init",
            "rollup",
            "--id",
            "wot",
            "--settlement-chain",
            "http://localhost:5050",
            "--settlement-account-address",
            "0x1234567890123456789012345678901234567890",
            "--settlement-account-private-key",
            "0x1234567890123456789012345678901234567890",
        ]);
        assert_matches!(result.args.mode, InitMode::Rollup(config) => {
            assert_eq!(config.settlement_facts_registry_contract, None);

            let configure_result = config.configure_from_args().await;
            assert!(configure_result.is_some());
            let configure_result = configure_result.unwrap();
            assert!(configure_result.is_err());
            assert_eq!(
                configure_result.unwrap_err().to_string(),
                "Specifying the facts registry contract (using `--settlement-facts-registry`) is \
                 required when settling on a custom chain"
            );
        });
    }

    #[test]
    fn mock_tee_is_a_tee_proof() {
        // The mock variant settles in TEE mode on-chain (`proof_kind = "tee"`); it differs from
        // the real AMD attestation only in which registry contract is wired in.
        assert!(ProofImpl::MockTee.is_tee());
        assert_eq!(ProofImpl::MockTee.category_label(), "TEE");
        assert_ne!(
            ProofImpl::MockTee.implementation_label(),
            ProofImpl::AmdSevSnpSp1Groth16.implementation_label()
        );
    }

    #[test]
    fn mock_tee_sepolia_registry_constant() {
        // The deployed mock AMD TEE registry on Starknet Sepolia (cartridge-gg/piltover).
        assert_eq!(
            MOCK_TEE_REGISTRY_SEPOLIA,
            katana_primitives::felt!(
                "0x037189b1807f1358074b70b3dc8ab79167bbf72cff1296286052f6dfe31c8f15"
            )
        );
    }
    #[test]
    fn cli_accept_tee_flag_with_registry() {
        #[derive(Parser)]
        struct Cli {
            #[command(flatten)]
            args: InitCommand,
        }

        let tee_registry = "0x1234567890123456789012345678901234567890";
        let result = Cli::parse_from([
            "init",
            "rollup",
            "--id",
            "tee-chain",
            "--settlement-chain",
            "sepolia",
            "--settlement-account-address",
            "0x1234567890123456789012345678901234567890",
            "--settlement-account-private-key",
            "0x1234567890123456789012345678901234567890",
            "--tee",
            "--tee-registry-address",
            tee_registry,
        ]);

        assert_matches!(result.args.mode, InitMode::Rollup(config) => {
            assert!(config.tee);
            assert_eq!(
                config.tee_registry_address,
                Some(ContractAddress::from_str(tee_registry).unwrap())
            );
        });
    }

    #[test]
    fn cli_tee_requires_tee_registry_address() {
        #[derive(Parser)]
        struct Cli {
            #[command(flatten)]
            args: InitCommand,
        }

        match Cli::try_parse_from([
            "init",
            "rollup",
            "--id",
            "tee-chain",
            "--settlement-chain",
            "sepolia",
            "--settlement-account-address",
            "0x1234567890123456789012345678901234567890",
            "--settlement-account-private-key",
            "0x1234567890123456789012345678901234567890",
            "--tee",
        ]) {
            Ok(..) => panic!("Expected --tee to require --tee-registry-address"),
            Err(err) => {
                let ctx = err.get(ContextKind::InvalidArg).unwrap();
                if let ContextValue::Strings(values) = ctx {
                    assert!(values.iter().any(|v| v.contains("--tee-registry-address")));
                } else {
                    panic!("Expected InvalidArg context with Strings value");
                }
            }
        }
    }

    #[test]
    fn cli_tee_registry_address_requires_tee() {
        #[derive(Parser)]
        struct Cli {
            #[command(flatten)]
            args: InitCommand,
        }

        // --tee-registry-address without --tee should fail.
        match Cli::try_parse_from([
            "init",
            "rollup",
            "--id",
            "tee-chain",
            "--settlement-chain",
            "sepolia",
            "--settlement-account-address",
            "0x1234567890123456789012345678901234567890",
            "--settlement-account-private-key",
            "0x1234567890123456789012345678901234567890",
            "--tee-registry-address",
            "0x1234567890123456789012345678901234567890",
        ]) {
            Ok(..) => panic!("Expected --tee-registry-address to require --tee"),
            Err(err) => assert!(err.to_string().contains("--tee")),
        }
    }

    #[test]
    fn cli_tee_conflicts_with_settlement_facts_registry() {
        #[derive(Parser)]
        struct Cli {
            #[command(flatten)]
            args: InitCommand,
        }

        match Cli::try_parse_from([
            "init",
            "rollup",
            "--id",
            "tee-chain",
            "--settlement-chain",
            "sepolia",
            "--settlement-account-address",
            "0x1234567890123456789012345678901234567890",
            "--settlement-account-private-key",
            "0x1234567890123456789012345678901234567890",
            "--tee",
            "--tee-registry-address",
            "0x1111111111111111111111111111111111111111",
            "--settlement-facts-registry",
            "0x2222222222222222222222222222222222222222",
        ]) {
            Ok(..) => panic!("Expected --tee and --settlement-facts-registry to conflict"),
            Err(err) => assert_eq!(err.kind(), clap::error::ErrorKind::ArgumentConflict),
        }
    }

    mod fact_registry_resolution {
        use katana_primitives::felt;
        use rstest::rstest;

        use super::*;

        const TEE: Felt = felt!("0xAAA");
        const OVR: Felt = felt!("0xBBB");
        const DEF: Felt = felt!("0xCCC");

        #[rstest]
        #[case::tee_mode(true, Some(TEE), None, TEE)]
        #[case::plain_default(false, None, None, DEF)]
        #[case::custom_override(false, None, Some(OVR), OVR)]
        // Clap blocks this combination in practice, but the helper is total:
        #[case::tee_wins_over_override(true, Some(TEE), Some(OVR), TEE)]
        fn resolve_effective_fact_registry_cases(
            #[case] tee: bool,
            #[case] tee_addr: Option<Felt>,
            #[case] override_addr: Option<Felt>,
            #[case] expected: Felt,
        ) {
            let got = resolve_effective_fact_registry(
                tee,
                tee_addr.map(ContractAddress::from),
                override_addr.map(ContractAddress::from),
                DEF,
            );
            assert_eq!(got, expected);
        }
    }
}
