//! Katana node CLI options and configuration.

use std::path::PathBuf;
use std::sync::Arc;

use alloy_primitives::U256;
#[cfg(feature = "server")]
use anyhow::bail;
use anyhow::{Context, Result};
pub use clap::Parser;
use katana_chain_spec::rollup::ChainConfigDir;
use katana_chain_spec::ChainSpec;
use katana_core::constants::DEFAULT_SEQUENCER_ADDRESS;
use katana_messaging::MessagingConfig;
use katana_node::config::db::DbConfig;
use katana_node::config::dev::{DevConfig, FixedL1GasPriceConfig};
use katana_node::config::execution::ExecutionConfig;
use katana_node::config::fork::ForkingConfig;
use katana_node::config::metrics::MetricsConfig;
#[cfg(feature = "cartridge")]
use katana_node::config::paymaster::PaymasterConfig;
use katana_node::config::rpc::RpcConfig;
#[cfg(feature = "server")]
use katana_node::config::rpc::{RpcModuleKind, RpcModulesList};
use katana_node::config::sequencing::SequencingConfig;
use katana_node::config::Config;
use katana_node::Node;
use katana_primitives::genesis::allocation::DevAllocationsGenerator;
use katana_primitives::genesis::constant::DEFAULT_PREFUNDED_ACCOUNT_BALANCE;
use serde::{Deserialize, Serialize};
use tracing::info;
use url::Url;

use crate::file::NodeArgsConfig;
use crate::options::*;
use crate::utils::{self, parse_chain_config_dir, parse_seed};

pub(crate) const LOG_TARGET: &str = "katana::cli";

#[derive(Parser, Debug, Serialize, Deserialize, Default, Clone)]
#[command(next_help_heading = "Node options")]
pub struct NodeArgs {
    /// Don't print anything on startup.
    #[arg(long)]
    pub silent: bool,

    /// Path to the chain configuration file.
    #[arg(long, hide = true)]
    #[arg(value_parser = parse_chain_config_dir)]
    pub chain: Option<ChainConfigDir>,

    /// Disable auto and interval mining, and mine on demand instead via an endpoint.
    #[arg(long)]
    #[arg(conflicts_with = "block_time")]
    pub no_mining: bool,

    /// Block time in milliseconds for interval mining.
    #[arg(short, long)]
    #[arg(value_name = "MILLISECONDS")]
    pub block_time: Option<u64>,

    #[arg(long = "sequencing.block-max-cairo-steps")]
    #[arg(value_name = "TOTAL")]
    pub block_cairo_steps_limit: Option<u64>,

    /// Directory path of the database to initialize from.
    ///
    /// The path must either be an empty directory or a directory which already contains a
    /// previously initialized Katana database.
    #[arg(long)]
    #[arg(value_name = "PATH")]
    pub db_dir: Option<PathBuf>,

    /// Configuration file
    #[arg(long)]
    pub config: Option<PathBuf>,

    /// Configure the messaging with an other chain.
    ///
    /// Configure the messaging to allow Katana listening/sending messages on a
    /// settlement chain that can be Ethereum or an other Starknet sequencer.
    #[arg(long)]
    #[arg(value_name = "PATH")]
    #[arg(value_parser = katana_messaging::MessagingConfig::parse)]
    #[arg(conflicts_with = "chain")]
    pub messaging: Option<MessagingConfig>,

    #[arg(long = "l1.provider", value_name = "URL", alias = "l1-provider")]
    #[arg(help = "The Ethereum RPC provider to sample the gas prices from to enable the gas \
                  price oracle.")]
    pub l1_provider_url: Option<Url>,

    #[command(flatten)]
    pub logging: LoggingOptions,

    #[command(flatten)]
    pub tracer: TracerOptions,

    #[cfg(feature = "server")]
    #[command(flatten)]
    pub metrics: MetricsOptions,

    #[cfg(feature = "server")]
    #[command(flatten)]
    pub server: ServerOptions,

    #[command(flatten)]
    pub starknet: StarknetOptions,

    #[command(flatten)]
    pub gpo: GasPriceOracleOptions,

    #[command(flatten)]
    pub forking: ForkingOptions,

    #[command(flatten)]
    pub development: DevOptions,

