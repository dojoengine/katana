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
/// use katana_node_bindings::Katana;
///
/// let port = 5050u16;
/// let url = format!("http://localhost:{}", port).to_string();
///
/// let katana = Katana::new().port(port).spawn();
///
/// drop(katana); // this will kill the instance
/// ```
#[derive(Clone, Debug, Default)]
#[must_use = "This Builder struct does nothing unless it is `spawn`ed"]
pub struct Katana {
    // General options
    dev: bool,
    no_mining: bool,
    json_log: bool,
    block_time: Option<u64>,
    db_dir: Option<PathBuf>,
    l1_provider: Option<String>,
    fork_block_number: Option<u64>,
    messaging: Option<PathBuf>,

    // Metrics options
    metrics_addr: Option<SocketAddr>,
    metrics_port: Option<u16>,

    // Server options
    http_addr: Option<SocketAddr>,
    http_port: Option<u16>,
    rpc_max_connections: Option<u64>,
    rpc_timeout_ms: Option<u64>,
    rpc_max_call_gas: Option<u64>,
    http_cors_domain: Option<String>,

    // Dev options
    seed: Option<u64>,
    accounts: Option<u16>,
    disable_fee: bool,
    disable_validate: bool,

    // Environment options
    chain_id: Option<Felt>,
    validate_max_steps: Option<u64>,
    invoke_max_steps: Option<u64>,
    eth_gas_price: Option<u64>,
    strk_gas_price: Option<u64>,
    genesis: Option<PathBuf>,

    // Cartridge options
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
    /// ```
    /// # use katana_node_bindings::Katana;
    /// fn a() {
    ///  let katana = Katana::default().spawn();
    ///
    ///  println!("Katana running at `{}`", katana.endpoint());
    /// # }
    /// ```
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a Katana builder which will execute `katana` at the given path.
    ///
    /// # Example
    ///
    /// ```
    /// # use katana_node_bindings::Katana;
    /// fn a() {
    ///  let katana = Katana::at("~/.katana/bin/katana").spawn();
    ///
    ///  println!("Katana running at `{}`", katana.endpoint());
    /// # }
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
    pub fn port<T: Into<u16>>(mut self, port: T) -> Self {
        self.http_port = Some(port.into());
        self
    }

    /// Sets the block-time in milliseconds which will be used when the `katana` instance is
    /// launched.
    pub const fn block_time(mut self, block_time: u64) -> Self {
        self.block_time = Some(block_time);
        self
    }

    /// Sets the database directory path which will be used when the `katana` instance is launched.
    pub fn db_dir<T: Into<PathBuf>>(mut self, db_dir: T) -> Self {
        self.db_dir = Some(db_dir.into());
        self
    }

    /// Sets the RPC URL to fork the network from.
    pub fn l1_provider<T: Into<String>>(mut self, rpc_url: T) -> Self {
        self.l1_provider = Some(rpc_url.into());
        self
    }

    /// Enables the dev mode.
    pub const fn dev(mut self, dev: bool) -> Self {
        self.dev = dev;
        self
    }

    /// Enables JSON logging.
    pub const fn json_log(mut self, json_log: bool) -> Self {
        self.json_log = json_log;
        self
    }

    /// Sets the fork block number which will be used when the `katana` instance is launched.
    pub const fn fork_block_number(mut self, fork_block_number: u64) -> Self {
        self.fork_block_number = Some(fork_block_number);
        self
    }

    /// Sets the messaging configuration path which will be used when the `katana` instance is
    /// launched.
    pub fn messaging<T: Into<PathBuf>>(mut self, messaging: T) -> Self {
        self.messaging = Some(messaging.into());
        self
    }

    /// Enables Prometheus metrics and sets the metrics server address.
    pub fn metrics_addr<T: Into<SocketAddr>>(mut self, addr: T) -> Self {
        self.metrics_addr = Some(addr.into());
        self
    }

    /// Enables Prometheus metrics and sets the metrics server port.
    pub fn metrics_port<T: Into<u16>>(mut self, port: T) -> Self {
        self.metrics_port = Some(port.into());
        self
    }

    /// Sets the host IP address the server will listen on.
    pub fn http_addr<T: Into<SocketAddr>>(mut self, addr: T) -> Self {
        self.http_addr = Some(addr.into());
        self
    }

    /// Sets the maximum number of concurrent connections allowed.
    pub const fn rpc_max_connections(mut self, max_connections: u64) -> Self {
        self.rpc_max_connections = Some(max_connections);
        self
    }

    /// Sets the maximum timeout for the RPC.
    pub const fn rpc_timeout_ms(mut self, timeout_ms: u64) -> Self {
        self.rpc_timeout_ms = Some(timeout_ms);
        self
    }

    /// Sets the maximum gas for the `starknet_call` RPC method.
    pub const fn rpc_max_call_gas(mut self, max_call_gas: u64) -> Self {
        self.rpc_max_call_gas = Some(max_call_gas);
        self
    }

