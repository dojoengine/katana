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
use katana_genesis::allocation::DevAllocationsGenerator;
use katana_genesis::constant::DEFAULT_PREFUNDED_ACCOUNT_BALANCE;
use katana_messaging::MessagingConfig;
use katana_node::config::db::DbConfig;
use katana_node::config::dev::{DevConfig, FixedL1GasPriceConfig};
use katana_node::config::execution::ExecutionConfig;
use katana_node::config::fork::ForkingConfig;
use katana_node::config::gateway::GatewayConfig;
#[cfg(all(feature = "server", feature = "grpc"))]
use katana_node::config::grpc::GrpcConfig;
use katana_node::config::metrics::MetricsConfig;
#[cfg(feature = "paymaster")]
use katana_node::config::paymaster::PaymasterConfig;
#[cfg(feature = "vrf")]
use katana_node::config::paymaster::{VrfConfig, VrfKeySource as NodeVrfKeySource};
use katana_node::config::rpc::RpcConfig;
#[cfg(feature = "server")]
use katana_node::config::rpc::{RpcModuleKind, RpcModulesList};
use katana_node::config::sequencing::SequencingConfig;
#[cfg(feature = "tee")]
use katana_node::config::tee::TeeConfig;
use katana_node::config::Config;
use katana_node::Node;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};
use url::Url;

use crate::file::NodeArgsConfig;
use crate::options::*;
use crate::utils::{self, parse_chain_config_dir, parse_seed};

pub(crate) const LOG_TARGET: &str = "katana::cli";
#[cfg(feature = "paymaster")]
const DEFAULT_PAYMASTER_API_KEY: &str = "paymaster_katana";

/// Sidecar-specific info for paymaster (used by CLI to start sidecar process).
#[cfg(feature = "paymaster")]
#[derive(Debug, Clone)]
pub struct PaymasterSidecarInfo {
    pub port: u16,
    pub api_key: String,
}

/// Sidecar-specific info for VRF (used by CLI to start sidecar process).
#[cfg(feature = "vrf")]
#[derive(Debug, Clone)]
pub struct VrfSidecarInfo {
    pub port: u16,
}

/// Node configuration with optional sidecar info.
///
/// This struct holds the node configuration along with any sidecar-specific
/// information needed by the CLI to start sidecar processes.
#[derive(Debug)]
pub struct NodeConfigWithSidecars {
    pub config: Config,
    #[cfg(feature = "paymaster")]
    pub paymaster_sidecar: Option<PaymasterSidecarInfo>,
    #[cfg(feature = "vrf")]
    pub vrf_sidecar: Option<VrfSidecarInfo>,
}

#[derive(Parser, Debug, Serialize, Deserialize, Default, Clone, PartialEq)]
#[command(next_help_heading = "Sequencer node options")]
pub struct SequencerNodeArgs {
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
    #[arg(long, alias = "db-dir")]
    #[arg(value_name = "PATH")]
    pub data_dir: Option<PathBuf>,

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
    pub gateway: GatewayOptions,

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

    #[cfg(feature = "paymaster")]
    #[command(flatten)]
    pub paymaster: PaymasterOptions,

    #[cfg(feature = "vrf")]
    #[command(flatten)]
    pub vrf: VrfOptions,

    #[cfg(feature = "tee")]
    #[command(flatten)]
    pub tee: TeeOptions,

    #[cfg(all(feature = "server", feature = "grpc"))]
    #[command(flatten)]
    pub grpc: GrpcOptions,
}

impl SequencerNodeArgs {
    pub async fn execute(&self) -> Result<()> {
        let logging = katana_tracing::LoggingConfig {
            stdout_format: self.logging.stdout.stdout_format,
            stdout_color: self.logging.stdout.color,
            file_enabled: self.logging.file.enabled,
            file_format: self.logging.file.file_format,
            file_directory: self.logging.file.directory.clone(),
            file_max_files: self.logging.file.max_files,
        };

        katana_tracing::init(logging, self.tracer_config()).await?;

        self.start_node().await
    }