    #[cfg(feature = "explorer")]
    #[command(flatten)]
    pub explorer: ExplorerOptions,

    #[cfg(feature = "cartridge")]
    #[command(flatten)]
    pub cartridge: CartridgeOptions,
}

impl NodeArgs {
    pub async fn execute(&self) -> Result<()> {
        // Initialize logging with tracer
        let tracer_config = self.tracer_config();
        katana_tracing::init(self.logging.log_format, tracer_config).await?;
        self.start_node().await
    }

    async fn start_node(&self) -> Result<()> {
        // Build the node
        let config = self.config()?;
        let node = Node::build(config).await.context("failed to build node")?;

        if !self.silent {
            utils::print_intro(self, &node.backend().chain_spec);
        }

        // Launch the node
        let handle = node.launch().await.context("failed to launch node")?;

        // Wait until an OS signal (ie SIGINT, SIGTERM) is received or the node is shutdown.
        tokio::select! {
            _ = dojo_utils::signal::wait_signals() => {
                // Gracefully shutdown the node before exiting
                handle.stop().await?;
            },

            _ = handle.stopped() => { }
        }

        info!("Shutting down.");

        Ok(())
    }

    pub fn config(&self) -> Result<katana_node::config::Config> {
        let db = self.db_config();
        let rpc = self.rpc_config()?;
        let dev = self.dev_config();
        let (chain, cs_messaging) = self.chain_spec()?;
        let metrics = self.metrics_config();
        let forking = self.forking_config()?;
        let execution = self.execution_config();
        let sequencing = self.sequencer_config();

        // the `katana init` will automatically generate a messaging config. so if katana is run
        // with `--chain` then the `--messaging` flag is not required. this is temporary and
        // the messagign config will eventually be removed slowly.
        let messaging = if cs_messaging.is_some() { cs_messaging } else { self.messaging.clone() };

        #[cfg(feature = "cartridge")]
        {
            let paymaster = self.cartridge_config();

            Ok(Config {
                db,
                dev,
                rpc,
                chain,
                metrics,
                forking,
                execution,
                messaging,
                paymaster,
                sequencing,
            })
        }

        #[cfg(not(feature = "cartridge"))]
        Ok(Config { metrics, db, dev, rpc, chain, execution, sequencing, messaging, forking })
    }

    fn sequencer_config(&self) -> SequencingConfig {
        SequencingConfig {
            block_time: self.block_time,
            no_mining: self.no_mining,
            block_cairo_steps_limit: self.block_cairo_steps_limit,
        }
    }

    pub fn rpc_config(&self) -> Result<RpcConfig> {
        #[cfg(feature = "server")]
        {
            use std::time::Duration;

            #[allow(unused_mut)]
            let mut modules = if let Some(modules) = &self.server.http_modules {
                // TODO: This check should be handled in the `katana-node` level. Right now if you
                // instantiate katana programmatically, you can still add the dev module without
                // enabling dev mode.
                //
                // We only allow the `dev` module in dev mode (ie `--dev` flag)
                if !self.development.dev && modules.contains(&RpcModuleKind::Dev) {
                    bail!("The `dev` module can only be enabled in dev mode (ie `--dev` flag)")
                }

                modules.clone()
            } else {
                // Expose the default modules if none is specified.
                let mut modules = RpcModulesList::default();

                // Ensures the `--dev` flag enabled the dev module.
                if self.development.dev {
                    modules.add(RpcModuleKind::Dev);
                }

                modules
            };

            // The cartridge rpc must be enabled if the paymaster is enabled.
            // We put it here so that even when the individual api are explicitly specified
            // (ie `--rpc.api`) we guarantee that the cartridge rpc is enabled.
            #[cfg(feature = "cartridge")]
            if self.cartridge.paymaster {
                modules.add(RpcModuleKind::Cartridge);
            }

            let cors_origins = self.server.http_cors_origins.clone();

            Ok(RpcConfig {
                apis: modules,
                port: self.server.http_port,
                addr: self.server.http_addr,
                max_connections: self.server.max_connections,
                max_concurrent_estimate_fee_requests: None,
                max_request_body_size: None,
                max_response_body_size: None,
                timeout: self.server.timeout.map(Duration::from_secs),
                cors_origins,
                #[cfg(feature = "explorer")]
                explorer: self.explorer.explorer,
                max_event_page_size: Some(self.server.max_event_page_size),
                max_proof_keys: Some(self.server.max_proof_keys),
                max_call_gas: Some(self.server.max_call_gas),
            })
        }

        #[cfg(not(feature = "server"))]
        {
            Ok(RpcConfig::default())
        }
    }

