use std::path::Path;
use std::str::FromStr;

use crate::core_contract::constants::{
    ATLANTIC_FACT_REGISTRY_MAINNET, ATLANTIC_FACT_REGISTRY_SEPOLIA, DEFAULT_PILTOVER_CLASS_HASH,
    FACT_REGISTRY_MOCK_BYTES, KATANA_STRK_FEE_TOKEN, MAINNET_RPC_URL, PILTOVER_CONTRACT_BYTES,
    SEPOLIA_RPC_URL, TEE_REGISTRY_MOCK_BYTES,
};
use crate::core_contract::utils::{
    compute_starknet_os_config_hash, declare_contract, declare_contract_from_bytes,
    deploy_contract, deploy_core_contract, set_fact_registry, set_program_info,
};
use anyhow::Result;
use clap::{Args, Parser, Subcommand, ValueEnum};
use dojo_utils::TransactionResult;
use serde::Serialize;
use starknet::core::types::Felt;
use starknet::core::utils::cairo_short_string_to_felt;
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::{JsonRpcClient, Provider};
use starknet::signers::{LocalWallet, SigningKey};
use tracing::debug;
use url::Url;

/// Output format for the final result of a subcommand.
///
/// `Text` (the default) keeps the current behavior: human-readable `info!` logs
/// on stderr, nothing special on stdout.
///
/// `Json` additionally emits a single JSON object to stdout with the structured
/// result of the command (addresses, tx hashes, block numbers). Human logs
/// remain on stderr unchanged, so the stdout stream is safe to pipe into `jq`.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, ValueEnum)]
pub enum OutputFormat {
    #[default]
    Text,
    Json,
}

/// Supported settlement chain options for rollup initialization.
#[derive(Debug, Clone, PartialEq, Eq)]
enum SettlementChain {
    Mainnet,
    Sepolia,
    Custom(String),
}

impl FromStr for SettlementChain {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "mainnet" => Ok(SettlementChain::Mainnet),
            "sepolia" => Ok(SettlementChain::Sepolia),
            _ => Ok(SettlementChain::Custom(s.to_string())),
        }
    }
}
#[derive(Debug, Parser)]
pub struct CoreContract {
    #[clap(subcommand)]
    cmd: CoreContractCmd,
    #[clap(long, env = "SETTLEMENT_ACCOUNT_PRIVATE_KEY")]
    private_key: String,
    #[clap(long, env = "SETTLEMENT_ACCOUNT_ADDRESS")]
    account_address: String,
    #[clap(long, env = "SETTLEMENT_RPC_URL")]
    settlement_rpc_url: Option<Url>,
    #[clap(long, env = "SETTLEMENT_CHAIN_ID")]
    settlement_chain_id: SettlementChain,
    /// Output format for the structured result (`text` or `json`). See
    /// [`OutputFormat`]. `--output json` pipes one JSON object to stdout
    /// suitable for `jq`; all human logs stay on stderr.
    #[clap(long, value_enum, default_value_t = OutputFormat::Text, env = "SAYA_OPS_OUTPUT")]
    output: OutputFormat,
}

/// Structured result of a `core-contract` subcommand, emitted to stdout when
/// `--output json` is set. Downstream orchestrators (docker-compose init
/// containers, terraform external providers, CI scripts) consume this in place
/// of parsing `info!` log lines.
///
/// Felts are rendered as `0x`-prefixed lowercase hex strings. `tx_hash` and
/// `deployed_block` are `null` when the underlying operation was a no-op (e.g.
/// the contract was already declared or deployed at the same salt).
#[derive(Debug, Serialize)]
#[serde(tag = "command", rename_all = "kebab-case")]
pub enum CoreContractResult {
    Declare {
        class_hash: String,
        tx_hash: Option<String>,
        declared_block: Option<u64>,
    },
    Deploy {
        class_hash: String,
        contract_address: String,
        salt: String,
        tx_hash: Option<String>,
        deployed_block: Option<u64>,
    },
    DeclareAndDeployFactRegistryMock {
        class_hash: String,
        contract_address: String,
        salt: String,
        tx_hash: Option<String>,
        deployed_block: Option<u64>,
    },
    DeclareAndDeployTeeRegistryMock {
        class_hash: String,
        contract_address: String,
        salt: String,
        tx_hash: Option<String>,
        deployed_block: Option<u64>,
    },
    SetupProgram {
        core_contract_address: String,
        snos_config_hash: String,
        fact_registry_address: String,
        set_program_info_tx: Option<String>,
        set_program_info_block: Option<u64>,
        set_fact_registry_tx: Option<String>,
        set_fact_registry_block: Option<u64>,
    },
}

