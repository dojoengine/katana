//! Options related to the CLI and the configuration file parsing.
//!
//! The clap args are first parsed, then the configuration file is parsed.
//! If no configuration file is provided, the default values are used form the clap args.
//! If a configuration file is provided, the values are merged with the clap args, however, the clap
//! args keep the precedence.
//!
//! Currently, the merge is made at the top level of the commands.

#[cfg(feature = "server")]
use std::net::IpAddr;
use std::num::NonZeroU128;
use std::path::PathBuf;

use clap::Args;
use katana_genesis::Genesis;
use katana_node::config::execution::{DEFAULT_INVOCATION_MAX_STEPS, DEFAULT_VALIDATION_MAX_STEPS};
#[cfg(feature = "server")]
use katana_node::config::gateway::{
    DEFAULT_GATEWAY_ADDR, DEFAULT_GATEWAY_PORT, DEFAULT_GATEWAY_TIMEOUT_SECS,
};
#[cfg(feature = "server")]
use katana_node::config::metrics::{DEFAULT_METRICS_ADDR, DEFAULT_METRICS_PORT};
#[cfg(feature = "server")]
use katana_node::config::rpc::{RpcModulesList, DEFAULT_RPC_MAX_PROOF_KEYS};
#[cfg(feature = "server")]
use katana_node::config::rpc::{
    DEFAULT_RPC_ADDR, DEFAULT_RPC_MAX_CALL_GAS, DEFAULT_RPC_MAX_EVENT_PAGE_SIZE, DEFAULT_RPC_PORT,
};
use katana_primitives::block::{BlockHashOrNumber, GasPrice};
use katana_primitives::chain::ChainId;
#[cfg(feature = "vrf")]
use katana_primitives::ContractAddress;
#[cfg(feature = "server")]
use katana_rpc_server::cors::HeaderValue;
use katana_tracing::{default_log_file_directory, gcloud, otlp, LogColor, LogFormat, TracerConfig};
use serde::{Deserialize, Serialize};
use serde_utils::serialize_opt_as_hex;
use url::Url;

#[cfg(feature = "server")]
use crate::utils::{deserialize_cors_origins, serialize_cors_origins};
use crate::utils::{parse_block_hash_or_number, parse_genesis};

const DEFAULT_DEV_SEED: &str = "0";
const DEFAULT_DEV_ACCOUNTS: u16 = 10;
const DEFAULT_LOG_FILE_MAX_FILES: usize = 7;

#[cfg(feature = "server")]
#[derive(Debug, Args, Clone, Serialize, Deserialize, PartialEq)]
#[command(next_help_heading = "Metrics options")]
pub struct MetricsOptions {
    /// Enable metrics.
    ///
    /// For now, metrics will still be collected even if this flag is not set. This only
    /// controls whether the metrics server is started or not.
    #[arg(long)]
    #[serde(default)]
    pub metrics: bool,

    /// The metrics will be served at the given address.
    #[arg(requires = "metrics")]
    #[arg(long = "metrics.addr", value_name = "ADDRESS")]
    #[arg(default_value_t = DEFAULT_METRICS_ADDR)]
    #[serde(default = "default_metrics_addr")]
    pub metrics_addr: IpAddr,

    /// The metrics will be served at the given port.
    #[arg(requires = "metrics")]
    #[arg(long = "metrics.port", value_name = "PORT")]
    #[arg(default_value_t = DEFAULT_METRICS_PORT)]
    #[serde(default = "default_metrics_port")]
    pub metrics_port: u16,
}

#[cfg(feature = "server")]
impl Default for MetricsOptions {
    fn default() -> Self {
        MetricsOptions {
            metrics: false,
            metrics_addr: DEFAULT_METRICS_ADDR,
            metrics_port: DEFAULT_METRICS_PORT,
        }
    }
}

#[cfg(feature = "server")]
#[derive(Debug, Args, Clone, Serialize, Deserialize, PartialEq)]
#[command(next_help_heading = "Gateway options")]
pub struct GatewayOptions {
    /// Enable the gateway server.
    #[arg(long = "gateway")]
    #[serde(default)]
    pub gateway_enable: bool,

    /// Gateway server listening interface.
    #[arg(requires = "gateway_enable")]
    #[arg(long = "gateway.addr", value_name = "ADDRESS")]
    #[arg(default_value_t = DEFAULT_GATEWAY_ADDR)]
    #[serde(default = "default_feeder_gateway_addr")]
    pub gateway_addr: IpAddr,

    /// Gateway server listening port.
    #[arg(requires = "gateway_enable")]
    #[arg(long = "gateway.port", value_name = "PORT")]
    #[arg(default_value_t = DEFAULT_GATEWAY_PORT)]
    #[serde(default = "default_feeder_gateway_port")]
    pub gateway_port: u16,