    fn chain_spec(&self) -> Result<(Arc<ChainSpec>, Option<MessagingConfig>)> {
        if let Some(path) = &self.chain {
            let mut cs = katana_chain_spec::rollup::read(path)?;
            cs.genesis.sequencer_address = *DEFAULT_SEQUENCER_ADDRESS;
            let messaging_config = MessagingConfig::from_chain_spec(&cs);
            Ok((Arc::new(ChainSpec::Rollup(cs)), Some(messaging_config)))
        }
        // exclusively for development mode
        else {
            let mut chain_spec = katana_chain_spec::dev::DEV_UNALLOCATED.clone();

            if let Some(id) = self.starknet.environment.chain_id {
                chain_spec.id = id;
            }

            if let Some(genesis) = &self.starknet.genesis {
                chain_spec.genesis = genesis.clone();
            } else {
                chain_spec.genesis.sequencer_address = *DEFAULT_SEQUENCER_ADDRESS;
            }

            // Generate dev accounts.
            // If `cartridge` is enabled, the first account will be the paymaster.
            let accounts = DevAllocationsGenerator::new(self.development.total_accounts)
                .with_seed(parse_seed(&self.development.seed))
                .with_balance(U256::from(DEFAULT_PREFUNDED_ACCOUNT_BALANCE))
                .generate();

            chain_spec.genesis.extend_allocations(accounts.into_iter().map(|(k, v)| (k, v.into())));

            #[cfg(feature = "cartridge")]
            if self.cartridge.controllers || self.cartridge.paymaster {
                katana_slot_controller::add_controller_classes(&mut chain_spec.genesis);
                katana_slot_controller::add_vrf_provider_class(&mut chain_spec.genesis);
            }

            Ok((Arc::new(ChainSpec::Dev(chain_spec)), None))
        }
    }

    fn dev_config(&self) -> DevConfig {
        let mut fixed_gas_prices = None;

        if let Some(eth) = self.gpo.l2_eth_gas_price {
            let prices = fixed_gas_prices.get_or_insert(FixedL1GasPriceConfig::default());
            prices.l2_gas_prices.eth = eth;
        }

        if let Some(strk) = self.gpo.l2_strk_gas_price {
            let prices = fixed_gas_prices.get_or_insert(FixedL1GasPriceConfig::default());
            prices.l2_gas_prices.strk = strk;
        }

        if let Some(eth) = self.gpo.l1_eth_gas_price {
            let prices = fixed_gas_prices.get_or_insert(FixedL1GasPriceConfig::default());
            prices.l1_gas_prices.eth = eth;
        }

        if let Some(strk) = self.gpo.l1_strk_gas_price {
            let prices = fixed_gas_prices.get_or_insert(FixedL1GasPriceConfig::default());
            prices.l1_gas_prices.strk = strk;
        }

        if let Some(eth) = self.gpo.l1_eth_data_gas_price {
            let prices = fixed_gas_prices.get_or_insert(FixedL1GasPriceConfig::default());
            prices.l1_data_gas_prices.eth = eth;
        }

        if let Some(strk) = self.gpo.l1_strk_data_gas_price {
            let prices = fixed_gas_prices.get_or_insert(FixedL1GasPriceConfig::default());
            prices.l1_data_gas_prices.strk = strk;
        }

        DevConfig {
            fixed_gas_prices,
            fee: !self.development.no_fee,
            account_validation: !self.development.no_account_validation,
        }
    }

    fn execution_config(&self) -> ExecutionConfig {
        ExecutionConfig {
            invocation_max_steps: self.starknet.environment.invoke_max_steps,
            validation_max_steps: self.starknet.environment.validate_max_steps,
            #[cfg(feature = "native")]
            compile_native: self.starknet.environment.compile_native,
            ..Default::default()
        }
    }