    async fn start_node(&self) -> Result<()> {
        // Build the node configuration with sidecar info
        let NodeConfigWithSidecars {
            config,
            #[cfg(feature = "paymaster")]
            paymaster_sidecar,
            #[cfg(feature = "vrf")]
            vrf_sidecar,
        } = self.config()?;

        if config.forking.is_some() {
            let node = Node::build_forked(config).await.context("failed to build forked node")?;

            if !self.silent {
                utils::print_intro(self, &node.backend().chain_spec);
            }

            // Launch the node
            let handle = node.launch().await.context("failed to launch forked node")?;

            // Bootstrap and start sidecars if needed
            #[cfg(feature = "paymaster")]
            let mut sidecars = bootstrap_and_start_sidecars(
                self,
                handle.node(),
                handle.rpc().addr(),
                paymaster_sidecar.as_ref(),
                #[cfg(feature = "vrf")]
                vrf_sidecar.as_ref(),
            )
            .await?;

            // Wait until an OS signal (ie SIGINT, SIGTERM) is received or the node is shutdown.
            tokio::select! {
                _ = katana_utils::wait_shutdown_signals() => {
                    // Gracefully shutdown the node before exiting
                    handle.stop().await?;
                },

                _ = handle.stopped() => { }
            }

            // Shutdown sidecar processes
            #[cfg(feature = "paymaster")]
            if let Some(ref mut s) = sidecars {
                s.shutdown().await;
            }
        } else {
            let node = Node::build(config).context("failed to build node")?;

            if !self.silent {
                utils::print_intro(self, &node.backend().chain_spec);
            }

            // Launch the node
            let handle = node.launch().await.context("failed to launch node")?;

            // Bootstrap and start sidecars if needed
            #[cfg(feature = "paymaster")]
            let mut sidecars = bootstrap_and_start_sidecars(
                self,
                handle.node(),
                handle.rpc().addr(),
                paymaster_sidecar.as_ref(),
                #[cfg(feature = "vrf")]
                vrf_sidecar.as_ref(),
            )
            .await?;

            // Wait until an OS signal (ie SIGINT, SIGTERM) is received or the node is shutdown.
            tokio::select! {
                _ = katana_utils::wait_shutdown_signals() => {
                    // Gracefully shutdown the node before exiting
                    handle.stop().await?;
                },

                _ = handle.stopped() => { }
            }

            // Shutdown sidecar processes
            #[cfg(feature = "paymaster")]
            if let Some(ref mut s) = sidecars {
                s.shutdown().await;
            }
        }

        info!("Shutting down.");

        Ok(())
    }