    /// Timeout for gateway requests (in seconds).
    #[arg(requires = "gateway_enable")]
    #[arg(long = "gateway.timeout", value_name = "TIMEOUT")]
    #[arg(default_value_t = DEFAULT_GATEWAY_TIMEOUT_SECS)]
    #[serde(default = "default_feeder_gateway_timeout")]
    pub gateway_timeout: u64,
}

#[cfg(feature = "server")]
impl Default for GatewayOptions {
    fn default() -> Self {
        GatewayOptions {
            gateway_enable: false,
            gateway_addr: DEFAULT_GATEWAY_ADDR,
            gateway_port: DEFAULT_GATEWAY_PORT,
            gateway_timeout: DEFAULT_GATEWAY_TIMEOUT_SECS,
        }
    }
}

#[cfg(feature = "server")]
#[derive(Debug, Args, Clone, Serialize, Deserialize, PartialEq)]
#[command(next_help_heading = "Server options")]
pub struct ServerOptions {
    /// HTTP-RPC server listening interface.
    #[arg(long = "http.addr", value_name = "ADDRESS")]
    #[arg(default_value_t = DEFAULT_RPC_ADDR)]
    #[serde(default = "default_http_addr")]
    pub http_addr: IpAddr,

    /// HTTP-RPC server listening port.
    #[arg(long = "http.port", value_name = "PORT")]
    #[arg(default_value_t = DEFAULT_RPC_PORT)]
    #[serde(default = "default_http_port")]
    pub http_port: u16,

    /// Comma separated list of domains from which to accept cross origin requests.
    #[arg(long = "http.cors_origins")]
    #[arg(value_delimiter = ',')]
    #[serde(
        default,
        serialize_with = "serialize_cors_origins",
        deserialize_with = "deserialize_cors_origins"
    )]
    pub http_cors_origins: Vec<HeaderValue>,

    /// API's offered over the HTTP-RPC interface.
    #[arg(long = "http.api", value_name = "MODULES")]
    #[arg(value_parser = RpcModulesList::parse)]
    #[serde(default)]
    pub http_modules: Option<RpcModulesList>,

    /// Maximum number of concurrent connections allowed.
    #[arg(long = "rpc.max-connections", value_name = "MAX")]
    pub max_connections: Option<u32>,

    /// Maximum request body size (in bytes).
    #[arg(long = "rpc.max-request-body-size", value_name = "SIZE")]
    pub max_request_body_size: Option<u32>,

    /// Maximum response body size (in bytes).
    #[arg(long = "rpc.max-response-body-size", value_name = "SIZE")]
    pub max_response_body_size: Option<u32>,

    /// Timeout for the RPC server request (in seconds).
    #[arg(long = "rpc.timeout", value_name = "TIMEOUT")]
    pub timeout: Option<u64>,

    /// Maximum page size for event queries.
    #[arg(long = "rpc.max-event-page-size", value_name = "SIZE")]
    #[arg(default_value_t = DEFAULT_RPC_MAX_EVENT_PAGE_SIZE)]
    #[serde(default = "default_page_size")]
    pub max_event_page_size: u64,

    /// Maximum keys for requesting storage proofs.
    #[arg(long = "rpc.max-proof-keys", value_name = "SIZE")]
    #[arg(default_value_t = DEFAULT_RPC_MAX_PROOF_KEYS)]
    #[serde(default = "default_proof_keys")]
    pub max_proof_keys: u64,

    /// Maximum gas for the `starknet_call` RPC method.
    #[arg(long = "rpc.max-call-gas", value_name = "GAS")]
    #[arg(default_value_t = DEFAULT_RPC_MAX_CALL_GAS)]
    #[serde(default = "default_max_call_gas")]
    pub max_call_gas: u64,
}

#[cfg(feature = "server")]
impl Default for ServerOptions {
    fn default() -> Self {
        ServerOptions {
            http_addr: DEFAULT_RPC_ADDR,
            http_port: DEFAULT_RPC_PORT,
            http_cors_origins: Vec::new(),
            http_modules: None,
            max_event_page_size: DEFAULT_RPC_MAX_EVENT_PAGE_SIZE,
            max_proof_keys: DEFAULT_RPC_MAX_PROOF_KEYS,
            max_connections: None,
            max_request_body_size: None,
            max_response_body_size: None,
            timeout: None,
            max_call_gas: DEFAULT_RPC_MAX_CALL_GAS,
        }
    }
}

