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

use clap::Args;
use katana_node::config::execution::{DEFAULT_INVOCATION_MAX_STEPS, DEFAULT_VALIDATION_MAX_STEPS};
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
use katana_primitives::genesis::Genesis;
#[cfg(feature = "server")]
use katana_rpc::cors::HeaderValue;
use katana_tracing::{gcloud, otlp, LogFormat, TracerConfig};
use serde::{Deserialize, Serialize};
use url::Url;

#[cfg(feature = "server")]
use crate::utils::{deserialize_cors_origins, serialize_cors_origins};
use crate::utils::{parse_block_hash_or_number, parse_genesis};

const DEFAULT_DEV_SEED: &str = "0";
const DEFAULT_DEV_ACCOUNTS: u16 = 10;

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
    /// Log format to use
    #[arg(long = "log.format", value_name = "FORMAT")]
    #[arg(default_value_t = LogFormat::Full)]
    pub log_format: LogFormat,
}
#[derive(Debug, Args, Default, Clone, Serialize, Deserialize, PartialEq)]
#[command(next_help_heading = "Gas Price Oracle Options")]
pub struct GasPriceOracleOptions {
    /// The L2 ETH gas price. (denominated in wei)
    #[arg(long = "gpo.l2-eth-gas-price", value_name = "WEI")]
    #[serde(serialize_with = "serialize_option_as_hex")]
    #[serde(deserialize_with = "deserialize_gas_price")]
    #[serde(default)]
    pub l2_eth_gas_price: Option<GasPrice>,

    /// The L2 STRK gas price. (denominated in fri)
    #[arg(long = "gpo.l2-strk-gas-price", value_name = "FRI")]
    #[serde(serialize_with = "serialize_option_as_hex")]
    #[serde(deserialize_with = "deserialize_gas_price")]
    #[serde(default)]
    pub l2_strk_gas_price: Option<GasPrice>,

    /// The L1 ETH gas price. (denominated in wei)
    #[arg(long = "gpo.l1-eth-gas-price", value_name = "WEI")]
    #[serde(serialize_with = "serialize_option_as_hex")]
    #[serde(default)]
    #[serde(deserialize_with = "deserialize_gas_price")]
    pub l1_eth_gas_price: Option<GasPrice>,

    /// The L1 STRK gas price. (denominated in fri)
    #[arg(long = "gpo.l1-strk-gas-price", value_name = "FRI")]
    #[serde(serialize_with = "serialize_option_as_hex")]
    #[serde(deserialize_with = "deserialize_gas_price")]
    #[serde(default)]
    pub l1_strk_gas_price: Option<GasPrice>,

    /// The L1 ETH data gas price. (denominated in wei)
    #[arg(long = "gpo.l1-eth-data-gas-price", value_name = "WEI")]
    #[serde(serialize_with = "serialize_option_as_hex")]
    #[serde(deserialize_with = "deserialize_gas_price")]
    #[serde(default)]
    pub l1_eth_data_gas_price: Option<GasPrice>,

    /// The L1 STRK data gas price. (denominated in fri)
    #[arg(long = "gpo.l1-strk-data-gas-price", value_name = "FRI")]
    #[serde(serialize_with = "serialize_option_as_hex")]
    #[serde(deserialize_with = "deserialize_gas_price")]
    #[serde(default)]
    pub l1_strk_data_gas_price: Option<GasPrice>,
}

#[cfg(feature = "cartridge")]
#[derive(Debug, Args, Clone, Serialize, Deserialize, PartialEq)]
#[command(next_help_heading = "Cartridge options")]
pub struct CartridgeOptions {
    /// Declare all versions of the Controller class at genesis. This is implictly enabled if
    /// `--cartridge.paymaster` is provided.
    #[arg(long = "cartridge.controllers")]
    pub controllers: bool,

    /// Whether to use the Cartridge paymaster.
    /// This has the cost to call the Cartridge API to check
    /// if a controller account exists on each estimate fee call.
    ///
    /// Mostly used for local development using controller, and must be
    /// disabled for slot deployments.
    #[arg(long = "cartridge.paymaster")]
    #[arg(default_value_t = false)]
    #[serde(default)]
    pub paymaster: bool,

    /// The root URL for the Cartridge API.
    ///
    /// This is used to fetch the calldata for the constructor of the given controller
    /// address (at the moment). Must be configurable for local development
    /// with local cartridge API.
    #[arg(long = "cartridge.api", requires = "paymaster")]
    #[arg(default_value = "https://api.cartridge.gg")]
    #[serde(default = "default_api_url")]
    pub api: Url,
}

#[cfg(feature = "cartridge")]
impl CartridgeOptions {
    pub fn merge(&mut self, other: Option<&Self>) {
        if let Some(other) = other {
            if self.paymaster == default_paymaster() {
                self.paymaster = other.paymaster;
            }

            if self.api == default_api_url() {
                self.api = other.api.clone();
            }
        }
    }
}

#[cfg(feature = "cartridge")]
impl Default for CartridgeOptions {
    fn default() -> Self {
        CartridgeOptions {
            controllers: false,
            paymaster: default_paymaster(),
            api: default_api_url(),
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

fn serialize_option_as_hex<S, T>(
    value: &Option<T>,
    serializer: S,
) -> std::result::Result<S::Ok, S::Error>
where
    S: serde::Serializer,
    T: serde::Serialize + std::fmt::LowerHex,
{
    match value {
        Some(value) => serializer.serialize_str(&format!("{value:#x}")),
        None => serializer.serialize_none(),
    }
}

#[cfg(feature = "cartridge")]
fn default_paymaster() -> bool {
    false
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