    fn forking_config(&self) -> Result<Option<ForkingConfig>> {
        if let Some(ref url) = self.forking.fork_provider {
            let cfg = ForkingConfig { url: url.clone(), block: self.forking.fork_block };
            return Ok(Some(cfg));
        }

        Ok(None)
    }

    fn db_config(&self) -> DbConfig {
        DbConfig { dir: self.db_dir.clone() }
    }

    fn metrics_config(&self) -> Option<MetricsConfig> {
        #[cfg(feature = "server")]
        if self.metrics.metrics {
            Some(MetricsConfig { addr: self.metrics.metrics_addr, port: self.metrics.metrics_port })
        } else {
            None
        }

        #[cfg(not(feature = "server"))]
        None
    }

    #[cfg(feature = "cartridge")]
    fn cartridge_config(&self) -> Option<PaymasterConfig> {
        if self.cartridge.paymaster {
            Some(PaymasterConfig { cartridge_api_url: self.cartridge.api.clone() })
        } else {
            None
        }
    }

    /// Parse the node config from the command line arguments and the config file,
    /// and merge them together prioritizing the command line arguments.
    pub fn with_config_file(mut self) -> Result<Self> {
        let config = if let Some(path) = &self.config {
            NodeArgsConfig::read(path)?
        } else {
            return Ok(self);
        };

        // the CLI (self) takes precedence over the config file.
        // Currently, the merge is made at the top level of the commands.
        // We may add recursive merging in the future.

        if !self.no_mining {
            self.no_mining = config.no_mining.unwrap_or_default();
        }

        if self.block_time.is_none() {
            self.block_time = config.block_time;
        }

        if self.db_dir.is_none() {
            self.db_dir = config.db_dir;
        }

        if self.logging == LoggingOptions::default() {
            if let Some(logging) = config.logging {
                self.logging = logging;
            }
        }

        if self.messaging.is_none() {
            self.messaging = config.messaging;
        }

        #[cfg(feature = "server")]
        {
            self.server.merge(config.server.as_ref());

            if self.metrics == MetricsOptions::default() {
                if let Some(metrics) = config.metrics {
                    self.metrics = metrics;
                }
            }
        }

        self.starknet.merge(config.starknet.as_ref());
        self.development.merge(config.development.as_ref());

        if self.gpo == GasPriceOracleOptions::default() {
            if let Some(gpo) = config.gpo {
                self.gpo = gpo;
            }
        }

        if self.forking == ForkingOptions::default() {
            if let Some(forking) = config.forking {
                self.forking = forking;
            }
        }

        #[cfg(feature = "cartridge")]
        {
            self.cartridge.merge(config.cartridge.as_ref());
        }

        Ok(self)
    }

    fn tracer_config(&self) -> Option<katana_tracing::TracerConfig> {
        self.tracer.config()
    }
}

#[cfg(test)]
mod test {
    use std::str::FromStr;

    use assert_matches::assert_matches;
    use katana_gas_price_oracle::{
        DEFAULT_ETH_L1_DATA_GAS_PRICE, DEFAULT_ETH_L1_GAS_PRICE, DEFAULT_ETH_L2_GAS_PRICE,
        DEFAULT_STRK_L1_DATA_GAS_PRICE, DEFAULT_STRK_L1_GAS_PRICE,
    };
    use katana_node::config::execution::{
        DEFAULT_INVOCATION_MAX_STEPS, DEFAULT_VALIDATION_MAX_STEPS,
    };
    use katana_node::config::rpc::RpcModuleKind;
    use katana_primitives::chain::ChainId;
    use katana_primitives::{address, felt, ContractAddress, Felt};

    use super::*;

    #[test]
    fn test_starknet_config_default() {
        let args = NodeArgs::parse_from(["katana"]);
        let config = args.config().unwrap();

        assert!(config.dev.fee);
        assert!(config.dev.account_validation);
        assert!(config.forking.is_none());
        assert_eq!(config.execution.invocation_max_steps, DEFAULT_INVOCATION_MAX_STEPS);
        assert_eq!(config.execution.validation_max_steps, DEFAULT_VALIDATION_MAX_STEPS);
        assert_eq!(config.db.dir, None);
        assert_eq!(config.chain.id(), ChainId::parse("KATANA").unwrap());
        assert_eq!(config.chain.genesis().sequencer_address, *DEFAULT_SEQUENCER_ADDRESS);
    }