#[cfg(feature = "server")]
impl ServerOptions {
    pub fn merge(&mut self, other: Option<&Self>) {
        if let Some(other) = other {
            if self.http_addr == DEFAULT_RPC_ADDR {
                self.http_addr = other.http_addr;
            }
            if self.http_port == DEFAULT_RPC_PORT {
                self.http_port = other.http_port;
            }
            if self.http_cors_origins.is_empty() {
                self.http_cors_origins = other.http_cors_origins.clone();
            }
            if self.http_modules.is_none() {
                self.http_modules = other.http_modules.clone();
            }
            if self.max_connections.is_none() {
                self.max_connections = other.max_connections;
            }
            if self.max_request_body_size.is_none() {
                self.max_request_body_size = other.max_request_body_size;
            }
            if self.max_response_body_size.is_none() {
                self.max_response_body_size = other.max_response_body_size;
            }
            if self.timeout.is_none() {
                self.timeout = other.timeout;
            }
            if self.max_event_page_size == DEFAULT_RPC_MAX_EVENT_PAGE_SIZE {
                self.max_event_page_size = other.max_event_page_size;
            }
            if self.max_proof_keys == DEFAULT_RPC_MAX_PROOF_KEYS {
                self.max_proof_keys = other.max_proof_keys;
            }
            if self.max_call_gas == DEFAULT_RPC_MAX_CALL_GAS {
                self.max_call_gas = other.max_call_gas;
            }
        }
    }
}

#[derive(Debug, Args, Clone, Serialize, Deserialize, Default, PartialEq)]
#[command(next_help_heading = "Starknet options")]
pub struct StarknetOptions {
    #[command(flatten)]
    #[serde(rename = "env")]
    pub environment: EnvironmentOptions,

    #[arg(long)]
    #[arg(value_parser = parse_genesis)]
    #[arg(conflicts_with_all(["seed", "total_accounts", "chain"]))]
    pub genesis: Option<Genesis>,
}

impl StarknetOptions {
    pub fn merge(&mut self, other: Option<&Self>) {
        if let Some(other) = other {
            self.environment.merge(Some(&other.environment));

            if self.genesis.is_none() {
                self.genesis = other.genesis.clone();
            }
        }
    }
}

#[derive(Debug, Args, Clone, Serialize, Deserialize, PartialEq)]
#[command(next_help_heading = "Environment options")]
pub struct EnvironmentOptions {
    /// The chain ID.
    ///
    /// The chain ID. If a raw hex string (`0x` prefix) is provided, then it'd
    /// used as the actual chain ID. Otherwise, it's represented as the raw
    /// ASCII values. It must be a valid Cairo short string.
    #[arg(long, conflicts_with = "chain")]
    #[arg(value_parser = ChainId::parse)]
    #[serde(default)]
    pub chain_id: Option<ChainId>,

    /// The maximum number of steps available for the account validation logic.
    #[arg(long)]
    #[arg(default_value_t = DEFAULT_VALIDATION_MAX_STEPS)]
    #[serde(default = "default_validate_max_steps")]
    pub validate_max_steps: u32,

    /// The maximum number of steps available for the account execution logic.
    #[arg(long)]
    #[arg(default_value_t = DEFAULT_INVOCATION_MAX_STEPS)]
    #[serde(default = "default_invoke_max_steps")]
    pub invoke_max_steps: u32,

    /// Enable cairo-native compilation for improved performance.
    #[cfg(feature = "native")]
    #[arg(long = "enable-native-compilation")]
    #[serde(default)]
    pub compile_native: bool,
}

impl Default for EnvironmentOptions {
    fn default() -> Self {
        EnvironmentOptions {
            validate_max_steps: DEFAULT_VALIDATION_MAX_STEPS,
            invoke_max_steps: DEFAULT_INVOCATION_MAX_STEPS,
            chain_id: None,
            #[cfg(feature = "native")]
            compile_native: false,
        }
    }
}

impl EnvironmentOptions {
    pub fn merge(&mut self, other: Option<&Self>) {
        if let Some(other) = other {
            if self.validate_max_steps == DEFAULT_VALIDATION_MAX_STEPS {
                self.validate_max_steps = other.validate_max_steps;
            }

            if self.invoke_max_steps == DEFAULT_INVOCATION_MAX_STEPS {
                self.invoke_max_steps = other.invoke_max_steps;
            }

            #[cfg(feature = "native")]
            if !self.compile_native {
                self.compile_native = other.compile_native;
            }
        }
    }
}

#[derive(Debug, Args, Clone, Serialize, Deserialize, PartialEq)]
#[command(next_help_heading = "Development options")]
#[serde(rename = "dev")]
pub struct DevOptions {
    /// Enable development mode.
    #[arg(long)]
    #[serde(default)]
    pub dev: bool,

    /// Specify the seed for randomness of accounts to be predeployed.
    #[arg(requires = "dev")]
    #[arg(long = "dev.seed", default_value = DEFAULT_DEV_SEED)]
    #[serde(default = "default_seed")]
    pub seed: String,

    /// Number of pre-funded accounts to generate.
    #[arg(requires = "dev")]
    #[arg(long = "dev.accounts", value_name = "NUM")]
    #[arg(default_value_t = DEFAULT_DEV_ACCOUNTS)]
    #[serde(default = "default_accounts")]
    pub total_accounts: u16,