    pub fn config(&self) -> Result<NodeConfigWithSidecars> {
        let db = self.db_config();
        let rpc = self.rpc_config()?;
        let dev = self.dev_config();
        let (chain, cs_messaging) = self.chain_spec()?;
        let metrics = self.metrics_config();
        let gateway = self.gateway_config();
        #[cfg(all(feature = "server", feature = "grpc"))]
        let grpc = self.grpc_config();
        let forking = self.forking_config()?;
        let execution = self.execution_config();
        let sequencing = self.sequencer_config();

        #[cfg(feature = "paymaster")]
        let (paymaster, paymaster_sidecar) = match self.paymaster_config()? {
            Some((config, sidecar)) => (Some(config), sidecar),
            None => (None, None),
        };

        #[cfg(feature = "vrf")]
        let (vrf, vrf_sidecar) = match self.vrf_config()? {
            Some((config, sidecar)) => (Some(config), sidecar),
            None => (None, None),
        };

        #[cfg(feature = "vrf")]
        if vrf.is_some() && paymaster.is_none() {
            return Err(anyhow::anyhow!("--vrf requires paymaster; enable --paymaster"));
        }

        // the `katana init` will automatically generate a messaging config. so if katana is run
        // with `--chain` then the `--messaging` flag is not required. this is temporary and
        // the messagign config will eventually be removed slowly.
        let messaging = if cs_messaging.is_some() { cs_messaging } else { self.messaging.clone() };

        let config = Config {
            db,
            dev,
            rpc,
            #[cfg(feature = "grpc")]
            grpc,
            chain,
            metrics,
            gateway,
            forking,
            execution,
            messaging,
            sequencing,
            #[cfg(feature = "paymaster")]
            paymaster,
            #[cfg(feature = "vrf")]
            vrf,
            #[cfg(feature = "tee")]
            tee: self.tee_config(),
        };

        Ok(NodeConfigWithSidecars {
            config,
            #[cfg(feature = "paymaster")]
            paymaster_sidecar,
            #[cfg(feature = "vrf")]
            vrf_sidecar,
        })
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
            {
                #[cfg(feature = "paymaster")]
                let paymaster_enabled = self.paymaster.is_enabled();
                #[cfg(not(feature = "paymaster"))]
                let paymaster_enabled = false;

                #[cfg(feature = "vrf")]
                let vrf_enabled = self.vrf.is_enabled();
                #[cfg(not(feature = "vrf"))]
                let vrf_enabled = false;

                if paymaster_enabled || vrf_enabled {
                    modules.add(RpcModuleKind::Cartridge);
                }
            }

            // The TEE rpc must be enabled if a TEE provider is specified.
            // We put it here so that even when the individual api are explicitly specified
            // (ie `--rpc.api`) we guarantee that the tee rpc is enabled.
            #[cfg(feature = "tee")]
            if self.tee.tee_provider.is_some() {
                modules.add(RpcModuleKind::Tee);
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
            // If paymaster is enabled, the first account is used by default.
            let accounts = DevAllocationsGenerator::new(self.development.total_accounts)
                .with_seed(parse_seed(&self.development.seed))
                .with_balance(U256::from(DEFAULT_PREFUNDED_ACCOUNT_BALANCE))
                .generate();

            chain_spec.genesis.extend_allocations(accounts.into_iter().map(|(k, v)| (k, v.into())));

            #[cfg(feature = "cartridge")]
            {
                if self.cartridge.controllers {
                    katana_slot_controller::add_controller_classes(&mut chain_spec.genesis);
                    katana_slot_controller::add_vrf_provider_class(&mut chain_spec.genesis);
                }
            }

            #[cfg(feature = "paymaster")]
            {
                if self.paymaster.is_enabled() {
                    katana_slot_controller::add_avnu_forwarder_class(&mut chain_spec.genesis);
                }
            }

            #[cfg(feature = "vrf")]
            {
                if self.vrf.is_enabled() {
                    katana_slot_controller::add_vrf_account_class(&mut chain_spec.genesis);
                    katana_slot_controller::add_vrf_consumer_class(&mut chain_spec.genesis);
                }
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
        DbConfig { dir: self.data_dir.clone() }
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

    fn gateway_config(&self) -> Option<GatewayConfig> {
        #[cfg(feature = "server")]
        if self.gateway.gateway_enable {
            use std::time::Duration;

            Some(GatewayConfig {
                addr: self.gateway.gateway_addr,
                port: self.gateway.gateway_port,
                timeout: Some(Duration::from_secs(self.gateway.gateway_timeout)),
            })
        } else {
            None
        }

        #[cfg(not(feature = "server"))]
        None
    }

    #[cfg(all(feature = "server", feature = "grpc"))]
    fn grpc_config(&self) -> Option<GrpcConfig> {
        if self.grpc.grpc_enable {
            use std::time::Duration;

            Some(GrpcConfig {
                addr: self.grpc.grpc_addr,
                port: self.grpc.grpc_port,
                timeout: self.grpc.grpc_timeout.map(Duration::from_secs),
            })
        } else {
            None
        }
    }

    #[cfg(feature = "paymaster")]
    fn paymaster_config(&self) -> Result<Option<(PaymasterConfig, Option<PaymasterSidecarInfo>)>> {
        if !self.paymaster.is_enabled() {
            return Ok(None);
        }

        // Determine mode based on whether URL is provided
        let is_external = self.paymaster.is_external();

        // For sidecar mode, allocate a free port and prepare sidecar info
        let (url, sidecar_info) = if is_external {
            // External mode: use the provided URL
            let url = self.paymaster.url.clone().expect("URL must be set in external mode");
            (url, None)
        } else {
            // Sidecar mode: allocate a free port
            let listener = std::net::TcpListener::bind("127.0.0.1:0")
                .context("failed to find free port for paymaster sidecar")?;
            let port = listener.local_addr()?.port();
            let url = Url::parse(&format!("http://127.0.0.1:{port}")).expect("valid url");

            // Validate and prepare API key
            let api_key = {
                let key = self
                    .paymaster
                    .api_key
                    .clone()
                    .unwrap_or_else(|| DEFAULT_PAYMASTER_API_KEY.to_string());
                if !key.starts_with("paymaster_") {
                    warn!(
                        target: LOG_TARGET,
                        %key,
                        "paymaster api key must start with 'paymaster_'; using default"
                    );
                    DEFAULT_PAYMASTER_API_KEY.to_string()
                } else {
                    key
                }
            };

            let sidecar_info = PaymasterSidecarInfo { port, api_key };
            (url, Some(sidecar_info))
        };

        let api_key = if is_external {
            self.paymaster.api_key.clone()
        } else {
            sidecar_info.as_ref().map(|s| s.api_key.clone())
        };

        let config = PaymasterConfig {
            url,
            api_key,
            prefunded_index: self.paymaster.prefunded_index,
            #[cfg(feature = "cartridge")]
            cartridge_api_url: Some(self.cartridge.api.clone()),
        };

        Ok(Some((config, sidecar_info)))
    }

    #[cfg(feature = "vrf")]
    fn vrf_config(&self) -> Result<Option<(VrfConfig, Option<VrfSidecarInfo>)>> {
        if !self.vrf.is_enabled() {
            return Ok(None);
        }

        // Determine mode based on whether URL is provided
        let is_external = self.vrf.is_external();

        let (url, sidecar_info) = if is_external {
            // External mode: use the provided URL
            let url = self.vrf.url.clone().expect("URL must be set in external mode");
            (url, None)
        } else {
            // Sidecar mode: use configured port (VRF server uses fixed port 3000)
            let port = self.vrf.port;
            let url = Url::parse(&format!("http://127.0.0.1:{port}")).expect("valid url");
            let sidecar_info = VrfSidecarInfo { port };
            (url, Some(sidecar_info))
        };

        let key_source = match self.vrf.key_source {
            VrfKeySource::Prefunded => NodeVrfKeySource::Prefunded,
            VrfKeySource::Sequencer => NodeVrfKeySource::Sequencer,
        };

        let config = VrfConfig { url, key_source, prefunded_index: self.vrf.prefunded_index };

        Ok(Some((config, sidecar_info)))
    }

    #[cfg(feature = "tee")]
    fn tee_config(&self) -> Option<TeeConfig> {
        self.tee.tee_provider.map(|provider_type| TeeConfig { provider_type })
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

        if self.data_dir.is_none() {
            self.data_dir = config.data_dir;
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

        #[cfg(all(feature = "server", feature = "grpc"))]
        {
            self.grpc.merge(config.grpc.as_ref());
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

        #[cfg(feature = "paymaster")]
        {
            self.paymaster.merge(config.paymaster.as_ref());
        }

        #[cfg(feature = "vrf")]
        {
            self.vrf.merge(config.vrf.as_ref());
        }

        #[cfg(feature = "explorer")]
        {
            if !self.explorer.explorer {
                if let Some(explorer) = &config.explorer {
                    self.explorer.explorer = explorer.explorer;
                }
            }
        }

        Ok(self)
    }

    fn tracer_config(&self) -> Option<katana_tracing::TracerConfig> {
        self.tracer.config()
    }
}

/// Bootstrap contracts and start sidecar processes if needed.
///
/// This function is called after the node is launched to:
/// 1. Bootstrap necessary contracts (forwarder, VRF accounts)
/// 2. Start sidecar processes in sidecar mode
#[cfg(feature = "paymaster")]
async fn bootstrap_and_start_sidecars<P>(
    args: &SequencerNodeArgs,
    node: &katana_node::Node<P>,
    rpc_addr: &std::net::SocketAddr,
    paymaster_sidecar: Option<&PaymasterSidecarInfo>,
    #[cfg(feature = "vrf")] vrf_sidecar: Option<&VrfSidecarInfo>,
) -> Result<Option<crate::sidecar::SidecarProcesses>>
where
    P: katana_provider::ProviderFactory + Clone,
    <P as katana_provider::ProviderFactory>::Provider:
        katana_core::backend::storage::ProviderRO,
    <P as katana_provider::ProviderFactory>::ProviderMut:
        katana_core::backend::storage::ProviderRW,
{
    #[cfg(feature = "vrf")]
    use crate::sidecar::{VrfBootstrapConfig, VrfKeySource, VrfSidecarConfig};
    use crate::sidecar::{
        bootstrap_sidecars, start_sidecars, BootstrapConfig, PaymasterSidecarConfig,
        SidecarStartConfig,
    };

    // If no sidecars need to be started, return None
    #[cfg(feature = "vrf")]
    let has_sidecars = paymaster_sidecar.is_some() || vrf_sidecar.is_some();
    #[cfg(not(feature = "vrf"))]
    let has_sidecars = paymaster_sidecar.is_some();

    if !has_sidecars {
        return Ok(None);
    }

    // Build bootstrap config
    let bootstrap_config = BootstrapConfig {
        paymaster_prefunded_index: paymaster_sidecar.map(|_| args.paymaster.prefunded_index),
        #[cfg(feature = "vrf")]
        vrf: vrf_sidecar.map(|_| {
            let key_source = match args.vrf.key_source {
                crate::options::VrfKeySource::Prefunded => VrfKeySource::Prefunded,
                crate::options::VrfKeySource::Sequencer => VrfKeySource::Sequencer,
            };
            VrfBootstrapConfig {
                key_source,
                prefunded_index: args.vrf.prefunded_index,
                sequencer_address: node.config().chain.genesis().sequencer_address,
            }
        }),
        fee_enabled: node.config().dev.fee,
    };

    // Bootstrap contracts
    let bootstrap = bootstrap_sidecars(
        &bootstrap_config,
        node.backend(),
        node.block_producer(),
        node.pool(),
    )
    .await?;

    // Build sidecar start config
    let paymaster_config = paymaster_sidecar.map(|info| PaymasterSidecarConfig {
        options: &args.paymaster,
        port: info.port,
        api_key: info.api_key.clone(),
    });

    #[cfg(feature = "vrf")]
    let vrf_config =
        vrf_sidecar.map(|info| VrfSidecarConfig { options: &args.vrf, port: info.port });

    let start_config = SidecarStartConfig {
        paymaster: paymaster_config,
        #[cfg(feature = "vrf")]
        vrf: vrf_config,
    };

    // Start sidecar processes
    let processes = start_sidecars(&start_config, &bootstrap, rpc_addr).await?;
    Ok(Some(processes))
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
    #[cfg(feature = "server")]
    use katana_node::config::rpc::RpcModuleKind;
    use katana_primitives::chain::ChainId;
    use katana_primitives::{address, felt, Felt};

    use super::*;

    #[test]
    fn test_starknet_config_default() {
        let args = SequencerNodeArgs::parse_from(["katana"]);
        let result = args.config().unwrap();
        let config = &result.config;

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
        let args = SequencerNodeArgs::parse_from([
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
            "--data-dir",
            "/path/to/db",
        ]);
        let result = args.config().unwrap();
        let config = &result.config;

        assert!(!config.dev.fee);
        assert!(!config.dev.account_validation);
        assert_eq!(config.execution.invocation_max_steps, 200);
        assert_eq!(config.execution.validation_max_steps, 100);
        assert_eq!(config.db.dir, Some(PathBuf::from("/path/to/db")));
        assert_eq!(config.chain.id(), ChainId::GOERLI);
        assert_eq!(config.chain.genesis().sequencer_address, *DEFAULT_SEQUENCER_ADDRESS);
    }

    #[test]
    fn test_db_dir_alias() {
        // --db-dir should work as an alias for --data-dir
        let args = SequencerNodeArgs::parse_from(["katana", "--db-dir", "/path/to/db"]);
        let config = args.config().unwrap();
        assert_eq!(config.db.dir, Some(PathBuf::from("/path/to/db")));
    }

    #[test]
    fn custom_fixed_gas_prices() {
        let result = SequencerNodeArgs::parse_from(["katana"]).config().unwrap();
        assert!(result.config.dev.fixed_gas_prices.is_none());

        let result = SequencerNodeArgs::parse_from(["katana", "--gpo.l1-eth-gas-price", "10"])
            .config()
            .unwrap();
        assert_matches!(result.config.dev.fixed_gas_prices, Some(prices) => {
            assert_eq!(prices.l1_gas_prices.eth.get(), 10);
            assert_eq!(prices.l1_gas_prices.strk, DEFAULT_ETH_L2_GAS_PRICE);
            assert_eq!(prices.l1_data_gas_prices.eth, DEFAULT_ETH_L1_DATA_GAS_PRICE);
            assert_eq!(prices.l1_data_gas_prices.strk, DEFAULT_STRK_L1_DATA_GAS_PRICE);
        });

        let result = SequencerNodeArgs::parse_from(["katana", "--gpo.l1-strk-gas-price", "20"])
            .config()
            .unwrap();
        assert_matches!(result.config.dev.fixed_gas_prices, Some(prices) => {
            assert_eq!(prices.l1_gas_prices.eth, DEFAULT_ETH_L1_GAS_PRICE);
            assert_eq!(prices.l1_gas_prices.strk.get(), 20);
            assert_eq!(prices.l1_data_gas_prices.eth, DEFAULT_ETH_L1_DATA_GAS_PRICE);
            assert_eq!(prices.l1_data_gas_prices.strk, DEFAULT_STRK_L1_DATA_GAS_PRICE);
        });

        let result = SequencerNodeArgs::parse_from(["katana", "--gpo.l1-eth-data-gas-price", "2"])
            .config()
            .unwrap();
        assert_matches!(result.config.dev.fixed_gas_prices, Some(prices) => {
            assert_eq!(prices.l1_gas_prices.eth, DEFAULT_ETH_L1_GAS_PRICE);
            assert_eq!(prices.l1_gas_prices.strk, DEFAULT_STRK_L1_GAS_PRICE);
            assert_eq!(prices.l1_data_gas_prices.eth.get(), 2);
            assert_eq!(prices.l1_data_gas_prices.strk, DEFAULT_STRK_L1_DATA_GAS_PRICE);
        });

        let result = SequencerNodeArgs::parse_from(["katana", "--gpo.l1-strk-data-gas-price", "2"])
            .config()
            .unwrap();
        assert_matches!(result.config.dev.fixed_gas_prices, Some(prices) => {
            assert_eq!(prices.l1_gas_prices.eth, DEFAULT_ETH_L1_GAS_PRICE);
            assert_eq!(prices.l1_gas_prices.strk, DEFAULT_STRK_L1_GAS_PRICE);
            assert_eq!(prices.l1_data_gas_prices.eth, DEFAULT_ETH_L1_DATA_GAS_PRICE);
            assert_eq!(prices.l1_data_gas_prices.strk.get(), 2);
        });

        let result = SequencerNodeArgs::parse_from([
            "katana",
            "--gpo.l1-eth-gas-price",
            "10",
            "--gpo.l1-strk-data-gas-price",
            "2",
        ])
        .config()
        .unwrap();

        assert_matches!(result.config.dev.fixed_gas_prices, Some(prices) => {
            assert_eq!(prices.l1_gas_prices.eth.get(), 10);
            assert_eq!(prices.l1_gas_prices.strk, DEFAULT_STRK_L1_GAS_PRICE);
            assert_eq!(prices.l1_data_gas_prices.eth, DEFAULT_ETH_L1_DATA_GAS_PRICE);
            assert_eq!(prices.l1_data_gas_prices.strk.get(), 2);
        });

        // Set all the gas prices options

        let result = SequencerNodeArgs::parse_from([
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

        assert_matches!(result.config.dev.fixed_gas_prices, Some(prices) => {
            assert_eq!(prices.l1_gas_prices.eth.get(), 10);
            assert_eq!(prices.l1_gas_prices.strk.get(), 20);
            assert_eq!(prices.l1_data_gas_prices.eth.get(), 1);
            assert_eq!(prices.l1_data_gas_prices.strk.get(), 2);
        })
    }

    #[test]
    fn genesis_with_fixed_gas_prices() {
        let result = SequencerNodeArgs::parse_from([
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
        let config = &result.config;

        assert_eq!(config.chain.genesis().number, 0);
        assert_eq!(config.chain.genesis().parent_hash, felt!("0x999"));
        assert_eq!(config.chain.genesis().timestamp, 5123512314);
        assert_eq!(config.chain.genesis().state_root, felt!("0x99"));
        assert_eq!(config.chain.genesis().sequencer_address, address!("0x100"));
        assert_eq!(config.chain.genesis().gas_prices.eth.get(), 9999);
        assert_eq!(config.chain.genesis().gas_prices.strk.get(), 8888);
        assert_matches!(&config.dev.fixed_gas_prices, Some(prices) => {
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

[explorer]
explorer = true
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

        let result = SequencerNodeArgs::parse_from(args.clone())
            .with_config_file()
            .unwrap()
            .config()
            .unwrap();
        let config = &result.config;

        assert_eq!(config.execution.validation_max_steps, 1234);
        assert_eq!(config.execution.invocation_max_steps, 9988);
        assert!(!config.dev.fee);
        assert_matches!(&config.dev.fixed_gas_prices, Some(prices) => {
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

        #[cfg(feature = "explorer")]
        assert!(config.rpc.explorer);
    }

    #[test]
    #[cfg(feature = "server")]
    fn parse_cors_origins() {
        use katana_rpc_server::cors::HeaderValue;

        let result = SequencerNodeArgs::parse_from([
            "katana",
            "--http.cors_origins",
            "*,http://localhost:3000,https://example.com",
        ])
        .config()
        .unwrap();

        let cors_origins = &result.config.rpc.cors_origins;

        assert_eq!(cors_origins.len(), 3);
        assert!(cors_origins.contains(&HeaderValue::from_static("*")));
        assert!(cors_origins.contains(&HeaderValue::from_static("http://localhost:3000")));
        assert!(cors_origins.contains(&HeaderValue::from_static("https://example.com")));
    }

    #[cfg(feature = "server")]
    #[test]
    fn http_modules() {
        // If the `--http.api` isn't specified, only starknet module will be exposed.
        let result = SequencerNodeArgs::parse_from(["katana"]).config().unwrap();
        let modules = &result.config.rpc.apis;
        assert_eq!(modules.len(), 1);
        assert!(modules.contains(&RpcModuleKind::Starknet));

        // If the `--http.api` is specified, only the ones in the list will be exposed.
        let result =
            SequencerNodeArgs::parse_from(["katana", "--http.api", "starknet"]).config().unwrap();
        let modules = &result.config.rpc.apis;
        assert_eq!(modules.len(), 1);
        assert!(modules.contains(&RpcModuleKind::Starknet));

        // Specifiying the dev module without enabling dev mode is forbidden.
        let err = SequencerNodeArgs::parse_from(["katana", "--http.api", "starknet,dev"])
            .config()
            .unwrap_err();
        assert!(err
            .to_string()
            .contains("The `dev` module can only be enabled in dev mode (ie `--dev` flag)"));
    }

    #[cfg(feature = "server")]
    #[test]
    fn test_dev_api_enabled() {
        let args = SequencerNodeArgs::parse_from(["katana", "--dev"]);
        let result = args.config().unwrap();

        assert!(result.config.rpc.apis.contains(&RpcModuleKind::Dev));
    }

    #[cfg(all(feature = "cartridge", feature = "paymaster"))]
    #[test]
    fn cartridge_paymaster() {
        // Test with --paymaster flag (sidecar mode)
        let args = SequencerNodeArgs::parse_from(["katana", "--paymaster"]);
        let result = args.config().unwrap();

        // Verify cartridge module is automatically enabled when paymaster is enabled
        assert!(result.config.rpc.apis.contains(&RpcModuleKind::Cartridge));

        // Test with paymaster explicitly specified in RPC modules
        let args =
            SequencerNodeArgs::parse_from(["katana", "--paymaster", "--http.api", "starknet"]);
        let result = args.config().unwrap();

        // Verify cartridge module is still enabled even when not in explicit RPC list
        assert!(result.config.rpc.apis.contains(&RpcModuleKind::Cartridge));
        assert!(result.config.rpc.apis.contains(&RpcModuleKind::Starknet));

        // Test with --paymaster.url (external mode - also enables paymaster)
        let args =
            SequencerNodeArgs::parse_from(["katana", "--paymaster.url", "http://localhost:8080"]);
        let result = args.config().unwrap();
        assert!(result.config.rpc.apis.contains(&RpcModuleKind::Cartridge));

        // Test without paymaster enabled
        let args = SequencerNodeArgs::parse_from(["katana"]);
        let result = args.config().unwrap();

        // Verify cartridge module is not enabled by default
        assert!(!result.config.rpc.apis.contains(&RpcModuleKind::Cartridge));
    }

    #[cfg(feature = "cartridge")]
    #[test]
    fn cartridge_controllers() {
        use katana_slot_controller::{
            ControllerLatest, ControllerV104, ControllerV105, ControllerV106, ControllerV107,
            ControllerV108, ControllerV109,
        };

        // Test with controllers enabled
        let args = SequencerNodeArgs::parse_from(["katana", "--cartridge.controllers"]);
        let result = args.config().unwrap();
        let config = &result.config;

        // Verify that all the Controller classes are added to the genesis
        assert!(config.chain.genesis().classes.contains_key(&ControllerV104::HASH));
        assert!(config.chain.genesis().classes.contains_key(&ControllerV105::HASH));
        assert!(config.chain.genesis().classes.contains_key(&ControllerV106::HASH));
        assert!(config.chain.genesis().classes.contains_key(&ControllerV107::HASH));
        assert!(config.chain.genesis().classes.contains_key(&ControllerV108::HASH));
        assert!(config.chain.genesis().classes.contains_key(&ControllerV109::HASH));
        assert!(config.chain.genesis().classes.contains_key(&ControllerLatest::HASH));

        // Test without controllers enabled
        let args = SequencerNodeArgs::parse_from(["katana"]);
        let result = args.config().unwrap();
        let config = &result.config;

        assert!(!config.chain.genesis().classes.contains_key(&ControllerV104::HASH));
        assert!(!config.chain.genesis().classes.contains_key(&ControllerV105::HASH));
        assert!(!config.chain.genesis().classes.contains_key(&ControllerV106::HASH));
        assert!(!config.chain.genesis().classes.contains_key(&ControllerV107::HASH));
        assert!(!config.chain.genesis().classes.contains_key(&ControllerV108::HASH));
        assert!(!config.chain.genesis().classes.contains_key(&ControllerV109::HASH));
        assert!(!config.chain.genesis().classes.contains_key(&ControllerLatest::HASH));
    }

    #[cfg(feature = "cartridge")]
    #[test]
    fn vrf_mode_adds_classes() {
        use katana_primitives::utils::class::parse_sierra_class;

        let args = SequencerNodeArgs::parse_from(["katana", "--paymaster", "--vrf"]);
        let result = args.config().unwrap();
        let config = &result.config;

        let vrf_account_class = parse_sierra_class(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../controller/classes/cartridge_vrf_VrfAccount.contract_class.json"
        )))
        .unwrap();
        let vrf_account_hash = vrf_account_class.class_hash().unwrap();

        let vrf_consumer_class = parse_sierra_class(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../controller/classes/cartridge_vrf_VrfConsumer.contract_class.json"
        )))
        .unwrap();
        let vrf_consumer_hash = vrf_consumer_class.class_hash().unwrap();

        assert!(config.chain.genesis().classes.get(&vrf_account_hash).is_some());
        assert!(config.chain.genesis().classes.get(&vrf_consumer_hash).is_some());
    }

    #[cfg(feature = "paymaster")]
    #[test]
    fn paymaster_adds_forwarder_class() {
        use katana_primitives::utils::class::parse_sierra_class;

        let args = SequencerNodeArgs::parse_from(["katana", "--paymaster"]);
        let result = args.config().unwrap();
        let config = &result.config;

        let forwarder_class = parse_sierra_class(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../controller/classes/avnu_Forwarder.contract_class.json"
        )))
        .unwrap();
        let forwarder_hash = forwarder_class.class_hash().unwrap();

        assert!(config.chain.genesis().classes.get(&forwarder_hash).is_some());
    }

    #[cfg(feature = "vrf")]
    #[test]
    fn vrf_requires_paymaster() {
        let args = SequencerNodeArgs::parse_from(["katana", "--vrf"]);
        let err = args.config().unwrap_err();
        assert!(err.to_string().contains("--vrf requires paymaster"));
    }
}
