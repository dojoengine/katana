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
use katana_genesis::constant::{
    DEFAULT_FROZEN_DEV_ACCOUNT_ADDRESS_CLASS_HASH, DEFAULT_PREFUNDED_ACCOUNT_BALANCE,
};
use katana_messaging::MessagingConfig;
use katana_sequencer_node::config::db::DbConfig;
use katana_sequencer_node::config::dev::{DevConfig, FixedL1GasPriceConfig};
use katana_sequencer_node::config::execution::ExecutionConfig;
use katana_sequencer_node::config::fork::ForkingConfig;
use katana_sequencer_node::config::gateway::GatewayConfig;
#[cfg(all(feature = "server", feature = "grpc"))]
use katana_sequencer_node::config::grpc::GrpcConfig;
use katana_sequencer_node::config::metrics::MetricsConfig;
use katana_sequencer_node::config::paymaster::{PaymasterConfig, VrfConfig};
use katana_sequencer_node::config::rpc::RpcConfig;
#[cfg(feature = "server")]
use katana_sequencer_node::config::rpc::{RpcModuleKind, RpcModulesList};
use katana_sequencer_node::config::sequencing::SequencingConfig;
use katana_sequencer_node::config::tee::TeeConfig;
use katana_sequencer_node::config::Config;
use katana_sequencer_node::Node;
use serde::{Deserialize, Serialize};
use tracing::info;
use url::Url;

use crate::file::NodeArgsConfig;
use crate::options::*;
use crate::utils::{self, parse_chain_config_dir, parse_seed, prompt_db_migration};

pub(crate) const LOG_TARGET: &str = "katana::cli";

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
    pub db: DbOptions,

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

    #[command(flatten)]
    pub paymaster: PaymasterOptions,

    #[command(flatten)]
    pub cartridge: CartridgeOptions,

    #[command(flatten)]
    pub tee: TeeOptions,

    #[cfg(all(feature = "server", feature = "grpc"))]
    #[command(flatten)]
    pub grpc: GrpcOptions,
}

impl SequencerNodeArgs {
    pub async fn execute(&self, build_info: crate::BuildInfo) -> Result<()> {
        let logging = katana_tracing::LoggingConfig {
            stdout_format: self.logging.stdout.stdout_format,
            stdout_color: self.logging.stdout.color,
            file_enabled: self.logging.file.enabled,
            file_format: self.logging.file.file_format,
            file_directory: self.logging.file.directory.clone(),
            file_max_files: self.logging.file.max_files,
        };

        katana_tracing::init(logging, self.tracer_config()).await?;

        self.start_node(build_info).await
    }