    /// Disable charging fee when executing transactions.
    #[arg(requires = "dev")]
    #[arg(long = "dev.no-fee")]
    #[serde(default)]
    pub no_fee: bool,

    /// Disable account validation when executing transactions.
    ///
    /// Skipping the transaction sender's account validation function.
    #[arg(requires = "dev")]
    #[arg(long = "dev.no-account-validation")]
    #[serde(default)]
    pub no_account_validation: bool,
}

impl Default for DevOptions {
    fn default() -> Self {
        DevOptions {
            dev: false,
            seed: DEFAULT_DEV_SEED.to_string(),
            total_accounts: DEFAULT_DEV_ACCOUNTS,
            no_fee: false,
            no_account_validation: false,
        }
    }
}

impl DevOptions {
    pub fn merge(&mut self, other: Option<&Self>) {
        if let Some(other) = other {
            if !self.dev {
                self.dev = other.dev;
            }

            if self.seed == DEFAULT_DEV_SEED {
                self.seed = other.seed.clone();
            }

            if self.total_accounts == DEFAULT_DEV_ACCOUNTS {
                self.total_accounts = other.total_accounts;
            }

            if !self.no_fee {
                self.no_fee = other.no_fee;
            }

            if !self.no_account_validation {
                self.no_account_validation = other.no_account_validation;
            }
        }
    }
}

#[derive(Debug, Args, Clone, Serialize, Deserialize, Default, PartialEq)]
#[command(next_help_heading = "Forking options")]
pub struct ForkingOptions {
    /// The RPC URL of the network to fork from.
    ///
    /// This will operate Katana in forked mode. Continuing from the tip of the forked network, or
    /// at a specific block if `fork.block` is provided.
    #[arg(long = "fork.provider", value_name = "URL", conflicts_with = "genesis")]
    pub fork_provider: Option<Url>,

    /// Fork the network at a specific block id, can either be a hash (0x-prefixed) or a block
    /// number.
    #[arg(long = "fork.block", value_name = "BLOCK", requires = "fork_provider")]
    #[arg(value_parser = parse_block_hash_or_number)]
    pub fork_block: Option<BlockHashOrNumber>,
}

#[derive(Debug, Args, Clone, Serialize, Deserialize, Default, PartialEq)]
#[command(next_help_heading = "Logging options")]
pub struct LoggingOptions {
    #[command(flatten)]
    pub stdout: StdoutLoggingOptions,

    #[command(flatten)]
    pub file: FileLoggingOptions,
}

#[derive(Debug, Args, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct StdoutLoggingOptions {
    #[arg(long = "log.stdout.format", value_name = "FORMAT")]
    #[arg(default_value_t = LogFormat::Full)]
    pub stdout_format: LogFormat,

    /// Sets whether or not the formatter emits ANSI terminal escape codes for colors and other
    /// text formatting
    ///
    /// Possible values:
    /// - always: Colors on
    /// - auto:   Auto-detect
    /// - never:  Colors off
    #[arg(long = "color", value_name = "COLOR")]
    #[arg(default_value_t = LogColor::Always)]
    pub color: LogColor,
}

#[derive(Debug, Args, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct FileLoggingOptions {
    /// Enable writing logs to files.
    #[arg(long = "log.file")]
    #[serde(default)]
    pub enabled: bool,

    #[arg(requires = "enabled")]
    #[arg(long = "log.file.format", value_name = "FORMAT")]
    #[arg(default_value_t = LogFormat::Full)]
    pub file_format: LogFormat,

    /// The path to put log files in
    #[arg(requires = "enabled")]
    #[arg(long = "log.file.directory", value_name = "PATH")]
    #[arg(default_value_os_t = default_log_file_directory())]
    #[serde(default = "default_log_file_directory")]
    pub directory: PathBuf,

    /// Maximum number of daily log files to keep.
    ///
    /// If `0` is supplied, no files are deleted (unlimited retention).
    #[arg(requires = "enabled")]
    #[arg(long = "log.file.max-files", value_name = "COUNT")]
    #[arg(default_value_t = DEFAULT_LOG_FILE_MAX_FILES)]
    pub max_files: usize,
}

#[derive(Debug, Args, Default, Clone, Serialize, Deserialize, PartialEq)]
#[command(next_help_heading = "Gas Price Oracle Options")]
pub struct GasPriceOracleOptions {
    /// The L2 ETH gas price. (denominated in wei)
    #[arg(long = "gpo.l2-eth-gas-price", value_name = "WEI")]
    #[serde(serialize_with = "serialize_opt_as_hex")]
    #[serde(deserialize_with = "deserialize_gas_price")]
    #[serde(default)]
    pub l2_eth_gas_price: Option<GasPrice>,