    #[test]
    fn test_starknet_config_custom() {
        let args = NodeArgs::parse_from([
            "katana",
            "--dev",
            "--dev.no-fee",
            "--dev.no-account-validation",
            "--chain-id",
            "SN_GOERLI",
            "--invoke-max-steps",
            "200",
            "--validate-max-steps",
            "100",
            "--db-dir",
            "/path/to/db",
        ]);
        let config = args.config().unwrap();

        assert!(!config.dev.fee);
        assert!(!config.dev.account_validation);
        assert_eq!(config.execution.invocation_max_steps, 200);
        assert_eq!(config.execution.validation_max_steps, 100);
        assert_eq!(config.db.dir, Some(PathBuf::from("/path/to/db")));
        assert_eq!(config.chain.id(), ChainId::GOERLI);
        assert_eq!(config.chain.genesis().sequencer_address, *DEFAULT_SEQUENCER_ADDRESS);
    }

    #[test]
    fn custom_fixed_gas_prices() {
        let config = NodeArgs::parse_from(["katana"]).config().unwrap();
        assert!(config.dev.fixed_gas_prices.is_none());

        let config =
            NodeArgs::parse_from(["katana", "--gpo.l1-eth-gas-price", "10"]).config().unwrap();
        assert_matches!(config.dev.fixed_gas_prices, Some(prices) => {
            assert_eq!(prices.l1_gas_prices.eth.get(), 10);
            assert_eq!(prices.l1_gas_prices.strk, DEFAULT_ETH_L2_GAS_PRICE);
            assert_eq!(prices.l1_data_gas_prices.eth, DEFAULT_ETH_L1_DATA_GAS_PRICE);
            assert_eq!(prices.l1_data_gas_prices.strk, DEFAULT_STRK_L1_DATA_GAS_PRICE);
        });

        let config =
            NodeArgs::parse_from(["katana", "--gpo.l1-strk-gas-price", "20"]).config().unwrap();
        assert_matches!(config.dev.fixed_gas_prices, Some(prices) => {
            assert_eq!(prices.l1_gas_prices.eth, DEFAULT_ETH_L1_GAS_PRICE);
            assert_eq!(prices.l1_gas_prices.strk.get(), 20);
            assert_eq!(prices.l1_data_gas_prices.eth, DEFAULT_ETH_L1_DATA_GAS_PRICE);
            assert_eq!(prices.l1_data_gas_prices.strk, DEFAULT_STRK_L1_DATA_GAS_PRICE);
        });

        let config =
            NodeArgs::parse_from(["katana", "--gpo.l1-eth-data-gas-price", "2"]).config().unwrap();
        assert_matches!(config.dev.fixed_gas_prices, Some(prices) => {
            assert_eq!(prices.l1_gas_prices.eth, DEFAULT_ETH_L1_GAS_PRICE);
            assert_eq!(prices.l1_gas_prices.strk, DEFAULT_STRK_L1_GAS_PRICE);
            assert_eq!(prices.l1_data_gas_prices.eth.get(), 2);
            assert_eq!(prices.l1_data_gas_prices.strk, DEFAULT_STRK_L1_DATA_GAS_PRICE);
        });

        let config =
            NodeArgs::parse_from(["katana", "--gpo.l1-strk-data-gas-price", "2"]).config().unwrap();
        assert_matches!(config.dev.fixed_gas_prices, Some(prices) => {
            assert_eq!(prices.l1_gas_prices.eth, DEFAULT_ETH_L1_GAS_PRICE);
            assert_eq!(prices.l1_gas_prices.strk, DEFAULT_STRK_L1_GAS_PRICE);
            assert_eq!(prices.l1_data_gas_prices.eth, DEFAULT_ETH_L1_DATA_GAS_PRICE);
            assert_eq!(prices.l1_data_gas_prices.strk.get(), 2);
        });

        let config = NodeArgs::parse_from([
            "katana",
            "--gpo.l1-eth-gas-price",
            "10",
            "--gpo.l1-strk-data-gas-price",
            "2",
        ])
        .config()
        .unwrap();

        assert_matches!(config.dev.fixed_gas_prices, Some(prices) => {
            assert_eq!(prices.l1_gas_prices.eth.get(), 10);
            assert_eq!(prices.l1_gas_prices.strk, DEFAULT_STRK_L1_GAS_PRICE);
            assert_eq!(prices.l1_data_gas_prices.eth, DEFAULT_ETH_L1_DATA_GAS_PRICE);
            assert_eq!(prices.l1_data_gas_prices.strk.get(), 2);
        });

        // Set all the gas prices options

        let config = NodeArgs::parse_from([
            "katana",
            "--gpo.l1-eth-gas-price",
            "10",
            "--gpo.l1-strk-gas-price",
            "20",
            "--gpo.l1-eth-data-gas-price",
            "1",
            "--gpo.l1-strk-data-gas-price",
            "2",
        ])
        .config()
        .unwrap();

        assert_matches!(config.dev.fixed_gas_prices, Some(prices) => {
            assert_eq!(prices.l1_gas_prices.eth.get(), 10);
            assert_eq!(prices.l1_gas_prices.strk.get(), 20);
            assert_eq!(prices.l1_data_gas_prices.eth.get(), 1);
            assert_eq!(prices.l1_data_gas_prices.strk.get(), 2);
        })
    }

