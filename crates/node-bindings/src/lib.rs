#![cfg_attr(not(test), warn(unused_crate_dependencies))]

//! Utilities for launching a Katana instance.

mod json;

use std::borrow::Cow;
use std::io::{BufRead, BufReader};
use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::process::{Child, Command};
use std::str::FromStr;
use std::time::{Duration, Instant};

pub use katana_tracing::LogFormat;
use starknet::core::types::{Felt, FromStrError};
use starknet::core::utils::cairo_short_string_to_felt;
use starknet::macros::short_string;
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::JsonRpcClient;
use starknet::signers::SigningKey;
use thiserror::Error;
use tracing::trace;
use url::Url;

use crate::json::{JsonLog, KatanaInfo};

/// How long we will wait for katana to indicate that it is ready.
const KATANA_STARTUP_TIMEOUT_MILLIS: u64 = 10_000;

/// Genesis accounts created by Katana
#[derive(Debug, Clone)]
pub struct Account {
    /// The account contract address
    pub address: Felt,
    /// The private key, if any
    pub private_key: Option<SigningKey>,
}

/// A katana CLI instance. Will close the instance when dropped.
///
/// Construct this using [`Katana`].
#[derive(Debug)]
pub struct KatanaInstance {
    port: u16,
    child: Child,
    accounts: Vec<Account>,
    chain_id: Felt,
}

impl KatanaInstance {
    /// Returns a reference to the child process.
    pub const fn child(&self) -> &Child {
        &self.child
    }

    /// Returns a mutable reference to the child process.
    pub fn child_mut(&mut self) -> &mut Child {
        &mut self.child
    }

    /// Returns the port of this instance
    pub const fn port(&self) -> u16 {
        self.port
    }

    /// Returns the chain of the katana instance
    pub fn chain_id(&self) -> Felt {
        self.chain_id
    }

    /// Returns the list of accounts created by this katana instance
    pub fn accounts(&self) -> &[Account] {
        &self.accounts
    }

    /// Returns the HTTP endpoint of JSON-RPC server of this instance
    pub fn rpc_addr(&self) -> SocketAddr {
        SocketAddr::new(std::net::IpAddr::V4(Ipv4Addr::LOCALHOST), self.port)
    }

    /// Returns a JSON-RPC client for the Starknet provider of this instance
    pub fn starknet_provider(&self) -> JsonRpcClient<HttpTransport> {
        let url = Url::parse(&format!("http://{}", self.rpc_addr())).expect("invalid url");
        JsonRpcClient::new(HttpTransport::new(url))
    }
}

impl Drop for KatanaInstance {
    fn drop(&mut self) {
        self.child.kill().expect("could not kill katana");
        let _ = self.child.wait().expect("could not kill katana");
    }
}

/// Errors that can occur when working with the [`Katana`].
#[derive(Debug, Error)]
pub enum Error {
    /// Spawning the katana process failed.
    #[error("could not start katana: {0}")]
    SpawnError(std::io::Error),

    /// Timed out waiting for a message from katana's stderr.
    #[error("timed out waiting for katana to spawn; is katana installed?")]
    Timeout,