    /// The L2 STRK gas price. (denominated in fri)
    #[arg(long = "gpo.l2-strk-gas-price", value_name = "FRI")]
    #[serde(serialize_with = "serialize_opt_as_hex")]
    #[serde(deserialize_with = "deserialize_gas_price")]
    #[serde(default)]
    pub l2_strk_gas_price: Option<GasPrice>,

    /// The L1 ETH gas price. (denominated in wei)
    #[arg(long = "gpo.l1-eth-gas-price", value_name = "WEI")]
    #[serde(serialize_with = "serialize_opt_as_hex")]
    #[serde(default)]
    #[serde(deserialize_with = "deserialize_gas_price")]
    pub l1_eth_gas_price: Option<GasPrice>,

    /// The L1 STRK gas price. (denominated in fri)
    #[arg(long = "gpo.l1-strk-gas-price", value_name = "FRI")]
    #[serde(serialize_with = "serialize_opt_as_hex")]
    #[serde(deserialize_with = "deserialize_gas_price")]
    #[serde(default)]
    pub l1_strk_gas_price: Option<GasPrice>,

    /// The L1 ETH data gas price. (denominated in wei)
    #[arg(long = "gpo.l1-eth-data-gas-price", value_name = "WEI")]
    #[serde(serialize_with = "serialize_opt_as_hex")]
    #[serde(deserialize_with = "deserialize_gas_price")]
    #[serde(default)]
    pub l1_eth_data_gas_price: Option<GasPrice>,

    /// The L1 STRK data gas price. (denominated in fri)
    #[arg(long = "gpo.l1-strk-data-gas-price", value_name = "FRI")]
    #[serde(serialize_with = "serialize_opt_as_hex")]
    #[serde(deserialize_with = "deserialize_gas_price")]
    #[serde(default)]
    pub l1_strk_data_gas_price: Option<GasPrice>,
}

#[cfg(feature = "cartridge")]
#[derive(Debug, Args, Clone, Serialize, Deserialize, PartialEq)]
#[command(next_help_heading = "Cartridge options")]
pub struct CartridgeOptions {
    /// Declare all versions of the Controller class at genesis.
    #[arg(long = "cartridge.controllers")]
    pub controllers: bool,
}

#[cfg(feature = "paymaster")]
#[derive(Debug, Args, Clone, Serialize, Deserialize, PartialEq)]
#[command(next_help_heading = "Paymaster options")]
pub struct PaymasterOptions {
    /// Enable the paymaster service.
    ///
    /// By default, the paymaster runs as a sidecar process. If `--paymaster.url` is provided,
    /// it will connect to an external paymaster service instead.
    #[arg(long = "paymaster", id = "paymaster_enabled")]
    #[serde(default)]
    pub enabled: bool,

    /// Paymaster JSON-RPC endpoint for external paymaster service.
    ///
    /// When provided, the paymaster will run in external mode, connecting to this URL
    /// instead of spawning a sidecar process.
    #[arg(requires = "paymaster_enabled")]
    #[arg(long = "paymaster.url", value_name = "URL", id = "paymaster_url")]
    #[serde(default)]
    pub url: Option<Url>,

    /// API key to send via `x-paymaster-api-key` when proxying requests.
    #[arg(requires = "paymaster_url")]
    #[arg(long = "paymaster.api-key", value_name = "KEY")]
    #[serde(default)]
    pub api_key: Option<String>,

    /// API key for the Avnu price provider (used by the sidecar).
    ///
    /// Only required when running in sidecar mode. Not needed if `--paymaster.url` is provided.
    #[arg(requires = "paymaster_enabled")]
    #[arg(conflicts_with = "paymaster_url")]
    #[arg(long = "paymaster.price-api-key", value_name = "KEY")]
    #[serde(default)]
    pub price_api_key: Option<String>,

    /// Optional path to the paymaster sidecar binary (defaults to `paymaster-service` in PATH).
    ///
    /// Only used when running in sidecar mode. Not applicable if `--paymaster.url` is provided.
    #[arg(requires = "paymaster_enabled")]
    #[arg(conflicts_with = "paymaster_url")]
    #[arg(long = "paymaster.bin", value_name = "PATH", id = "paymaster_bin")]
    #[serde(default)]
    pub bin: Option<PathBuf>,

    /// Enable Cartridge paymaster
    #[cfg(feature = "cartridge")]
    #[arg(requires = "paymaster_enabled")]
    #[arg(long = "paymaster.cartridge")]
    #[serde(default)]
    pub cartridge_paymaster: bool,

    /// The base URL for the Cartridge API.
    ///
    /// This is used to fetch the calldata for the constructor of the given controller
    /// address (at the moment). Must be configurable for local development
    /// with local cartridge API.
    #[cfg(feature = "cartridge")]
    #[arg(long = "paymaster.cartridge-api")]
    #[arg(default_value = "https://api.cartridge.gg")]
    #[arg(requires_all = ["paymaster_enabled", "cartridge_paymaster"])]
    #[serde(default = "default_api_url")]
    pub cartridge_api: Url,