    #[test]
    fn genesis_with_fixed_gas_prices() {
        let config = NodeArgs::parse_from([
            "katana",
            "--genesis",
            "./test-data/genesis.json",
            "--gpo.l1-eth-gas-price",
            "100",
            "--gpo.l1-strk-gas-price",
            "200",
            "--gpo.l1-eth-data-gas-price",
            "111",
            "--gpo.l1-strk-data-gas-price",
            "222",
        ])
        .config()
        .unwrap();

        assert_eq!(config.chain.genesis().number, 0);
        assert_eq!(config.chain.genesis().parent_hash, felt!("0x999"));
        assert_eq!(config.chain.genesis().timestamp, 5123512314);
        assert_eq!(config.chain.genesis().state_root, felt!("0x99"));
        assert_eq!(config.chain.genesis().sequencer_address, address!("0x100"));
        assert_eq!(config.chain.genesis().gas_prices.eth.get(), 9999);
        assert_eq!(config.chain.genesis().gas_prices.strk.get(), 8888);
        assert_matches!(config.dev.fixed_gas_prices, Some(prices) => {
            assert_eq!(prices.l1_gas_prices.eth.get(), 100);
            assert_eq!(prices.l1_gas_prices.strk.get(), 200);
            assert_eq!(prices.l1_data_gas_prices.eth.get(), 111);
            assert_eq!(prices.l1_data_gas_prices.strk.get(), 222);
        })
    }

    #[test]
    fn config_from_file_and_cli() {
        // CLI args must take precedence over the config file.
        let content = r#"
[gpo]
l1_eth_gas_price = "0xfe"
l1_strk_gas_price = "200"
l1_eth_data_gas_price = "111"
l1_strk_data_gas_price = "222"

[dev]
total_accounts = 20

[starknet.env]
validate_max_steps = 500
invoke_max_steps = 9988
chain_id.Named = "Mainnet"
        "#;
        let path = std::env::temp_dir().join("katana-config.json");
        std::fs::write(&path, content).unwrap();

        let path_str = path.to_string_lossy().to_string();

        let args = vec![
            "katana",
            "--config",
            path_str.as_str(),
            "--genesis",
            "./test-data/genesis.json",
            "--validate-max-steps",
            "1234",
            "--dev",
            "--dev.no-fee",
            "--chain-id",
            "0x123",
        ];

        let config =
            NodeArgs::parse_from(args.clone()).with_config_file().unwrap().config().unwrap();

        assert_eq!(config.execution.validation_max_steps, 1234);
        assert_eq!(config.execution.invocation_max_steps, 9988);
        assert!(!config.dev.fee);
        assert_matches!(config.dev.fixed_gas_prices, Some(prices) => {
            assert_eq!(prices.l1_gas_prices.eth.get(), 254);
            assert_eq!(prices.l1_gas_prices.strk.get(), 200);
            assert_eq!(prices.l1_data_gas_prices.eth.get(), 111);
            assert_eq!(prices.l1_data_gas_prices.strk.get(), 222);
        });
        assert_eq!(config.chain.genesis().number, 0);
        assert_eq!(config.chain.genesis().parent_hash, felt!("0x999"));
        assert_eq!(config.chain.genesis().timestamp, 5123512314);
        assert_eq!(config.chain.genesis().state_root, felt!("0x99"));
        assert_eq!(config.chain.genesis().sequencer_address, address!("0x100"));
        assert_eq!(config.chain.genesis().gas_prices.eth.get(), 9999);
        assert_eq!(config.chain.genesis().gas_prices.strk.get(), 8888);
        assert_eq!(config.chain.id(), ChainId::Id(Felt::from_str("0x123").unwrap()));
    }