    /// Unable to parse felt value from katana's stdout.
    #[error("failed to parse felt value from katana's stdout")]
    ParseFelt(#[from] FromStrError),

    /// A line could not be read from the katana stderr.
    #[error("could not read line from katana stderr: {0}")]
    ReadLineError(std::io::Error),

    /// The child katana process's stderr was not captured.
    #[error("could not get stderr for katana child process")]
    NoStderr,

    #[error("failed to parse instance address: {0}")]
    AddrParse(#[from] std::net::AddrParseError),

    /// A line indicating the account address was found but the actual value was not.
    #[error("missing account address")]
    MissingAccountAddress,

    /// A line indicating the private key was found but the actual value was not.
    #[error("missing account private key")]
    MissingAccountPrivateKey,

    /// A line indicating the instance address was found but the actual value was not.
    #[error("missing rpc server address")]
    MissingSocketAddr,

    /// A line indicating the instance address was found but the actual value was not.
    #[error("missing chain id")]
    MissingChainId,

    #[error("encountered unexpected format: {0}")]
    UnexpectedFormat(String),

    #[error("failed to match regex: {0}")]
    Regex(#[from] regex::Error),

    #[error("expected logs to be in JSON format: {0}")]
    ExpectedJsonFormat(#[from] serde_json::Error),
}

/// The string indicator from which the RPC server address can be extracted from.
const RPC_ADDR_LOG_SUBSTR: &str = "RPC server started.";
/// The string indicator from which the chain id can be extracted from.
const CHAIN_ID_LOG_SUBSTR: &str = "Starting node.";

/// Builder for launching `katana`.
///
/// # Panics
///
/// If `spawn` is called without `katana` being available in the user's $PATH
///
/// # Example
///
/// ```no_run
/// use katana_node_bindings::{Katana, LogFormat};
///
/// let port = 5050u16;
/// let url = format!("http://localhost:{}", port).to_string();
///
/// let katana = Katana::new().port(port).log_format(LogFormat::Json).spawn();
///
/// drop(katana); // this will kill the instance
/// ```
#[derive(Clone, Debug, Default)]
#[must_use = "This Builder struct does nothing unless it is `spawn`ed"]
pub struct Katana {
    // General options
    dev: bool,
    no_mining: bool,
    silent: bool,
    log_format: Option<LogFormat>,
    block_time: Option<u64>,
    sequencing_block_max_cairo_steps: Option<u64>,
    db_dir: Option<PathBuf>,
    config: Option<PathBuf>,
    messaging: Option<PathBuf>,

    // Forking options
    fork_provider: Option<String>,
    fork_block: Option<u64>,
    l1_provider: Option<String>, // Keep for backward compatibility

    // Tracer options
    tracer_gcloud: bool,
    tracer_otlp: bool,
    tracer_gcloud_project: Option<String>,
    tracer_otlp_endpoint: Option<String>,

    // Metrics options
    metrics: bool,
    metrics_addr: Option<SocketAddr>,
    metrics_port: Option<u16>,

    // Server options
    http_addr: Option<SocketAddr>,
    http_port: Option<u16>,
    http_cors_origins: Option<String>,
    http_api: Option<String>,
    rpc_max_connections: Option<u64>,
    rpc_max_request_body_size: Option<u64>,
    rpc_max_response_body_size: Option<u64>,
    rpc_timeout: Option<u64>,
    rpc_max_event_page_size: Option<u64>,
    rpc_max_proof_keys: Option<u64>,
    rpc_max_call_gas: Option<u64>,

    // Dev options
    seed: Option<u64>,
    accounts: Option<u16>,
    disable_fee: bool,
    disable_validate: bool,

    // Environment options
    chain_id: Option<Felt>,
    validate_max_steps: Option<u64>,
    invoke_max_steps: Option<u64>,
    genesis: Option<PathBuf>,

    // Gas Price Oracle options
    gpo_l2_eth_gas_price: Option<u64>,
    gpo_l2_strk_gas_price: Option<u64>,
    gpo_l1_eth_gas_price: Option<u64>,
    gpo_l1_strk_gas_price: Option<u64>,
    gpo_l1_eth_data_gas_price: Option<u64>,
    gpo_l1_strk_data_gas_price: Option<u64>,

    // Explorer options
    explorer: bool,

    // Cartridge options
    cartridge_controllers: bool,
    cartridge_paymaster: bool,
    cartridge_api: Option<String>,

    // Deprecated options (kept for backward compatibility)
    json_log: bool,
    rpc_timeout_ms: Option<u64>,
    http_cors_domain: Option<String>,
    fork_block_number: Option<u64>,
    eth_gas_price: Option<u64>,
    strk_gas_price: Option<u64>,
    enable_cartridge_paymaster: bool,
    cartridge_api_url: Option<String>,

    // Others
    timeout: Option<u64>,
    program: Option<PathBuf>,
}

impl Katana {
    /// Creates an empty Katana builder.
    /// The default port is 5050.
    ///
    /// # Example
    ///
    /// ```ignore
    /// use katana_node_bindings::Katana;
    ///
    /// fn main() {
    /// 	let katana = Katana::default().spawn();
    /// 	println!("Katana running at `{}`", katana.rpc_addr());
    /// }
    /// ```
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a Katana builder which will execute `katana` at the given path.
    ///
    /// # Example
    ///
    /// ```ignore
    /// use katana_node_bindings::Katana;
    ///
    /// fn main() {
    ///     let katana = Katana::at("~/.katana/bin/katana").spawn();
    ///     println!("Katana running at `{}`", katana.rpc_addr());
    /// }
    /// ```
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self::new().path(path)
    }

    /// Sets the `path` to the `katana` cli
    ///
    /// By default, it's expected that `katana` is in `$PATH`, see also
    /// [`std::process::Command::new()`]
    pub fn path<T: Into<PathBuf>>(mut self, path: T) -> Self {
        self.program = Some(path.into());
        self
    }

    /// Sets the port which will be used when the `katana` instance is launched.
    ///
    /// Default: `5050`
    ///
    /// ## CLI Flag
    ///
    /// `--http.port <PORT>`
    pub fn port<T: Into<u16>>(mut self, port: T) -> Self {
        self.http_port = Some(port.into());
        self
    }

    /// Sets the block-time in milliseconds which will be used when the `katana` instance is
    /// launched.
    ///
    /// ## CLI Flag
    ///
    /// `-b, --block-time <MILLISECONDS>`
    pub const fn block_time(mut self, block_time: u64) -> Self {
        self.block_time = Some(block_time);
        self
    }

    /// Sets the database directory path which will be used when the `katana` instance is launched.
    ///
    /// ## CLI Flag
    ///
    /// `--db-dir <PATH>`
    pub fn db_dir<T: Into<PathBuf>>(mut self, db_dir: T) -> Self {
        self.db_dir = Some(db_dir.into());
        self
    }

    /// Enables the dev mode.
    ///
    /// ## CLI Flag
    ///
    /// `--dev`
    pub const fn dev(mut self, dev: bool) -> Self {
        self.dev = dev;
        self
    }

    /// Sets the messaging configuration path which will be used when the `katana` instance is
    /// launched.
    ///
    /// ## CLI Flag
    ///
    /// `--messaging <PATH>`
    pub fn messaging<T: Into<PathBuf>>(mut self, messaging: T) -> Self {
        self.messaging = Some(messaging.into());
        self
    }

    /// Enables Prometheus metrics and sets the metrics server address.
    ///
    /// Default: `127.0.0.1`
    ///
    /// ## CLI Flag
    ///
    /// `--metrics.addr <ADDRESS>`
    pub fn metrics_addr<T: Into<SocketAddr>>(mut self, addr: T) -> Self {
        self.metrics_addr = Some(addr.into());
        self
    }

    /// Enables Prometheus metrics and sets the metrics server port.
    ///
    /// Default: `9100`
    ///
    /// ## CLI Flag
    ///
    /// `--metrics.port <PORT>`
    pub fn metrics_port<T: Into<u16>>(mut self, port: T) -> Self {
        self.metrics_port = Some(port.into());
        self
    }

    /// Sets the host IP address the server will listen on.
    ///
    /// Default: `127.0.0.1`
    ///
    /// ## CLI Flag
    ///
    /// `--http.addr <ADDRESS>`
    pub fn http_addr<T: Into<SocketAddr>>(mut self, addr: T) -> Self {
        self.http_addr = Some(addr.into());
        self
    }

    /// Sets the maximum number of concurrent connections allowed.
    ///
    /// ## CLI Flag
    ///
    /// `--rpc.max-connections <MAX>`
    pub const fn rpc_max_connections(mut self, max_connections: u64) -> Self {
        self.rpc_max_connections = Some(max_connections);
        self
    }

    /// Sets the maximum gas for the `starknet_call` RPC method.
    ///
    /// Default: `1000000000`
    ///
    /// ## CLI Flag
    ///
    /// `--rpc.max-call-gas <GAS>`
    pub const fn rpc_max_call_gas(mut self, max_call_gas: u64) -> Self {
        self.rpc_max_call_gas = Some(max_call_gas);
        self
    }

    /// Sets the seed for randomness of accounts to be predeployed.
    ///
    /// Default: `0`
    ///
    /// ## CLI Flag
    ///
    /// `--dev.seed <SEED>`
    pub const fn seed(mut self, seed: u64) -> Self {
        self.seed = Some(seed);
        self
    }

    /// Sets the number of pre-funded accounts to generate.
    ///
    /// Default: `10`
    ///
    /// ## CLI Flag
    ///
    /// `--dev.accounts <NUM>`
    pub fn accounts(mut self, accounts: u16) -> Self {
        self.accounts = Some(accounts);
        self
    }

    /// Enable or disable charging fee when executing transactions.
    /// Enabled by default.
    ///
    /// ## CLI Flag
    ///
    /// `--dev.no-fee` (when disabled)
    pub const fn fee(mut self, enable: bool) -> Self {
        self.disable_fee = !enable;
        self
    }

    /// Enables or disable transaction validation.
    /// Enabled by default.
    ///
    /// ## CLI Flag
    ///
    /// `--dev.no-account-validation` (when disabled)
    pub const fn validate(mut self, enable: bool) -> Self {
        self.disable_validate = !enable;
        self
    }

    /// Sets the chain ID.
    ///
    /// ## CLI Flag
    ///
    /// `--chain-id <CHAIN_ID>`
    pub const fn chain_id(mut self, id: Felt) -> Self {
        self.chain_id = Some(id);
        self
    }

    /// Sets the maximum number of steps available for the account validation logic.
    ///
    /// Default: `1000000`
    ///
    /// ## CLI Flag
    ///
    /// `--validate-max-steps <VALIDATE_MAX_STEPS>`
    pub const fn validate_max_steps(mut self, validate_max_steps: u64) -> Self {
        self.validate_max_steps = Some(validate_max_steps);
        self
    }

    /// Sets the maximum number of steps available for the account execution logic.
    ///
    /// Default: `10000000`
    ///
    /// ## CLI Flag
    ///
    /// `--invoke-max-steps <INVOKE_MAX_STEPS>`
    pub const fn invoke_max_steps(mut self, invoke_max_steps: u64) -> Self {
        self.invoke_max_steps = Some(invoke_max_steps);
        self
    }

    /// Sets the genesis configuration path.
    ///
    /// ## CLI Flag
    ///
    /// `--genesis <GENESIS>`
    pub fn genesis<T: Into<PathBuf>>(mut self, genesis: T) -> Self {
        self.genesis = Some(genesis.into());
        self
    }

    /// Sets the timeout which will be used when the `katana` instance is launched.
    ///
    /// Note: This is an internal timeout for the bindings, not a CLI flag.
    pub const fn timeout(mut self, timeout: u64) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Disable auto and interval mining, and mine on demand instead via an endpoint.
    ///
    /// ## CLI Flag
    ///
    /// `--no-mining`
    pub const fn no_mining(mut self, no_mining: bool) -> Self {
        self.no_mining = no_mining;
        self
    }

    /// Don't print anything on startup.
    ///
    /// ## CLI Flag
    ///
    /// `--silent`
    pub const fn silent(mut self, silent: bool) -> Self {
        self.silent = silent;
        self
    }

    /// Sets the log format to use.
    ///
    /// Default: `LogFormat::Full`
    ///
    /// ## CLI Flag
    ///
    /// `--log.format <FORMAT>`
    pub const fn log_format(mut self, format: LogFormat) -> Self {
        self.log_format = Some(format);
        self
    }

    /// Sets the maximum number of Cairo steps available for block sequencing.
    ///
    /// ## CLI Flag
    ///
    /// `--sequencing.block-max-cairo-steps <TOTAL>`
    pub const fn sequencing_block_max_cairo_steps(mut self, steps: u64) -> Self {
        self.sequencing_block_max_cairo_steps = Some(steps);
        self
    }

    /// Sets the configuration file path.
    ///
    /// ## CLI Flag
    ///
    /// `--config <CONFIG>`
    pub fn config<T: Into<PathBuf>>(mut self, config: T) -> Self {
        self.config = Some(config.into());
        self
    }

    /// Sets the RPC URL to fork the network from.
    ///
    /// ## CLI Flag
    ///
    /// `--fork.provider <URL>`
    pub fn fork_provider<T: Into<String>>(mut self, provider: T) -> Self {
        self.fork_provider = Some(provider.into());
        self
    }

    /// Sets the fork block number.
    ///
    /// ## CLI Flag
    ///
    /// `--fork.block <BLOCK>`
    pub const fn fork_block(mut self, block: u64) -> Self {
        self.fork_block = Some(block);
        self
    }

    /// Enable Google Cloud Trace exporter.
    ///
    /// ## CLI Flag
    ///
    /// `--tracer.gcloud`
    pub const fn tracer_gcloud(mut self, enable: bool) -> Self {
        self.tracer_gcloud = enable;
        self
    }

    /// Enable OpenTelemetry Protocol (OTLP) exporter.
    ///
    /// ## CLI Flag
    ///
    /// `--tracer.otlp`
    pub const fn tracer_otlp(mut self, enable: bool) -> Self {
        self.tracer_otlp = enable;
        self
    }

    /// Sets the Google Cloud project ID.
    ///
    /// ## CLI Flag
    ///
    /// `--tracer.gcloud-project <PROJECT_ID>`
    pub fn tracer_gcloud_project<T: Into<String>>(mut self, project_id: T) -> Self {
        self.tracer_gcloud_project = Some(project_id.into());
        self
    }

    /// Sets the OTLP endpoint URL.
    ///
    /// ## CLI Flag
    ///
    /// `--tracer.otlp-endpoint <URL>`
    pub fn tracer_otlp_endpoint<T: Into<String>>(mut self, endpoint: T) -> Self {
        self.tracer_otlp_endpoint = Some(endpoint.into());
        self
    }

    /// Enable metrics collection.
    ///
    /// ## CLI Flag
    ///
    /// `--metrics`
    pub const fn metrics(mut self, enable: bool) -> Self {
        self.metrics = enable;
        self
    }

    /// Sets the comma separated list of domains from which to accept cross origin requests.
    ///
    /// ## CLI Flag
    ///
    /// `--http.cors_origins <HTTP_CORS_ORIGINS>`
    pub fn http_cors_origins<T: Into<String>>(mut self, origins: T) -> Self {
        self.http_cors_origins = Some(origins.into());
        self
    }

    /// Sets the API modules offered over the HTTP-RPC interface.
    ///
    /// ## CLI Flag
    ///
    /// `--http.api <MODULES>`
    pub fn http_api<T: Into<String>>(mut self, modules: T) -> Self {
        self.http_api = Some(modules.into());
        self
    }

    /// Sets the maximum request body size (in bytes).
    ///
    /// ## CLI Flag
    ///
    /// `--rpc.max-request-body-size <SIZE>`
    pub const fn rpc_max_request_body_size(mut self, size: u64) -> Self {
        self.rpc_max_request_body_size = Some(size);
        self
    }

    /// Sets the maximum response body size (in bytes).
    ///
    /// ## CLI Flag
    ///
    /// `--rpc.max-response-body-size <SIZE>`
    pub const fn rpc_max_response_body_size(mut self, size: u64) -> Self {
        self.rpc_max_response_body_size = Some(size);
        self
    }

    /// Sets the timeout for the RPC server request (in seconds).
    ///
    /// ## CLI Flag
    ///
    /// `--rpc.timeout <TIMEOUT>`
    pub const fn rpc_timeout(mut self, timeout: u64) -> Self {
        self.rpc_timeout = Some(timeout);
        self
    }

    /// Sets the maximum page size for event queries.
    ///
    /// Default: `1024`
    ///
    /// ## CLI Flag
    ///
    /// `--rpc.max-event-page-size <SIZE>`
    pub const fn rpc_max_event_page_size(mut self, size: u64) -> Self {
        self.rpc_max_event_page_size = Some(size);
        self
    }

    /// Sets the maximum keys for requesting storage proofs.
    ///
    /// Default: `100`
    ///
    /// ## CLI Flag
    ///
    /// `--rpc.max-proof-keys <SIZE>`
    pub const fn rpc_max_proof_keys(mut self, keys: u64) -> Self {
        self.rpc_max_proof_keys = Some(keys);
        self
    }

    /// Sets the L2 ETH gas price (denominated in wei).
    ///
    /// ## CLI Flag
    ///
    /// `--gpo.l2-eth-gas-price <WEI>`
    pub const fn gpo_l2_eth_gas_price(mut self, price: u64) -> Self {
        self.gpo_l2_eth_gas_price = Some(price);
        self
    }

    /// Sets the L2 STRK gas price (denominated in fri).
    ///
    /// ## CLI Flag
    ///
    /// `--gpo.l2-strk-gas-price <FRI>`
    pub const fn gpo_l2_strk_gas_price(mut self, price: u64) -> Self {
        self.gpo_l2_strk_gas_price = Some(price);
        self
    }

    /// Sets the L1 ETH gas price (denominated in wei).
    ///
    /// ## CLI Flag
    ///
    /// `--gpo.l1-eth-gas-price <WEI>`
    pub const fn gpo_l1_eth_gas_price(mut self, price: u64) -> Self {
        self.gpo_l1_eth_gas_price = Some(price);
        self
    }

    /// Sets the L1 STRK gas price (denominated in fri).
    ///
    /// ## CLI Flag
    ///
    /// `--gpo.l1-strk-gas-price <FRI>`
    pub const fn gpo_l1_strk_gas_price(mut self, price: u64) -> Self {
        self.gpo_l1_strk_gas_price = Some(price);
        self
    }

    /// Sets the L1 ETH data gas price (denominated in wei).
    ///
    /// ## CLI Flag
    ///
    /// `--gpo.l1-eth-data-gas-price <WEI>`
    pub const fn gpo_l1_eth_data_gas_price(mut self, price: u64) -> Self {
        self.gpo_l1_eth_data_gas_price = Some(price);
        self
    }

    /// Sets the L1 STRK data gas price (denominated in fri).
    ///
    /// ## CLI Flag
    ///
    /// `--gpo.l1-strk-data-gas-price <FRI>`
    pub const fn gpo_l1_strk_data_gas_price(mut self, price: u64) -> Self {
        self.gpo_l1_strk_data_gas_price = Some(price);
        self
    }

    /// Enable and launch the explorer frontend.
    ///
    /// ## CLI Flag
    ///
    /// `--explorer`
    pub const fn explorer(mut self, enable: bool) -> Self {
        self.explorer = enable;
        self
    }

    /// Declare all versions of the Controller class at genesis.
    ///
    /// ## CLI Flag
    ///
    /// `--cartridge.controllers`
    pub const fn cartridge_controllers(mut self, enable: bool) -> Self {
        self.cartridge_controllers = enable;
        self
    }

    /// Whether to use the Cartridge paymaster.
    ///
    /// ## CLI Flag
    ///
    /// `--cartridge.paymaster`
    pub const fn cartridge_paymaster(mut self, enable: bool) -> Self {
        self.cartridge_paymaster = enable;
        self
    }

    /// Sets the root URL for the Cartridge API.
    ///
    /// Default: `https://api.cartridge.gg`
    ///
    /// ## CLI Flag
    ///
    /// `--cartridge.api <API>`
    pub fn cartridge_api<T: Into<String>>(mut self, api: T) -> Self {
        self.cartridge_api = Some(api.into());
        self
    }

    // Deprecated methods for backward compatibility
    /// Enables JSON logging.
    ///
    /// **Deprecated**: Use `log_format(LogFormat::Json)` instead.
    #[deprecated(note = "Use `log_format(LogFormat::Json)` instead")]
    pub const fn json_log(mut self, json_log: bool) -> Self {
        self.json_log = json_log;
        self
    }

    /// Sets the fork block number which will be used when the `katana` instance is launched.
    ///
    /// **Deprecated**: Use `fork_block()` instead.
    #[deprecated(note = "Use `fork_block()` instead")]
    pub const fn fork_block_number(mut self, fork_block_number: u64) -> Self {
        self.fork_block_number = Some(fork_block_number);
        self
    }

    /// Sets the RPC URL to fork the network from.
    ///
    /// **Deprecated**: Use `fork_provider()` instead.
    #[deprecated(note = "Use `fork_provider()` instead")]
    pub fn l1_provider<T: Into<String>>(mut self, rpc_url: T) -> Self {
        self.l1_provider = Some(rpc_url.into());
        self
    }

    /// Sets the maximum timeout for the RPC.
    ///
    /// **Deprecated**: Use `rpc_timeout()` instead (note: units changed from ms to seconds).
    #[deprecated(note = "Use `rpc_timeout()` instead")]
    pub const fn rpc_timeout_ms(mut self, timeout_ms: u64) -> Self {
        self.rpc_timeout_ms = Some(timeout_ms);
        self
    }

    /// Enables the CORS layer and sets the allowed origins, separated by commas.
    ///
    /// **Deprecated**: Use `http_cors_origins()` instead.
    #[deprecated(note = "Use `http_cors_origins()` instead")]
    pub fn http_cors_domain<T: Into<String>>(mut self, allowed_origins: T) -> Self {
        self.http_cors_domain = Some(allowed_origins.into());
        self
    }

    /// Sets the L1 ETH gas price (denominated in wei).
    ///
    /// **Deprecated**: Use `gpo_l2_eth_gas_price()` instead.
    #[deprecated(note = "Use `gpo_l2_eth_gas_price()` instead")]
    pub const fn eth_gas_price(mut self, eth_gas_price: u64) -> Self {
        self.eth_gas_price = Some(eth_gas_price);
        self
    }

    /// Sets the L1 STRK gas price (denominated in fri).
    ///
    /// **Deprecated**: Use `gpo_l2_strk_gas_price()` instead.
    #[deprecated(note = "Use `gpo_l2_strk_gas_price()` instead")]
    pub const fn strk_gas_price(mut self, strk_gas_price: u64) -> Self {
        self.strk_gas_price = Some(strk_gas_price);
        self
    }

    /// Whether to use the Cartridge paymaster.
    ///
    /// **Deprecated**: Use `cartridge_paymaster()` instead.
    #[deprecated(note = "Use `cartridge_paymaster()` instead")]
    pub const fn enable_cartridge_paymaster(mut self, enable: bool) -> Self {
        self.enable_cartridge_paymaster = enable;
        self
    }

    /// Sets the root URL for the Cartridge API.
    ///
    /// **Deprecated**: Use `cartridge_api()` instead.
    #[deprecated(note = "Use `cartridge_api()` instead")]
    pub fn cartridge_api_url<T: Into<String>>(mut self, url: T) -> Self {
        self.cartridge_api_url = Some(url.into());
        self
    }

    /// Consumes the builder and spawns `katana`.
    ///
    /// # Panics
    ///
    /// If spawning the instance fails at any point.
    #[track_caller]
    pub fn spawn(self) -> KatanaInstance {
        self.try_spawn().expect("could not spawn katana")
    }

    /// Consumes the builder and spawns `katana`. If spawning fails, returns an error.
    pub fn try_spawn(self) -> Result<KatanaInstance, Error> {
        let mut cmd = self.program.as_ref().map_or_else(|| Command::new("katana"), Command::new);
        cmd.stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::inherit());

        // Node options
        if self.silent {
            cmd.arg("--silent");
        }

        if self.no_mining {
            cmd.arg("--no-mining");
        }

        if let Some(block_time) = self.block_time {
            cmd.arg("-b").arg(block_time.to_string());
        }

        if let Some(steps) = self.sequencing_block_max_cairo_steps {
            cmd.arg("--sequencing.block-max-cairo-steps").arg(steps.to_string());
        }

        if let Some(db_dir) = self.db_dir {
            cmd.arg("--db-dir").arg(db_dir);
        }

        if let Some(config) = self.config {
            cmd.arg("--config").arg(config);
        }

        if let Some(messaging) = self.messaging {
            cmd.arg("--messaging").arg(messaging);
        }

        // Handle backward compatibility for l1_provider
        if let Some(url) = self.l1_provider {
            cmd.arg("--l1.provider").arg(url);
        }

        // Logging options
        if let Some(ref format) = self.log_format {
            cmd.arg("--log.format").arg(format.to_string());
        } else if self.json_log {
            // Backward compatibility
            cmd.arg("--log.format").arg("json");
        }

        // Tracer options
        if self.tracer_gcloud {
            cmd.arg("--tracer.gcloud");
        }

        if self.tracer_otlp {
            cmd.arg("--tracer.otlp");
        }

        if let Some(project_id) = self.tracer_gcloud_project {
            cmd.arg("--tracer.gcloud-project").arg(project_id);
        }

        if let Some(endpoint) = self.tracer_otlp_endpoint {
            cmd.arg("--tracer.otlp-endpoint").arg(endpoint);
        }

        // Server options
        if let Some(host) = self.http_addr {
            cmd.arg("--http.addr").arg(host.to_string());
        }

        // In the case where port 0 is set, we will need to extract the actual port number
        // from the logs.
        let mut port = self.http_port.unwrap_or(0);
        cmd.arg("--http.port").arg(port.to_string());

        if let Some(origins) = self.http_cors_origins {
            cmd.arg("--http.cors_origins").arg(origins);
        } else if let Some(domain) = self.http_cors_domain {
            // Backward compatibility
            cmd.arg("--http.cors_origins").arg(domain);
        }

        if let Some(modules) = self.http_api {
            cmd.arg("--http.api").arg(modules);
        }

        // Need to make sure that the `--dev` is not being set twice.
        let mut is_dev = false;

        if self.dev {
            cmd.arg("--dev");
            is_dev = true;
        }

        if let Some(seed) = self.seed {
            if !is_dev {
                cmd.arg("--dev");
                is_dev = true;
            }

            cmd.arg("--dev.seed").arg(seed.to_string());
        }

        if let Some(accounts) = self.accounts {
            if !is_dev {
                cmd.arg("--dev");
                is_dev = true;
            }

            cmd.arg("--dev.accounts").arg(accounts.to_string());
        }

        if self.disable_fee {
            if !is_dev {
                cmd.arg("--dev");
                is_dev = true;
            }

            cmd.arg("--dev.no-fee");
        }

        if self.disable_validate {
            if !is_dev {
                cmd.arg("--dev");
            }

            cmd.arg("--dev.no-account-validation");
        }

        // RPC options
        if let Some(max_connections) = self.rpc_max_connections {
            cmd.arg("--rpc.max-connections").arg(max_connections.to_string());
        }

        if let Some(size) = self.rpc_max_request_body_size {
            cmd.arg("--rpc.max-request-body-size").arg(size.to_string());
        }

        if let Some(size) = self.rpc_max_response_body_size {
            cmd.arg("--rpc.max-response-body-size").arg(size.to_string());
        }

        if let Some(timeout) = self.rpc_timeout {
            cmd.arg("--rpc.timeout").arg(timeout.to_string());
        } else if let Some(timeout_ms) = self.rpc_timeout_ms {
            // Backward compatibility - convert ms to seconds
            cmd.arg("--rpc.timeout").arg((timeout_ms / 1000).to_string());
        }

        if let Some(size) = self.rpc_max_event_page_size {
            cmd.arg("--rpc.max-event-page-size").arg(size.to_string());
        }

        if let Some(keys) = self.rpc_max_proof_keys {
            cmd.arg("--rpc.max-proof-keys").arg(keys.to_string());
        }

        if let Some(max_call_gas) = self.rpc_max_call_gas {
            cmd.arg("--rpc.max-call-gas").arg(max_call_gas.to_string());
        }

        // Metrics options
        let mut metrics_enabled = self.metrics;

        if let Some(addr) = self.metrics_addr {
            if !metrics_enabled {
                cmd.arg("--metrics");
                metrics_enabled = true;
            }
            cmd.arg("--metrics.addr").arg(addr.to_string());
        }

        if let Some(port) = self.metrics_port {
            if !metrics_enabled {
                cmd.arg("--metrics");
                metrics_enabled = true;
            }
            cmd.arg("--metrics.port").arg(port.to_string());
        }

        if metrics_enabled && !self.metrics_addr.is_some() && !self.metrics_port.is_some() {
            cmd.arg("--metrics");
        }

        // Forking options
        if let Some(provider) = self.fork_provider {
            cmd.arg("--fork.provider").arg(provider);
        }

        if let Some(block) = self.fork_block {
            cmd.arg("--fork.block").arg(block.to_string());
        } else if let Some(fork_block_number) = self.fork_block_number {
            // Backward compatibility
            cmd.arg("--fork.block").arg(fork_block_number.to_string());
        }

        if let Some(chain_id) = self.chain_id {
            // Referring to katana docs:
            // "If the `str` starts with `0x` it is parsed as a hex string, otherwise it is parsed
            // as a Cairo short string."
            cmd.arg("--chain-id").arg(format!("{chain_id:#x}"));
        }

        if let Some(validate_max_steps) = self.validate_max_steps {
            cmd.arg("--validate-max-steps").arg(validate_max_steps.to_string());
        }

        if let Some(invoke_max_steps) = self.invoke_max_steps {
            cmd.arg("--invoke-max-steps").arg(invoke_max_steps.to_string());
        }

        if let Some(genesis) = self.genesis {
            cmd.arg("--genesis").arg(genesis);
        }

        // Gas Price Oracle options
        if let Some(price) = self.gpo_l2_eth_gas_price {
            cmd.arg("--gpo.l2-eth-gas-price").arg(price.to_string());
        } else if let Some(price) = self.eth_gas_price {
            // Backward compatibility - assume it's L2 ETH gas price
            cmd.arg("--gpo.l2-eth-gas-price").arg(price.to_string());
        }

        if let Some(price) = self.gpo_l2_strk_gas_price {
            cmd.arg("--gpo.l2-strk-gas-price").arg(price.to_string());
        } else if let Some(price) = self.strk_gas_price {
            // Backward compatibility - assume it's L2 STRK gas price
            cmd.arg("--gpo.l2-strk-gas-price").arg(price.to_string());
        }

        if let Some(price) = self.gpo_l1_eth_gas_price {
            cmd.arg("--gpo.l1-eth-gas-price").arg(price.to_string());
        }

        if let Some(price) = self.gpo_l1_strk_gas_price {
            cmd.arg("--gpo.l1-strk-gas-price").arg(price.to_string());
        }

        if let Some(price) = self.gpo_l1_eth_data_gas_price {
            cmd.arg("--gpo.l1-eth-data-gas-price").arg(price.to_string());
        }

        if let Some(price) = self.gpo_l1_strk_data_gas_price {
            cmd.arg("--gpo.l1-strk-data-gas-price").arg(price.to_string());
        }

        // Explorer options
        if self.explorer {
            cmd.arg("--explorer");
        }

        // Cartridge options
        if self.cartridge_controllers {
            cmd.arg("--cartridge.controllers");
        }

        if self.cartridge_paymaster {
            cmd.arg("--cartridge.paymaster");
        } else if self.enable_cartridge_paymaster {
            // Backward compatibility
            cmd.arg("--cartridge.paymaster");
        }

        if let Some(api) = self.cartridge_api {
            cmd.arg("--cartridge.api").arg(api);
        } else if let Some(url) = self.cartridge_api_url {
            // Backward compatibility
            cmd.arg("--cartridge.api").arg(url);
        }

        let mut child = cmd.spawn().map_err(Error::SpawnError)?;
        let stdout = child.stdout.as_mut().ok_or(Error::NoStderr)?;

        let start = Instant::now();
        let mut reader = BufReader::new(stdout);

        let mut accounts = Vec::new();
        // var to store the current account being processed
        let mut current_account: Option<Account> = None;
        // var to store the chain id parsed from the logs. default to KATANA (default katana chain
        // id) if not specified
        let mut chain_id: Felt = self.chain_id.unwrap_or(short_string!("KATANA"));

        loop {
            if start + Duration::from_millis(self.timeout.unwrap_or(KATANA_STARTUP_TIMEOUT_MILLIS))
                <= Instant::now()
            {
                return Err(Error::Timeout);
            }

            let mut line = String::new();
            reader.read_line(&mut line).map_err(Error::ReadLineError)?;
            trace!(line);

            if self.json_log || self.log_format == Some(LogFormat::Json) {
                // Because we using a concrete type for rpc addr log, we need to parse this first.
                // Otherwise if we were to inverse the if statements, the else block
                // would never be executed as all logs can be parsed as `JsonLog`.
                if let Ok(log) = serde_json::from_str::<JsonLog<json::RpcAddr>>(&line) {
                    debug_assert!(log.fields.message.contains(RPC_ADDR_LOG_SUBSTR));
                    port = log.fields.other.addr.port();
                    // We can safely break here as we don't need any information after the rpc
                    // address
                    break;
                }
                // Try parsing as chain id log
                else if let Ok(log) = serde_json::from_str::<JsonLog<json::ChainId>>(&line) {
                    debug_assert!(log.fields.message.contains(CHAIN_ID_LOG_SUBSTR));
                    let chain_raw = log.fields.other.chain;
                    chain_id = if chain_raw.starts_with("0x") {
                        Felt::from_str(&chain_raw)?
                    } else {
                        cairo_short_string_to_felt(&chain_raw)
                            .map_err(|_| Error::UnexpectedFormat("invalid chain id".to_string()))?
                    };
                }
                // Parse all logs as generic logs
                else if let Ok(info) = serde_json::from_str::<JsonLog>(&line) {
                    // Check if this log is a katana startup info log
                    if let Ok(info) = KatanaInfo::try_from(info.fields.message) {
                        for (address, info) in info.accounts {
                            let address = Felt::from_str(&address)?;
                            let private_key = Felt::from_str(&info.private_key)?;
                            let key = SigningKey::from_secret_scalar(private_key);
                            accounts.push(Account { address, private_key: Some(key) });
                        }

                        continue;
                    }
                }
            } else {
                if line.contains(RPC_ADDR_LOG_SUBSTR) {
                    let addr = parse_rpc_addr_log(&line)?;
                    port = addr.port();
                    // The address is the last thing to be displayed so we can safely break here.
                    break;
                }

                if line.contains(CHAIN_ID_LOG_SUBSTR) {
                    chain_id = parse_chain_id_log(&line)?;
                }

                const ACC_ADDRESS_PREFIX: &str = "| Account address |";
                if line.starts_with(ACC_ADDRESS_PREFIX) {
                    // If there is currently an account being handled, but we've reached the next
                    // account line, that means the previous address didn't have
                    // a private key, so we can add it to the accounts list and
                    // start processing the next account.
                    if let Some(acc) = current_account.take() {
                        accounts.push(acc);
                    }

                    let hex = line
                        .strip_prefix(ACC_ADDRESS_PREFIX)
                        .ok_or(Error::MissingAccountAddress)?
                        .trim();

                    let address = Felt::from_str(hex)?;
                    let account = Account { address, private_key: None };
                    current_account = Some(account);
                }

                const ACC_PK_PREFIX: &str = "| Private key     |";
                if line.starts_with(ACC_PK_PREFIX) {
                    // the private key may or may not be present for a particular account, so we
                    // have to handle both cases properly
                    let Some(acc) = current_account.take() else {
                        let msg = "Account address not found before private key".to_string();
                        return Err(Error::UnexpectedFormat(msg));
                    };

                    let hex = line
                        .strip_prefix(ACC_PK_PREFIX)
                        .ok_or(Error::MissingAccountPrivateKey)?
                        .trim();

                    let private_key = Felt::from_str(hex)?;
                    let signing_key = SigningKey::from_secret_scalar(private_key);
                    accounts.push(Account { private_key: Some(signing_key), ..acc });
                }
            }
        }

        Ok(KatanaInstance { port, child, accounts, chain_id })
    }
}

/// Removes ANSI escape codes from a string.
///
/// This is useful for removing the color codes from the katana output.
fn clean_ansi_escape_codes(input: &str) -> Result<Cow<'_, str>, Error> {
    let re = regex::Regex::new(r"\x1b\[[0-9;]*[a-zA-Z]")?;
    Ok(re.replace_all(input, ""))
}

// Example RPC address log format (ansi color codes removed):
// 2024-10-10T14:20:53.563106Z  INFO rpc: RPC server started. addr=127.0.0.1:60373
fn parse_rpc_addr_log(log: &str) -> Result<SocketAddr, Error> {
    // remove any ANSI escape codes from the log.
    let cleaned = clean_ansi_escape_codes(log)?;

    // This will separate the log into two parts as separated by `addr=` str and we take
    // only the second part which is the address.
    let addr_part = cleaned.split("addr=").nth(1).ok_or(Error::MissingSocketAddr)?;
    let addr = addr_part.trim();
    Ok(SocketAddr::from_str(addr)?)
}

// Example chain ID log format (ansi color codes removed):
// 2024-10-18T01:30:14.023880Z  INFO katana_node: Starting node. chain=0x4b4154414e41
fn parse_chain_id_log(log: &str) -> Result<Felt, Error> {
    // remove any ANSI escape codes from the log.
    let cleaned = clean_ansi_escape_codes(log)?;

    // This will separate the log into two parts as separated by `chain=` str and we take
    // only the second part which is the chain ID.
    let chain_part = cleaned.split("chain=").nth(1).ok_or(Error::MissingChainId)?.trim();
    if chain_part.starts_with("0x") {
        Ok(Felt::from_str(chain_part)?)
    } else {
        Ok(cairo_short_string_to_felt(chain_part)
            .map_err(|_| Error::UnexpectedFormat("invalid chain id".to_string()))?)
    }
}

#[cfg(test)]
mod tests {
    use starknet::core::types::{BlockId, BlockTag};
    use starknet::providers::Provider;