    #[cfg(feature = "vrf")]
    #[command(flatten)]
    pub vrf: VrfOptions,
}

#[cfg(feature = "paymaster")]
impl PaymasterOptions {
    /// Returns true if the paymaster should run in external mode (URL provided).
    pub fn is_external(&self) -> bool {
        self.url.is_some()
    }

    pub fn merge(&mut self, other: Option<&Self>) {
        if let Some(other) = other {
            if !self.enabled {
                self.enabled = other.enabled;
            }

            if self.url.is_none() {
                self.url = other.url.clone();
            }

            if self.api_key.is_none() {
                self.api_key = other.api_key.clone();
            }

            if self.price_api_key.is_none() {
                self.price_api_key = other.price_api_key.clone();
            }

            if self.bin.is_none() {
                self.bin = other.bin.clone();
            }

            if self.vrf == VrfOptions::default() {
                self.vrf = other.vrf.clone();
            }
        }
    }
}

#[cfg(feature = "vrf")]
#[derive(Debug, Default, Args, Clone, Serialize, Deserialize, PartialEq)]
#[command(next_help_heading = "VRF options")]
pub struct VrfOptions {
    /// Enable the VRF service.
    ///
    /// By default, the VRF runs as a sidecar process. If `--vrf.url` is provided,
    /// it will connect to an external VRF service instead.
    ///
    /// Requires the Cartridge paymaster to be enabled i.e., `--paymaster.cartridge`.
    #[arg(long = "vrf", id = "vrf_enabled")]
    #[arg(requires = "cartridge_paymaster")]
    #[serde(default)]
    pub enabled: bool,

    /// VRF service endpoint for external VRF service.
    ///
    /// When provided, the VRF will run in external mode, connecting to this URL
    /// instead of spawning a sidecar process.
    #[arg(requires_all = ["vrf_enabled", "vrf_account_contract"])]
    #[arg(long = "vrf.url", value_name = "URL", id = "vrf_url")]
    #[serde(default)]
    pub url: Option<Url>,

    #[arg(requires = "vrf_url")]
    #[arg(long = "vrf.contract", value_name = "ADDRESS", id = "vrf_account_contract")]
    #[serde(default)]
    pub vrf_account_contract: Option<ContractAddress>,

    /// Optional path to the VRF sidecar binary (defaults to `vrf-server` in PATH).
    ///
    /// Only used when running in sidecar mode. Not applicable if `--vrf.url` is provided.
    #[arg(requires = "vrf_enabled", conflicts_with = "vrf_url")]
    #[arg(long = "vrf.bin", value_name = "PATH", id = "vrf_bin")]
    #[serde(default)]
    pub bin: Option<PathBuf>,
}

#[cfg(feature = "vrf")]
impl VrfOptions {
    /// Returns true if the VRF should run in external mode (URL provided).
    pub fn is_external(&self) -> bool {
        self.url.is_some()
    }

    pub fn merge(&mut self, other: Option<&Self>) {
        if let Some(other) = other {
            if !self.enabled {
                self.enabled = other.enabled;
            }

            if self.url.is_none() {
                self.url = other.url.clone();
            }

            if self.bin.is_none() {
                self.bin = other.bin.clone();
            }
        }
    }
}

#[cfg(feature = "cartridge")]
impl CartridgeOptions {
    pub fn merge(&mut self, other: Option<&Self>) {
        if let Some(other) = other {
            if !self.controllers {
                self.controllers = other.controllers;
            }
        }
    }
}

#[cfg(feature = "cartridge")]
impl Default for CartridgeOptions {
    fn default() -> Self {
        CartridgeOptions { controllers: false }
    }
}

#[cfg(feature = "paymaster")]
impl Default for PaymasterOptions {
    fn default() -> Self {
        Self {
            enabled: false,
            url: None,
            api_key: None,
            bin: None,
            price_api_key: None,
            cartridge_paymaster: false,
            cartridge_api: default_api_url(),
            #[cfg(feature = "vrf")]
            vrf: VrfOptions::default(),
        }
    }
}

#[cfg(feature = "explorer")]
#[derive(Debug, Default, Args, Clone, Serialize, Deserialize, PartialEq)]
#[command(next_help_heading = "Explorer options")]
pub struct ExplorerOptions {
    /// Enable and launch the explorer frontend
    ///
    /// This will start a web server that serves the explorer UI.
    /// The explorer will be accessible at the `/explorer` path relative to the RPC server URL.
    ///
    /// For example, if the RPC server is running at `localhost:5050`, the explorer will be
    /// available at `localhost:5050/explorer`.
    #[arg(long)]
    #[serde(default)]
    pub explorer: bool,
}

// ** Default functions to setup serde of the configuration file **
fn default_seed() -> String {
    DEFAULT_DEV_SEED.to_string()
}