    async fn start_node(&self, build_info: crate::BuildInfo) -> Result<()> {
        // Build the node configuration
        let config = self.config(build_info)?;

        // Resolve sidecar binaries before resources are committed.
        let paymaster_bin = if self.paymaster.enabled && !self.paymaster.is_external() {
            use anyhow::anyhow;

            use crate::sidecar::{resolve_sidecar_binary, SidecarKind};

            let sidecar = SidecarKind::Paymaster;
            let bin_path = self.paymaster.bin.as_deref();

            let resolved_path =
                resolve_sidecar_binary(sidecar, bin_path).await?.ok_or_else(|| {
                    anyhow!("Paymaster service binary {} not installed", sidecar.binary_filename())
                })?;

            Some(resolved_path)
        } else {
            None
        };

        let vrf_bin = if self.cartridge.vrf.enabled && !self.cartridge.vrf.is_external() {
            use anyhow::anyhow;

            use crate::sidecar::{resolve_sidecar_binary, SidecarKind};

            let sidecar = SidecarKind::Vrf;
            let bin_path = self.cartridge.vrf.bin.as_deref();

            let resolved_path =
                resolve_sidecar_binary(SidecarKind::Vrf, bin_path).await?.ok_or_else(|| {
                    anyhow!("VRF service binary {} not installed", sidecar.binary_filename())
                })?;

            Some(resolved_path)
        } else {
            None
        };

        if config.forking.is_some() {
            // Pass config by value: build_forked needs exclusive Arc access to mutate chain_spec.
            // Cloning would create a second Arc reference and cause Arc::get_mut to panic.
            let node = Node::build_forked(config).await.context("failed to build forked node")?;

            if !self.silent {
                utils::print_intro(self, &node.backend().chain_spec);
            }

            let handle = node.launch().await.context("failed to launch forked node")?;

            let mut paymaster = if let Some(bin_path) = paymaster_bin {
                use crate::sidecar::bootstrap_paymaster;

                let paymaster = bootstrap_paymaster(
                    bin_path,
                    handle.node().config().paymaster.as_ref().unwrap().url.clone(),
                    *handle.rpc().addr(),
                    &handle.node().config().chain,
                )
                .await?
                .start()
                .await?;

                Some(paymaster)
            } else {
                None
            };

            let mut vrf = if let Some(bin_path) = vrf_bin {
                use crate::sidecar::bootstrap_vrf;

                let paymaster_cfg = handle.node().config().paymaster.as_ref().unwrap();
                let cartridge_api_cfg = paymaster_cfg.cartridge_api.as_ref().unwrap();
                let vrf_url = cartridge_api_cfg.vrf.as_ref().unwrap().url.clone();

                let vrf = bootstrap_vrf(
                    bin_path,
                    vrf_url,
                    *handle.rpc().addr(),
                    &handle.node().config().chain,
                )
                .await?
                .start()
                .await?;

                Some(vrf)
            } else {
                None
            };

            // Wait until an OS signal (ie SIGINT, SIGTERM) is received or the node is shutdown.
            tokio::select! {
                _ = katana_utils::wait_shutdown_signals() => {
                    // Gracefully shutdown the node before exiting
                    handle.stop().await?;
                },

                _ = handle.stopped() => { }
            }

            if let Some(ref mut s) = paymaster {
                s.shutdown().await?;
            }

            if let Some(ref mut s) = vrf {
                s.shutdown().await?;
            }
        } else {
            let node = Node::build(config.clone()).context("failed to build node")?;

            if !self.silent {
                utils::print_intro(self, &node.backend().chain_spec);
            }

            let handle = node.launch().await.context("failed to launch node")?;

            let mut paymaster = if let Some(bin_path) = paymaster_bin {
                use crate::sidecar;

                let paymaster_service = sidecar::bootstrap_paymaster(
                    bin_path,
                    config.paymaster.as_ref().unwrap().url.clone(),
                    *handle.rpc().addr(),
                    &handle.node().config().chain,
                )
                .await?;

                // When VRF is enabled, whitelist the VRF account on the forwarder so that
                // VRF transactions can be routed through it.

                if let Some(vrf_account) = config
                    .paymaster
                    .as_ref()
                    .and_then(|p| p.cartridge_api.as_ref())
                    .and_then(|c| c.vrf.as_ref())
                    .map(|v| v.vrf_account)
                {
                    paymaster_service.whitelist_address(vrf_account).await?;
                }

                Some(paymaster_service.start().await?)
            } else {
                None
            };

            let mut vrf = if let Some(bin_path) = vrf_bin {
                use crate::sidecar::bootstrap_vrf;

                let paymaster_cfg = handle.node().config().paymaster.as_ref().unwrap();
                let cartridge_api_cfg = paymaster_cfg.cartridge_api.as_ref().unwrap();
                let vrf_url = cartridge_api_cfg.vrf.as_ref().unwrap().url.clone();

                let vrf = bootstrap_vrf(
                    bin_path,
                    vrf_url,
                    *handle.rpc().addr(),
                    &handle.node().config().chain,
                )
                .await?
                .start()
                .await?;

                Some(vrf)
            } else {
                None
            };

            // Wait until an OS signal (ie SIGINT, SIGTERM) is received or the node is shutdown.
            tokio::select! {
                _ = katana_utils::wait_shutdown_signals() => {
                    // Gracefully shutdown the node before exiting
                    handle.stop().await?;
                },

                _ = handle.stopped() => { }
            }

            if let Some(ref mut s) = paymaster {
                s.shutdown().await?;
            }

            if let Some(ref mut s) = vrf {
                s.shutdown().await?;
            }
        }

        info!("Shutting down.");

        Ok(())
    }