    use super::*;

    #[tokio::test]
    async fn can_launch_katana() {
        // this will launch katana with random ports
        let katana = Katana::new().spawn();

        // assert some default values
        assert_eq!(katana.accounts().len(), 10);
        assert_eq!(katana.chain_id(), short_string!("KATANA"));
        // assert that all accounts have private key
        assert!(katana.accounts().iter().all(|a| a.private_key.is_some()));

        // try to connect as a provider
        let provider = katana.starknet_provider();
        assert!(provider.chain_id().await.is_ok())
    }

    #[test]
    fn can_launch_katana_with_json_log() {
        let katana =
            Katana::new().log_format(LogFormat::Json).chain_id(short_string!("SN_SEPOLIA")).spawn();
        // Assert default values when using JSON logging
        assert_eq!(katana.accounts().len(), 10);
        assert_eq!(katana.chain_id(), short_string!("SN_SEPOLIA"));
        // assert that all accounts have private key
        assert!(katana.accounts().iter().all(|a| a.private_key.is_some()));
    }

    #[tokio::test]
    async fn can_launch_katana_with_more_accounts() {
        let katana = Katana::new().accounts(20).spawn();
        let provider = katana.starknet_provider();

        // Check that 20 accounts were created
        assert_eq!(katana.accounts().len(), 20);

        // Verify  that each account actually exists
        for (index, account) in katana.accounts().iter().enumerate() {
            // Check each account address has a deployed contract by verifying it has a class hash
            let address = account.address;
            let result = provider.get_class_hash_at(BlockId::Tag(BlockTag::Pending), address).await;
            assert!(result.is_ok(), "Account #{index} contract does not exist at {address}");
        }
    }