fn default_accounts() -> u16 {
    DEFAULT_DEV_ACCOUNTS
}

fn default_validate_max_steps() -> u32 {
    DEFAULT_VALIDATION_MAX_STEPS
}

fn default_invoke_max_steps() -> u32 {
    DEFAULT_INVOCATION_MAX_STEPS
}

#[cfg(feature = "server")]
fn default_http_addr() -> IpAddr {
    DEFAULT_RPC_ADDR
}

#[cfg(feature = "server")]
fn default_http_port() -> u16 {
    DEFAULT_RPC_PORT
}

#[cfg(feature = "server")]
fn default_page_size() -> u64 {
    DEFAULT_RPC_MAX_EVENT_PAGE_SIZE
}

#[cfg(feature = "server")]
fn default_proof_keys() -> u64 {
    katana_node::config::rpc::DEFAULT_RPC_MAX_PROOF_KEYS
}

#[cfg(feature = "server")]
fn default_feeder_gateway_addr() -> IpAddr {
    DEFAULT_GATEWAY_ADDR
}

#[cfg(feature = "server")]
fn default_feeder_gateway_port() -> u16 {
    DEFAULT_GATEWAY_PORT
}

#[cfg(feature = "server")]
fn default_feeder_gateway_timeout() -> u64 {
    DEFAULT_GATEWAY_TIMEOUT_SECS
}

#[cfg(feature = "server")]
fn default_metrics_addr() -> IpAddr {
    DEFAULT_METRICS_ADDR
}

#[cfg(feature = "server")]
fn default_metrics_port() -> u16 {
    DEFAULT_METRICS_PORT
}

#[cfg(feature = "server")]
fn default_max_call_gas() -> u64 {
    DEFAULT_RPC_MAX_CALL_GAS
}

/// Deserialize a string (hex or decimal) into a [`GasPrice`]
fn deserialize_gas_price<'de, D>(deserializer: D) -> Result<Option<GasPrice>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use std::str::FromStr;

    use serde::de::Error;

    let s = String::deserialize(deserializer)?;

    // Parse the string as u128 first, handling both hex and decimal formats
    let value = if let Some(s) = s.strip_prefix("0x") {
        u128::from_str_radix(s, 16).map_err(D::Error::custom)?
    } else {
        u128::from_str(&s).map_err(D::Error::custom)?
    };

    NonZeroU128::new(value)
        .map(GasPrice::new)
        .map(Some)
        .ok_or_else(|| D::Error::custom("value cannot be zero"))
}

#[cfg(feature = "cartridge")]
fn default_api_url() -> Url {
    Url::parse("https://api.cartridge.gg").expect("qed; invalid url")
}

#[derive(Debug, Default, Args, Clone, Serialize, Deserialize, PartialEq)]
#[command(next_help_heading = "Tracer options")]
pub struct TracerOptions {
    /// Enable Google Cloud Trace exporter
    #[arg(long = "tracer.gcloud")]
    #[arg(conflicts_with_all(["tracer_otlp", "otlp_endpoint"]))]
    #[serde(default)]
    pub tracer_gcloud: bool,

    /// Enable OpenTelemetry Protocol (OTLP) exporter
    #[arg(long = "tracer.otlp")]
    #[serde(default)]
    pub tracer_otlp: bool,

    /// Google Cloud project ID
    #[arg(long = "tracer.gcloud-project")]
    #[arg(requires = "tracer_gcloud", value_name = "PROJECT_ID")]
    #[arg(conflicts_with_all(["tracer_otlp", "otlp_endpoint"]))]
    #[serde(default)]
    pub gcloud_project_id: Option<String>,

    /// OTLP endpoint URL
    #[arg(long = "tracer.otlp-endpoint")]
    #[arg(requires = "tracer_otlp", value_name = "URL")]
    #[serde(default)]
    pub otlp_endpoint: Option<String>,
}

impl TracerOptions {
    /// Get the tracer configuration based on the options
    pub fn config(&self) -> Option<TracerConfig> {
        if self.tracer_gcloud {
            Some(TracerConfig::Gcloud(gcloud::GcloudConfig {
                project_id: self.gcloud_project_id.clone(),
            }))
        } else if self.tracer_otlp {
            Some(TracerConfig::Otlp(otlp::OtlpConfig { endpoint: self.otlp_endpoint.clone() }))
        } else {
            None
        }
    }

    pub fn merge(mut self, other: TracerOptions) -> Self {
        if other.tracer_gcloud {
            self.tracer_gcloud = other.tracer_gcloud;
        }

        if other.tracer_otlp {
            self.tracer_otlp = other.tracer_otlp;
        }

        if other.gcloud_project_id.is_some() {
            self.gcloud_project_id = other.gcloud_project_id;
        }

        if other.otlp_endpoint.is_some() {
            self.otlp_endpoint = other.otlp_endpoint;
        }
        self
    }
}