fn fhex(f: Felt) -> String {
    format!("{:#x}", f)
}

fn tx_outcome(r: &TransactionResult) -> (Option<String>, Option<u64>) {
    match r {
        TransactionResult::Noop => (None, None),
        TransactionResult::Hash(h) => (Some(fhex(*h)), None),
        TransactionResult::HashReceipt(h, receipt) => {
            (Some(fhex(*h)), Some(receipt.block.block_number()))
        }
    }
}

#[derive(Debug, Subcommand)]
pub enum CoreContractCmd {
    Declare(DeclareArgs),
    Deploy(DeployArgs),
    DeclareAndDeployFactRegistryMock(DeployFactRegistryArgs),
    DeclareAndDeployTeeRegistryMock(DeployTeeRegistryArgs),
    SetupProgram(SetupProgramArgs),
}

#[derive(Debug, Args)]
pub struct SetupProgramArgs {
    #[clap(long, env = "FACT_REGISTRY_ADDRESS")]
    fact_registry_address: Option<Felt>,
    #[clap(long, env = "CORE_CONTRACT_ADDRESS")]
    core_contract_address: Felt,
    #[clap(long, env = "FEE_TOKEN_ADDRESS", default_value = KATANA_STRK_FEE_TOKEN)]
    fee_token_address: Felt,
    #[clap(long, env = "CHAIN_ID")]
    chain_id: String,
}

#[derive(Debug, Args)]
pub struct DeployArgs {
    #[clap(long, env = "CLASS_HASH", default_value = DEFAULT_PILTOVER_CLASS_HASH)]
    class_hash: Felt,
    #[clap(long, env = "SALT")]
    salt: Felt,
}

#[derive(Debug, Args)]
pub struct DeployFactRegistryArgs {
    /// Path to the fact registry mock contract (optional, uses embedded contract by default)
    #[clap(long, env = "FACT_REGISTRY_PATH")]
    fact_registry_path: Option<String>,
    #[clap(long, env = "FACT_REGISTRY_SALT")]
    salt: Felt,
}

#[derive(Debug, Args)]
pub struct DeployTeeRegistryArgs {
    /// Path to the TEE registry mock contract (optional, uses embedded contract by default).
    ///
    /// The embedded contract is `piltover_mock_amd_tee_registry`, a permissive
    /// `IAMDTeeRegistry` implementation that bypasses on-chain Groth16
    /// verification of SP1 attestation proofs and round-trips a `VerifierJournal`
    /// through Cairo Serde. Used by e2e tests for `saya-tee --mock-prove`.
    #[clap(long, env = "TEE_REGISTRY_PATH")]
    tee_registry_path: Option<String>,
    #[clap(long, env = "TEE_REGISTRY_SALT")]
    salt: Felt,
}

#[derive(Debug, Args)]
pub struct DeclareArgs {
    /// Path to the core contract (optional, uses embedded contract by default)
    #[clap(long, env = "CORE_CONTRACT_PATH")]
    core_contract_path: Option<String>,
}

impl CoreContract {
    /// Validates that custom chains have required parameters
    fn validate(&self) -> Result<()> {
        if let SettlementChain::Custom(chain_name) = &self.settlement_chain_id {
            if self.settlement_rpc_url.is_none() {
                anyhow::bail!(
                    "Settlement RPC URL is required for custom chain '{}'. \
                     Provide it via --settlement-rpc-url or SETTLEMENT_RPC_URL env var",
                    chain_name
                );
            }

            // For SetupProgram command, also check fact registry
            if let CoreContractCmd::SetupProgram(ref args) = self.cmd {
                if args.fact_registry_address.is_none() {
                    anyhow::bail!(
                        "Fact registry address is required for custom chain '{}'. \
                         Provide it via --fact-registry-address or FACT_REGISTRY_ADDRESS env var",
                        chain_name
                    );
                }
            }
        }

        Ok(())
    }