    #[test]
    fn assert_block_time_is_natural_number() {
        let katana = Katana::new().block_time(12);
        assert_eq!(katana.block_time.unwrap().to_string(), "12");
        let _ = katana.spawn();
    }

    #[test]
    fn can_launch_katana_with_sub_seconds_block_time() {
        let _ = Katana::new().block_time(500).spawn();
    }

    #[tokio::test]
    async fn can_launch_katana_with_specific_port() {
        let specific_port = 49999;
        let katana = Katana::new().port(specific_port).spawn();
        assert_eq!(katana.port(), specific_port);

        let provider = katana.starknet_provider();
        let result = provider.chain_id().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn assert_custom_chain_id() {
        let chain_id = short_string!("SN_GOERLI");
        let katana = Katana::new().chain_id(chain_id).spawn();

        let provider = katana.starknet_provider();
        let actual_chain_id = provider.chain_id().await.unwrap();

        assert_eq!(chain_id, actual_chain_id);
    }

    #[test]
    fn can_launch_katana_with_db_dir() {
        let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
        let db_path = temp_dir.path().join("test-db");
        assert!(!db_path.exists());

        let _katana = Katana::new().db_dir(db_path.clone()).spawn();

        // Check that the db directory is created
        assert!(db_path.exists());
        assert!(db_path.is_dir());
    }

    #[test]
    fn test_parse_rpc_addr_log() {
        // actual rpc log from katana
        let log = "\u{1b}[2m2024-10-10T14:48:55.397891Z\u{1b}[0m \u{1b}[32m INFO\u{1b}[0m \
                   \u{1b}[2mrpc\u{1b}[0m\u{1b}[2m:\u{1b}[0m RPC server started. \
                   \u{1b}[3maddr\u{1b}[0m\u{1b}[2m=\u{1b}[0m127.0.0.1:60817\n";
        let addr = parse_rpc_addr_log(log).unwrap();
        assert_eq!(addr.ip().to_string(), "127.0.0.1");
        assert_eq!(addr.port(), 60817);
    }

    #[tokio::test]
    async fn can_launch_katana_with_custom_chain_id() {
        let custom_chain_id = Felt::from_str("0x1234").unwrap();
        let katana = Katana::new().chain_id(custom_chain_id).spawn();

        assert_eq!(katana.chain_id(), custom_chain_id);

        let provider = katana.starknet_provider();
        let actual_chain_id = provider.chain_id().await.unwrap();

        assert_eq!(custom_chain_id, actual_chain_id);
    }

    #[test]
    fn test_new_log_format_option() {
        let katana = Katana::new().log_format(LogFormat::Json);
        assert_eq!(katana.log_format.as_ref().unwrap(), &LogFormat::Json);

        let katana_full = Katana::new().log_format(LogFormat::Full);
        assert_eq!(katana_full.log_format.as_ref().unwrap(), &LogFormat::Full);
    }

    #[test]
    fn test_log_format_display() {
        assert_eq!(LogFormat::Json.to_string(), "json");
        assert_eq!(LogFormat::Full.to_string(), "full");
    }

    #[test]
    fn test_log_format_from_str() {
        assert_eq!("json".parse::<LogFormat>().unwrap(), LogFormat::Json);
        assert_eq!("JSON".parse::<LogFormat>().unwrap(), LogFormat::Json);
        assert_eq!("full".parse::<LogFormat>().unwrap(), LogFormat::Full);
        assert_eq!("FULL".parse::<LogFormat>().unwrap(), LogFormat::Full);

        assert!("invalid".parse::<LogFormat>().is_err());
    }

    #[test]
    fn test_log_format_default() {
        assert_eq!(LogFormat::default(), LogFormat::Full);
    }

    #[test]
    fn test_forking_options() {
        let katana = Katana::new().fork_provider("http://localhost:8545").fork_block(12345);
        assert_eq!(katana.fork_provider.as_ref().unwrap(), "http://localhost:8545");
        assert_eq!(katana.fork_block.unwrap(), 12345);
    }

    #[test]
    fn test_gas_price_oracle_options() {
        let katana = Katana::new()
            .gpo_l2_eth_gas_price(1000)
            .gpo_l2_strk_gas_price(2000)
            .gpo_l1_eth_gas_price(3000)
            .gpo_l1_strk_gas_price(4000);

        assert_eq!(katana.gpo_l2_eth_gas_price.unwrap(), 1000);
        assert_eq!(katana.gpo_l2_strk_gas_price.unwrap(), 2000);
        assert_eq!(katana.gpo_l1_eth_gas_price.unwrap(), 3000);
        assert_eq!(katana.gpo_l1_strk_gas_price.unwrap(), 4000);
    }

    #[test]
    fn test_rpc_options() {
        let katana = Katana::new()
            .rpc_max_request_body_size(1024)
            .rpc_max_response_body_size(2048)
            .rpc_timeout(30)
            .rpc_max_event_page_size(100)
            .rpc_max_proof_keys(50);

        assert_eq!(katana.rpc_max_request_body_size.unwrap(), 1024);
        assert_eq!(katana.rpc_max_response_body_size.unwrap(), 2048);
        assert_eq!(katana.rpc_timeout.unwrap(), 30);
        assert_eq!(katana.rpc_max_event_page_size.unwrap(), 100);
        assert_eq!(katana.rpc_max_proof_keys.unwrap(), 50);
    }

    #[test]
    fn test_cartridge_options() {
        let katana = Katana::new()
            .cartridge_controllers(true)
            .cartridge_paymaster(true)
            .cartridge_api("https://api.cartridge.gg");

        assert!(katana.cartridge_controllers);
        assert!(katana.cartridge_paymaster);
        assert_eq!(katana.cartridge_api.as_ref().unwrap(), "https://api.cartridge.gg");
    }

    #[test]
    fn test_explorer_option() {
        let katana = Katana::new().explorer(true);
        assert!(katana.explorer);
    }
}