#[derive(Debug, Args, Clone, Serialize, Deserialize, PartialEq)]
#[command(next_help_heading = "Pruning options")]
pub struct PruningOptions {
    /// State pruning mode
    ///
    /// Determines how much historical state to retain:
    /// - 'archive': Keep all historical state (no pruning, default)
    /// - 'full:N': Keep last N blocks of historical state
    #[arg(long = "prune.mode", value_name = "MODE", default_value = "archive")]
    #[arg(value_parser = parse_pruning_mode)]
    pub mode: PruningMode,
}

impl Default for PruningOptions {
    fn default() -> Self {
        Self { mode: PruningMode::Archive }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum PruningMode {
    Archive,
    Full(u64),
}

fn parse_pruning_mode(s: &str) -> Result<PruningMode, String> {
    match s.to_lowercase().as_str() {
        "archive" => Ok(PruningMode::Archive),
        s if s.starts_with("full:") => {
            let n =
                s.strip_prefix("full:").and_then(|n| n.parse::<u64>().ok()).ok_or_else(|| {
                    "Invalid full format. Use 'full:N' where N is the number of blocks to keep"
                        .to_string()
                })?;
            Ok(PruningMode::Full(n))
        }
        _ => Err(format!("Invalid pruning mode '{s}'. Valid modes are: 'archive', 'full:N'")),
    }
}

#[cfg(feature = "tee")]
#[derive(Debug, Args, Clone, Serialize, Deserialize, Default, PartialEq)]
#[command(next_help_heading = "TEE options")]
pub struct TeeOptions {
    /// Enable TEE attestation support with AMD SEV-SNP.
    ///
    /// When enabled, the TEE RPC API becomes available for generating
    /// hardware-backed attestation quotes. Requires running in an SEV-SNP VM
    /// with /dev/sev-guest available.
    #[arg(long = "tee.provider", value_name = "PROVIDER")]
    #[serde(default)]
    pub tee_provider: Option<katana_tee::TeeProviderType>,
}

#[cfg(feature = "tee")]
impl TeeOptions {
    pub fn merge(&mut self, other: Option<&Self>) {
        if let Some(other) = other {
            if self.tee_provider.is_none() {
                self.tee_provider = other.tee_provider;
            }
        }
    }
}

#[cfg(all(feature = "server", feature = "grpc"))]
#[derive(Debug, Args, Clone, Serialize, Deserialize, PartialEq)]
#[command(next_help_heading = "gRPC server options")]
pub struct GrpcOptions {
    /// Enable the gRPC server.
    ///
    /// When enabled, the gRPC server will start alongside the JSON-RPC server,
    /// providing high-performance endpoints for Starknet operations.
    #[arg(long = "grpc")]
    #[serde(default)]
    pub grpc_enable: bool,

    /// gRPC server listening interface.
    #[arg(requires = "grpc_enable")]
    #[arg(long = "grpc.addr", value_name = "ADDRESS")]
    #[arg(default_value_t = default_grpc_addr())]
    #[serde(default = "default_grpc_addr")]
    pub grpc_addr: IpAddr,

    /// gRPC server listening port.
    #[arg(requires = "grpc_enable")]
    #[arg(long = "grpc.port", value_name = "PORT")]
    #[arg(default_value_t = default_grpc_port())]
    #[serde(default = "default_grpc_port")]
    pub grpc_port: u16,

    /// gRPC request timeout in seconds.
    #[arg(requires = "grpc_enable")]
    #[arg(long = "grpc.timeout", value_name = "TIMEOUT")]
    pub grpc_timeout: Option<u64>,
}

#[cfg(all(feature = "server", feature = "grpc"))]
impl Default for GrpcOptions {
    fn default() -> Self {
        GrpcOptions {
            grpc_enable: false,
            grpc_addr: default_grpc_addr(),
            grpc_port: default_grpc_port(),
            grpc_timeout: None,
        }
    }
}

#[cfg(all(feature = "server", feature = "grpc"))]
impl GrpcOptions {
    pub fn merge(&mut self, other: Option<&Self>) {
        if let Some(other) = other {
            if !self.grpc_enable {
                self.grpc_enable = other.grpc_enable;
            }
            if self.grpc_addr == default_grpc_addr() {
                self.grpc_addr = other.grpc_addr;
            }
            if self.grpc_port == default_grpc_port() {
                self.grpc_port = other.grpc_port;
            }
            if self.grpc_timeout.is_none() {
                self.grpc_timeout = other.grpc_timeout;
            }
        }
    }
}

#[cfg(all(feature = "server", feature = "grpc"))]
fn default_grpc_addr() -> IpAddr {
    katana_node::config::grpc::DEFAULT_GRPC_ADDR
}

#[cfg(all(feature = "server", feature = "grpc"))]
fn default_grpc_port() -> u16 {
    katana_node::config::grpc::DEFAULT_GRPC_PORT
}