    #[test]
    #[cfg(feature = "server")]
    fn parse_cors_origins() {
        use katana_rpc::cors::HeaderValue;

        let config = NodeArgs::parse_from([
            "katana",
            "--http.cors_origins",
            "*,http://localhost:3000,https://example.com",
        ])
        .config()
        .unwrap();

        let cors_origins = config.rpc.cors_origins;

        assert_eq!(cors_origins.len(), 3);
        assert!(cors_origins.contains(&HeaderValue::from_static("*")));
        assert!(cors_origins.contains(&HeaderValue::from_static("http://localhost:3000")));
        assert!(cors_origins.contains(&HeaderValue::from_static("https://example.com")));
    }

    #[test]
    fn http_modules() {
        // If the `--http.api` isn't specified, only starknet module will be exposed.
        let config = NodeArgs::parse_from(["katana"]).config().unwrap();
        let modules = config.rpc.apis;
        assert_eq!(modules.len(), 1);
        assert!(modules.contains(&RpcModuleKind::Starknet));

        // If the `--http.api` is specified, only the ones in the list will be exposed.
        let config = NodeArgs::parse_from(["katana", "--http.api", "starknet"]).config().unwrap();
        let modules = config.rpc.apis;
        assert_eq!(modules.len(), 1);
        assert!(modules.contains(&RpcModuleKind::Starknet));

        // Specifiying the dev module without enabling dev mode is forbidden.
        let err =
            NodeArgs::parse_from(["katana", "--http.api", "starknet,dev"]).config().unwrap_err();
        assert!(err
            .to_string()
            .contains("The `dev` module can only be enabled in dev mode (ie `--dev` flag)"));
    }

    #[test]
    fn test_dev_api_enabled() {
        let args = NodeArgs::parse_from(["katana", "--dev"]);
        let config = args.config().unwrap();

        assert!(config.rpc.apis.contains(&RpcModuleKind::Dev));
    }

    #[cfg(feature = "cartridge")]
    #[test]
    fn cartridge_paymaster() {
        let args = NodeArgs::parse_from(["katana", "--cartridge.paymaster"]);
        let config = args.config().unwrap();

        // Verify cartridge module is automatically enabled
        assert!(config.rpc.apis.contains(&RpcModuleKind::Cartridge));

        // Test with paymaster explicitly specified in RPC modules
        let args =
            NodeArgs::parse_from(["katana", "--cartridge.paymaster", "--http.api", "starknet"]);
        let config = args.config().unwrap();

        // Verify cartridge module is still enabled even when not in explicit RPC list
        assert!(config.rpc.apis.contains(&RpcModuleKind::Cartridge));
        assert!(config.rpc.apis.contains(&RpcModuleKind::Starknet));

        // Verify that all the Controller classes are added to the genesis
        for (_, class) in katana_slot_controller::CONTROLLERS.iter() {
            assert!(config.chain.genesis().classes.get(&class.hash).is_some());
        }

        // Test without paymaster enabled
        let args = NodeArgs::parse_from(["katana"]);
        let config = args.config().unwrap();

        // Verify cartridge module is not enabled by default
        assert!(!config.rpc.apis.contains(&RpcModuleKind::Cartridge));

        for (_, class) in katana_slot_controller::CONTROLLERS.iter() {
            assert!(config.chain.genesis().classes.get(&class.hash).is_none());
        }
    }
}