    pub async fn run(self) -> Result<()> {
        // Validate dependencies before proceeding
        self.validate()?;

        let provider = self.get_provider();
        let signer: LocalWallet = LocalWallet::from_signing_key(SigningKey::from_secret_scalar(
            Felt::from_hex(&self.private_key).expect("Invalid private key"),
        ));

        let address = Felt::from_hex(&self.account_address).expect("Invalid address");

        let chain_id = provider.chain_id().await.expect("Failed to fetch chain id");
        let encoding = starknet::accounts::ExecutionEncoding::New;
        let account = starknet::accounts::SingleOwnerAccount::new(
            provider,
            signer.clone(),
            address,
            chain_id,
            encoding,
        );
        let result = match self.cmd {
            CoreContractCmd::Declare(declare_args) => {
                let (class_hash, tx) = if let Some(path) = declare_args.core_contract_path {
                    declare_contract(account.clone(), "Core contract", Path::new(&path)).await?
                } else {
                    declare_contract_from_bytes(
                        account.clone(),
                        "Core contract",
                        PILTOVER_CONTRACT_BYTES,
                    )
                    .await?
                };

                let (tx_hash, declared_block) = tx_outcome(&tx);
                CoreContractResult::Declare {
                    class_hash: fhex(class_hash),
                    tx_hash,
                    declared_block,
                }
            }
            CoreContractCmd::Deploy(deploy_args) => {
                let (contract_address, tx) = deploy_core_contract(
                    account.clone(),
                    "Core contract",
                    deploy_args.class_hash,
                    deploy_args.salt,
                )
                .await?;

                let (tx_hash, deployed_block) = tx_outcome(&tx);
                CoreContractResult::Deploy {
                    class_hash: fhex(deploy_args.class_hash),
                    contract_address: fhex(contract_address),
                    salt: fhex(deploy_args.salt),
                    tx_hash,
                    deployed_block,
                }
            }
            CoreContractCmd::DeclareAndDeployFactRegistryMock(deploy_fact_registry_args) => {
                let (class_hash, _declare_tx) =
                    if let Some(path) = deploy_fact_registry_args.fact_registry_path {
                        declare_contract(account.clone(), "Fact registry mock", Path::new(&path))
                            .await?
                    } else {
                        declare_contract_from_bytes(
                            account.clone(),
                            "Fact registry mock",
                            FACT_REGISTRY_MOCK_BYTES,
                        )
                        .await?
                    };

                let (fact_registry_address, deploy_tx) = deploy_contract(
                    account.clone(),
                    "Fact registry mock",
                    class_hash,
                    deploy_fact_registry_args.salt,
                    &[],
                )
                .await?;

                let (tx_hash, deployed_block) = tx_outcome(&deploy_tx);
                CoreContractResult::DeclareAndDeployFactRegistryMock {
                    class_hash: fhex(class_hash),
                    contract_address: fhex(fact_registry_address),
                    salt: fhex(deploy_fact_registry_args.salt),
                    tx_hash,
                    deployed_block,
                }
            }
            CoreContractCmd::DeclareAndDeployTeeRegistryMock(deploy_tee_registry_args) => {
                let (class_hash, _declare_tx) = if let Some(path) =
                    deploy_tee_registry_args.tee_registry_path
                {
                    declare_contract(account.clone(), "TEE registry mock", Path::new(&path)).await?
                } else {
                    declare_contract_from_bytes(
                        account.clone(),
                        "TEE registry mock",
                        TEE_REGISTRY_MOCK_BYTES,
                    )
                    .await?
                };

                let (tee_registry_address, deploy_tx) = deploy_contract(
                    account.clone(),
                    "TEE registry mock",
                    class_hash,
                    deploy_tee_registry_args.salt,
                    &[],
                )
                .await?;

                let (tx_hash, deployed_block) = tx_outcome(&deploy_tx);
                CoreContractResult::DeclareAndDeployTeeRegistryMock {
                    class_hash: fhex(class_hash),
                    contract_address: fhex(tee_registry_address),
                    salt: fhex(deploy_tee_registry_args.salt),
                    tx_hash,
                    deployed_block,
                }
            }
            CoreContractCmd::SetupProgram(ref setup_program_args) => {
                let chain_id = cairo_short_string_to_felt(&setup_program_args.chain_id)?;

                let snos_config_hash =
                    compute_starknet_os_config_hash(chain_id, setup_program_args.fee_token_address);
                debug!(snos_config_hash = ?snos_config_hash, "Computed Starknet OS config hash.");
                let set_program_tx = set_program_info(
                    account.clone(),
                    setup_program_args.core_contract_address,
                    snos_config_hash,
                )
                .await?;
                let fact_registry =
                    self.get_fact_registry_address(setup_program_args.fact_registry_address);
                debug!(tx = ?set_program_tx, "set_program_info transaction submitted.");
                let set_fact_tx = set_fact_registry(
                    account.clone(),
                    setup_program_args.core_contract_address,
                    fact_registry,
                )
                .await?;
                debug!(tx = ?set_fact_tx, "set_fact_registry transaction submitted.");
                let (spi_tx_hash, spi_block) = tx_outcome(&set_program_tx);
                let (sfr_tx_hash, sfr_block) = tx_outcome(&set_fact_tx);
                CoreContractResult::SetupProgram {
                    core_contract_address: fhex(setup_program_args.core_contract_address),
                    snos_config_hash: fhex(snos_config_hash),
                    fact_registry_address: fhex(fact_registry),
                    set_program_info_tx: spi_tx_hash,
                    set_program_info_block: spi_block,
                    set_fact_registry_tx: sfr_tx_hash,
                    set_fact_registry_block: sfr_block,
                }
            }
        };

        // Stdout carries the primary command result. Text mode prints
        // human-readable `Label: 0x...` lines so a user running the command
        // interactively can tell at a glance what each hex value is. Scripts
        // that need to parse reliably should use `--output json`.
        //
        // Diagnostic/progress logs live on stderr via `debug!` + `trace!`.
        match self.output {
            OutputFormat::Json => {
                println!("{}", serde_json::to_string(&result)?);
            }
            OutputFormat::Text => match &result {
                CoreContractResult::Declare { class_hash, .. } => {
                    println!("Core contract class hash: {}", class_hash);
                }
                CoreContractResult::Deploy {
                    contract_address, ..
                } => {
                    println!("Core contract address: {}", contract_address);
                }
                CoreContractResult::DeclareAndDeployFactRegistryMock {
                    class_hash,
                    contract_address,
                    ..
                } => {
                    println!("Fact registry mock class hash: {}", class_hash);
                    println!("Fact registry mock address: {}", contract_address);
                }
                CoreContractResult::DeclareAndDeployTeeRegistryMock {
                    class_hash,
                    contract_address,
                    ..
                } => {
                    println!("TEE registry mock class hash: {}", class_hash);
                    println!("TEE registry mock address: {}", contract_address);
                }
                CoreContractResult::SetupProgram { .. } => {
                    // Action, not query: success is implied by a zero exit
                    // code. Users who want the tx hashes can request them
                    // with `--output json`.
                }
            },
        }

        Ok(())
    }
    pub fn get_provider(&self) -> JsonRpcClient<HttpTransport> {
        match &self.settlement_chain_id {
            SettlementChain::Mainnet => {
                JsonRpcClient::new(HttpTransport::new(Url::parse(MAINNET_RPC_URL).unwrap()))
            }
            SettlementChain::Sepolia => {
                JsonRpcClient::new(HttpTransport::new(Url::parse(SEPOLIA_RPC_URL).unwrap()))
            }
            SettlementChain::Custom(_) => {
                // Safe to unwrap: validated in validate()
                JsonRpcClient::new(HttpTransport::new(self.settlement_rpc_url.clone().unwrap()))
            }
        }
    }
    pub fn get_fact_registry_address(&self, fact_registry_address: Option<Felt>) -> Felt {
        if let Some(addr) = fact_registry_address {
            return addr;
        }

        match &self.settlement_chain_id {
            SettlementChain::Mainnet => ATLANTIC_FACT_REGISTRY_MAINNET,
            SettlementChain::Sepolia => ATLANTIC_FACT_REGISTRY_SEPOLIA,
            SettlementChain::Custom(_) => {
                // Safe to unwrap: validated in validate() for SetupProgram command
                fact_registry_address.unwrap()
            }
        }
    }
}