    /// Enables the CORS layer and sets the allowed origins, separated by commas.
    pub fn http_cors_domain<T: Into<String>>(mut self, allowed_origins: T) -> Self {
        self.http_cors_domain = Some(allowed_origins.into());
        self
    }

    /// Sets the seed for randomness of accounts to be predeployed.
    pub const fn seed(mut self, seed: u64) -> Self {
        self.seed = Some(seed);
        self
    }

    /// Sets the number of pre-funded accounts to generate.
    pub fn accounts(mut self, accounts: u16) -> Self {
        self.accounts = Some(accounts);
        self
    }

    /// Enable or disable charging fee when executing transactions.
    /// Enabled by default.
    pub const fn fee(mut self, enable: bool) -> Self {
        self.disable_fee = !enable;
        self
    }

    /// Enables or disable transaction validation.
    /// Enabled by default.
    pub const fn validate(mut self, enable: bool) -> Self {
        self.disable_validate = !enable;
        self
    }

    /// Sets the chain ID.
    pub const fn chain_id(mut self, id: Felt) -> Self {
        self.chain_id = Some(id);
        self
    }

    /// Sets the maximum number of steps available for the account validation logic.
    pub const fn validate_max_steps(mut self, validate_max_steps: u64) -> Self {
        self.validate_max_steps = Some(validate_max_steps);
        self
    }

    /// Sets the maximum number of steps available for the account execution logic.
    pub const fn invoke_max_steps(mut self, invoke_max_steps: u64) -> Self {
        self.invoke_max_steps = Some(invoke_max_steps);
        self
    }

    /// Sets the L1 ETH gas price (denominated in wei).
    pub const fn eth_gas_price(mut self, eth_gas_price: u64) -> Self {
        self.eth_gas_price = Some(eth_gas_price);
        self
    }

    /// Sets the L1 STRK gas price (denominated in fri).
    pub const fn strk_gas_price(mut self, strk_gas_price: u64) -> Self {
        self.strk_gas_price = Some(strk_gas_price);
        self
    }

    /// Sets the genesis configuration path.
    pub fn genesis<T: Into<PathBuf>>(mut self, genesis: T) -> Self {
        self.genesis = Some(genesis.into());
        self
    }

    /// Sets the timeout which will be used when the `katana` instance is launched.
    pub const fn timeout(mut self, timeout: u64) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Disable auto and interval mining, and mine on demand instead via an endpoint.
    pub const fn no_mining(mut self, no_mining: bool) -> Self {
        self.no_mining = no_mining;
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

        if let Some(host) = self.http_addr {
            cmd.arg("--http.addr").arg(host.to_string());
        }

        // In the case where port 0 is set, we will need to extract the actual port number
        // from the logs.
        let mut port = self.http_port.unwrap_or(0);
        cmd.arg("--http.port").arg(port.to_string());

        if self.no_mining {
            cmd.arg("--no-mining");
        }

        if let Some(block_time) = self.block_time {
            cmd.arg("-b").arg(block_time.to_string());
        }

        if let Some(db_dir) = self.db_dir {
            cmd.arg("--db-dir").arg(db_dir);
        }

        if let Some(url) = self.l1_provider {
            cmd.arg("--l1.provider").arg(url);
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

        if self.json_log {
            cmd.args(["--log.format", "json"]);
        }

        if let Some(fork_block_number) = self.fork_block_number {
            cmd.args(["--fork", "--fork.block"]).arg(fork_block_number.to_string());
        }

        if let Some(messaging) = self.messaging {
            cmd.arg("--messaging").arg(messaging);
        }

        // Need to make sure that the `--metrics` is not being set twice.
        let mut metrics_enabled = false;

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
            }

            cmd.arg("--metrics.port").arg(port.to_string());
        }

        if let Some(max_connections) = self.rpc_max_connections {
            cmd.arg("--rpc.max-connections").arg(max_connections.to_string());
        }

        if let Some(timeout_ms) = self.rpc_timeout_ms {
            cmd.arg("--rpc.timeout-ms").arg(timeout_ms.to_string());
        }

        if let Some(max_call_gas) = self.rpc_max_call_gas {
            cmd.arg("--rpc.max-call-gas").arg(max_call_gas.to_string());
        }

        if let Some(allowed_origins) = self.http_cors_domain {
            cmd.arg("--http.corsdomain").arg(allowed_origins);
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

        if let Some(url) = self.cartridge_api_url {
            cmd.arg("--cartridge.api-url").arg(url);
        }

        if self.enable_cartridge_paymaster {
            cmd.arg("--cartridge.paymaster");
        }

        loop {
            if start + Duration::from_millis(self.timeout.unwrap_or(KATANA_STARTUP_TIMEOUT_MILLIS))
                <= Instant::now()
            {
                return Err(Error::Timeout);
            }

            let mut line = String::new();
            reader.read_line(&mut line).map_err(Error::ReadLineError)?;
            trace!(line);

            if self.json_log {
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
        let katana = Katana::new().json_log(true).chain_id(short_string!("SN_SEPOLIA")).spawn();
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
}