    pub fn config(&self, build_info: crate::BuildInfo) -> Result<Config> {
        let db = self.db_config()?;
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

        let paymaster = self.paymaster_config(&chain)?;

        // the `katana init` will automatically generate a messaging config. so if katana is run
        // with `--chain` then the `--messaging` flag is not required. this is temporary and
        // the messagign config will eventually be removed slowly.
        let messaging = if cs_messaging.is_some() { cs_messaging } else { self.messaging.clone() };

        Ok(Config {
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
            build_info,
            paymaster,
            tee: self.tee_config(),
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
            if self.cartridge.paymaster {
                modules.add(RpcModuleKind::Cartridge);
            }

            // The TEE rpc must be enabled if a TEE provider is specified.
            // We put it here so that even when the individual api are explicitly specified
            // (ie `--rpc.api`) we guarantee that the tee rpc is enabled.
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
                .with_frozen_address_class_hash(DEFAULT_FROZEN_DEV_ACCOUNT_ADDRESS_CLASS_HASH)
                .with_seed(parse_seed(&self.development.seed))
                .with_balance(U256::from(DEFAULT_PREFUNDED_ACCOUNT_BALANCE))
                .generate();

            chain_spec.genesis.extend_allocations(accounts.into_iter().map(|(k, v)| (k, v.into())));

            if self.cartridge.controllers {
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
            let cfg = ForkingConfig {
                url: url.clone(),
                block: self.forking.fork_block,
                init_dev_genesis: !self.forking.no_dev_genesis,
            };
            return Ok(Some(cfg));
        }

        Ok(None)
    }

    fn db_config(&self) -> Result<DbConfig> {
        let mut migrate = self.db.migrate;

        if !migrate {
            if let Some(ref path) = self.db.dir {
                if path.exists() {
                    migrate = prompt_db_migration(path)?;
                }
            }
        }

        Ok(DbConfig { dir: self.db.dir.clone(), migrate })
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
        if self.gateway.enable {
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

    fn paymaster_config(
        &self,
        chain_spec: &Arc<katana_chain_spec::ChainSpec>,
    ) -> Result<Option<PaymasterConfig>> {
        if !self.paymaster.enabled {
            return Ok(None);
        }

        use crate::sidecar::DEFAULT_PAYMASTER_API_KEY;

        let mut config = if self.paymaster.is_external() {
            let url = self.paymaster.url.clone().expect("URL must be set in external mode");
            let api_key = self.paymaster.api_key.clone();
            PaymasterConfig { url, api_key, cartridge_api: None }
        } else {
            // find free port
            let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
            let url = Url::parse(&format!("http://{}", listener.local_addr()?))?;

            let api_key = self
                .paymaster
                .api_key
                .clone()
                .unwrap_or_else(|| DEFAULT_PAYMASTER_API_KEY.to_string());

            if !api_key.starts_with("paymaster_") {
                anyhow::bail!(
                    "invalid api key {api_key}; paymaster api key must start with `paymaster_`"
                );
            }

            PaymasterConfig { url, api_key: Some(api_key), cartridge_api: None }
        };

        if self.cartridge.paymaster {
            let vrf = self.vrf_config(chain_spec)?;

            use anyhow::anyhow;
            use katana_genesis::allocation::GenesisAccountAlloc;
            use katana_sequencer_node::config::paymaster::CartridgeApiConfig;

            // Derive paymaster credentials from genesis account 0
            let (address, private_key) = {
                let (address, allocation) = chain_spec
                    .genesis()
                    .accounts()
                    .next()
                    .ok_or_else(|| anyhow!("no genesis accounts available for paymaster"))?;

                let private_key = match allocation {
                    GenesisAccountAlloc::DevAccount(account) => account.private_key,
                    _ => return Err(anyhow!("paymaster account {address} has no private key")),
                };

                (*address, private_key)
            };

            config.cartridge_api = Some(CartridgeApiConfig {
                vrf,
                controller_deployer_address: address,
                controller_deployer_private_key: private_key,
                cartridge_api_url: self.cartridge.cartridge_api.clone(),
            });
        }

        Ok(Some(config))
    }

    fn vrf_config(&self, _chain: &ChainSpec) -> Result<Option<VrfConfig>> {
        let options = &self.cartridge.vrf;

        if !options.enabled {
            return Ok(None);
        }

        if options.is_external() {
            let url = options.url.clone().expect("must be set if external");
            let vrf_account = options.vrf_account_contract.expect("must be set if external");

            Ok(Some(VrfConfig { url, vrf_account }))
        } else {
            use cartridge::get_default_vrf_account;

            let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
            let addr = listener.local_addr()?;
            let url = Url::parse(&format!("http://{addr}"))?;

            let vrf_account_info = get_default_vrf_account()?;
            let vrf_account_address = vrf_account_info.account_address;

            Ok(Some(VrfConfig { url, vrf_account: vrf_account_address }))
        }
    }

    fn tee_config(&self) -> Option<TeeConfig> {
        self.tee
            .tee_provider
            .map(|provider_type| TeeConfig { provider_type, fork_block_number: None })
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

        self.db.merge(config.db.as_ref());

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

        {
            self.cartridge.merge(config.cartridge.as_ref());
        }

        {
            self.paymaster.merge(config.paymaster.as_ref());
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

#[cfg(test)]
mod test {
    use std::str::FromStr;

    use assert_matches::assert_matches;
    use katana_gas_price_oracle::{
        DEFAULT_ETH_L1_DATA_GAS_PRICE, DEFAULT_ETH_L1_GAS_PRICE, DEFAULT_ETH_L2_GAS_PRICE,
        DEFAULT_STRK_L1_DATA_GAS_PRICE, DEFAULT_STRK_L1_GAS_PRICE,
    };
    use katana_primitives::chain::ChainId;
    use katana_primitives::{address, felt, Felt};
    use katana_sequencer_node::config::execution::{
        DEFAULT_INVOCATION_MAX_STEPS, DEFAULT_VALIDATION_MAX_STEPS,
    };
    #[cfg(feature = "server")]
    use katana_sequencer_node::config::rpc::RpcModuleKind;

    use super::*;

    #[test]
    fn test_starknet_config_default() {
        let args = SequencerNodeArgs::parse_from(["katana"]);
        let result = args.config(crate::BuildInfo::default()).unwrap();
        let config = &result;

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
    fn default_predeployed_account_address_is_backward_compatible() {
        let args = SequencerNodeArgs::parse_from(["katana"]);
        let result = args.config(crate::BuildInfo::default()).unwrap();

        // Keep the first user-facing predeployed account stable. This address/key pair is relied
        // on by downstream integration tests and tooling that use Katana's default prefunded
        // account.
        let (address, allocation) =
            result.chain.genesis().accounts().next().expect("must have a default dev account");

        assert_eq!(
            *address,
            address!("0x127fd5f1fe78a71f8bcd1fec63e3fe2f0486b6ecd5c86a0466c3a21fa5cfcec")
        );
        assert_eq!(
            allocation.private_key(),
            Some(felt!("0xc5b2fcab997346f3ea1c00b002ecf6f382c5f9c9659a3894eb783c5320f912"))
        );
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
        let result = args.config(crate::BuildInfo::default()).unwrap();
        let config = &result;

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
        let result = args.config(crate::BuildInfo::default()).unwrap();
        assert_eq!(result.db.dir, Some(PathBuf::from("/path/to/db")));
    }

    #[test]
    fn custom_fixed_gas_prices() {
        let result =
            SequencerNodeArgs::parse_from(["katana"]).config(crate::BuildInfo::default()).unwrap();
        assert!(result.dev.fixed_gas_prices.is_none());

        let result = SequencerNodeArgs::parse_from(["katana", "--gpo.l1-eth-gas-price", "10"])
            .config(crate::BuildInfo::default())
            .unwrap();
        assert_matches!(result.dev.fixed_gas_prices, Some(prices) => {
            assert_eq!(prices.l1_gas_prices.eth.get(), 10);
            assert_eq!(prices.l1_gas_prices.strk, DEFAULT_ETH_L2_GAS_PRICE);
            assert_eq!(prices.l1_data_gas_prices.eth, DEFAULT_ETH_L1_DATA_GAS_PRICE);
            assert_eq!(prices.l1_data_gas_prices.strk, DEFAULT_STRK_L1_DATA_GAS_PRICE);
        });

        let result = SequencerNodeArgs::parse_from(["katana", "--gpo.l1-strk-gas-price", "20"])
            .config(crate::BuildInfo::default())
            .unwrap();
        assert_matches!(result.dev.fixed_gas_prices, Some(prices) => {
            assert_eq!(prices.l1_gas_prices.eth, DEFAULT_ETH_L1_GAS_PRICE);
            assert_eq!(prices.l1_gas_prices.strk.get(), 20);
            assert_eq!(prices.l1_data_gas_prices.eth, DEFAULT_ETH_L1_DATA_GAS_PRICE);
            assert_eq!(prices.l1_data_gas_prices.strk, DEFAULT_STRK_L1_DATA_GAS_PRICE);
        });

        let result = SequencerNodeArgs::parse_from(["katana", "--gpo.l1-eth-data-gas-price", "2"])
            .config(crate::BuildInfo::default())
            .unwrap();
        assert_matches!(result.dev.fixed_gas_prices, Some(prices) => {
            assert_eq!(prices.l1_gas_prices.eth, DEFAULT_ETH_L1_GAS_PRICE);
            assert_eq!(prices.l1_gas_prices.strk, DEFAULT_STRK_L1_GAS_PRICE);
            assert_eq!(prices.l1_data_gas_prices.eth.get(), 2);
            assert_eq!(prices.l1_data_gas_prices.strk, DEFAULT_STRK_L1_DATA_GAS_PRICE);
        });

        let result = SequencerNodeArgs::parse_from(["katana", "--gpo.l1-strk-data-gas-price", "2"])
            .config(crate::BuildInfo::default())
            .unwrap();
        assert_matches!(result.dev.fixed_gas_prices, Some(prices) => {
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
        .config(crate::BuildInfo::default())
        .unwrap();

        assert_matches!(result.dev.fixed_gas_prices, Some(prices) => {
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
        .config(crate::BuildInfo::default())
        .unwrap();

        assert_matches!(result.dev.fixed_gas_prices, Some(prices) => {
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
        .config(crate::BuildInfo::default())
        .unwrap();
        let config = &result;

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
            .config(crate::BuildInfo::default())
            .unwrap();
        let config = &result;

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
        use katana_rpc_server::middleware::cors::HeaderValue;

        let result = SequencerNodeArgs::parse_from([
            "katana",
            "--http.cors_origins",
            "*,http://localhost:3000,https://example.com",
        ])
        .config(crate::BuildInfo::default())
        .unwrap();

        let cors_origins = &result.rpc.cors_origins;

        assert_eq!(cors_origins.len(), 3);
        assert!(cors_origins.contains(&HeaderValue::from_static("*")));
        assert!(cors_origins.contains(&HeaderValue::from_static("http://localhost:3000")));
        assert!(cors_origins.contains(&HeaderValue::from_static("https://example.com")));
    }

    #[cfg(feature = "server")]
    #[test]
    fn http_modules() {
        // If the `--http.api` isn't specified, `starknet` and `node` are exposed by default.
        let result =
            SequencerNodeArgs::parse_from(["katana"]).config(crate::BuildInfo::default()).unwrap();
        let modules = &result.rpc.apis;
        assert_eq!(modules.len(), 2);
        assert!(modules.contains(&RpcModuleKind::Starknet));
        assert!(modules.contains(&RpcModuleKind::Node));

        // If the `--http.api` is specified, only the ones in the list will be exposed.
        let result = SequencerNodeArgs::parse_from(["katana", "--http.api", "starknet"])
            .config(crate::BuildInfo::default())
            .unwrap();
        let modules = &result.rpc.apis;
        assert_eq!(modules.len(), 1);
        assert!(modules.contains(&RpcModuleKind::Starknet));

        // Specifiying the dev module without enabling dev mode is forbidden.
        let err = SequencerNodeArgs::parse_from(["katana", "--http.api", "starknet,dev"])
            .config(crate::BuildInfo::default())
            .unwrap_err();
        assert!(err
            .to_string()
            .contains("The `dev` module can only be enabled in dev mode (ie `--dev` flag)"));
    }

    #[cfg(feature = "server")]
    #[test]
    fn test_dev_api_enabled() {
        let args = SequencerNodeArgs::parse_from(["katana", "--dev"]);
        let result = args.config(crate::BuildInfo::default()).unwrap();

        assert!(result.rpc.apis.contains(&RpcModuleKind::Dev));
    }

    #[cfg(feature = "server")]
    #[test]
    fn test_node_api_explicit() {
        let result = SequencerNodeArgs::parse_from(["katana", "--http.api", "node"])
            .config(crate::BuildInfo::default())
            .unwrap();
        let modules = &result.rpc.apis;
        assert_eq!(modules.len(), 1);
        assert!(modules.contains(&RpcModuleKind::Node));

        let result = SequencerNodeArgs::parse_from(["katana", "--http.api", "starknet,node"])
            .config(crate::BuildInfo::default())
            .unwrap();
        let modules = &result.rpc.apis;
        assert_eq!(modules.len(), 2);
        assert!(modules.contains(&RpcModuleKind::Starknet));
        assert!(modules.contains(&RpcModuleKind::Node));
    }

    #[test]
    fn cartridge_paymaster() {
        // Test with --paymaster flag (sidecar mode)
        let args =
            SequencerNodeArgs::parse_from(["katana", "--paymaster", "--cartridge.paymaster"]);
        let result = args.config(crate::BuildInfo::default()).unwrap();

        // Verify cartridge module is automatically enabled when paymaster is enabled
        assert!(result.rpc.apis.contains(&RpcModuleKind::Cartridge));

        // Test with paymaster explicitly specified in RPC modules
        let args = SequencerNodeArgs::parse_from([
            "katana",
            "--http.api",
            "starknet",
            "--paymaster",
            "--cartridge.paymaster",
        ]);
        let result = args.config(crate::BuildInfo::default()).unwrap();

        // Verify cartridge module is still enabled even when not in explicit RPC list
        assert!(result.rpc.apis.contains(&RpcModuleKind::Cartridge));
        assert!(result.rpc.apis.contains(&RpcModuleKind::Starknet));

        // Test with --paymaster.url (external mode - also enables paymaster)
        let args = SequencerNodeArgs::parse_from([
            "katana",
            "--paymaster",
            "--paymaster.url",
            "http://localhost:8080",
            "--cartridge.paymaster",
        ]);
        let result = args.config(crate::BuildInfo::default()).unwrap();
        assert!(result.rpc.apis.contains(&RpcModuleKind::Cartridge));

        // Test without paymaster enabled
        let args = SequencerNodeArgs::parse_from(["katana", "--paymaster"]);
        let result = args.config(crate::BuildInfo::default()).unwrap();

        // Verify cartridge module is not enabled by default
        assert!(!result.rpc.apis.contains(&RpcModuleKind::Cartridge));

        // Test without paymaster enabled
        let args = SequencerNodeArgs::parse_from(["katana"]);
        let result = args.config(crate::BuildInfo::default()).unwrap();

        // Verify cartridge module is not enabled by default
        assert!(!result.rpc.apis.contains(&RpcModuleKind::Cartridge));
    }

    #[test]
    fn cartridge_controllers() {
        use katana_slot_controller::{
            ControllerLatest, ControllerV104, ControllerV105, ControllerV106, ControllerV107,
            ControllerV108, ControllerV109,
        };

        // Test with controllers enabled
        let args = SequencerNodeArgs::parse_from(["katana", "--cartridge.controllers"]);
        let result = args.config(crate::BuildInfo::default()).unwrap();
        let config = &result;

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
        let result = args.config(crate::BuildInfo::default()).unwrap();
        let config = &result;

        assert!(!config.chain.genesis().classes.contains_key(&ControllerV104::HASH));
        assert!(!config.chain.genesis().classes.contains_key(&ControllerV105::HASH));
        assert!(!config.chain.genesis().classes.contains_key(&ControllerV106::HASH));
        assert!(!config.chain.genesis().classes.contains_key(&ControllerV107::HASH));
        assert!(!config.chain.genesis().classes.contains_key(&ControllerV108::HASH));
        assert!(!config.chain.genesis().classes.contains_key(&ControllerV109::HASH));
        assert!(!config.chain.genesis().classes.contains_key(&ControllerLatest::HASH));
    }
}
