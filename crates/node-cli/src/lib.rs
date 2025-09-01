#![cfg_attr(not(test), warn(unused_crate_dependencies))]

pub mod utils;

use std::sync::Arc;

use alloy_primitives::U256;
use anyhow::{Context, Result};
use clap::Parser;
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
pub use katana_node_cli_args::options::*;
use katana_node_cli_args::utils::parse_seed;
pub use katana_node_cli_args::{NodeArgs, NodeArgsConfig};
use katana_primitives::genesis::allocation::DevAllocationsGenerator;
use katana_primitives::genesis::constant::DEFAULT_PREFUNDED_ACCOUNT_BALANCE;
use serde::{Deserialize, Serialize};
use tracing::info;

pub(crate) const LOG_TARGET: &str = "katana::cli";

#[derive(Parser, Debug, Serialize, Deserialize, Default, Clone)]
pub struct Cli {
    #[command(flatten)]
    pub args: NodeArgs,
}

impl Cli {
    pub fn parse_from<I, T>(itr: I) -> Self
    where
        I: IntoIterator<Item = T>,
        T: Into<std::ffi::OsString> + Clone,
    {
        Self { args: NodeArgs::parse_from(itr) }
    }
}

impl Cli {
    pub async fn execute(mut self) -> Result<()> {
        self.args.with_config_file()?;

        // Initialize logging with tracer
        let tracer_config = self.tracer_config();
        katana_tracing::init(self.args.logging.log_format, tracer_config).await?;

        self.start_node().await
    }

    async fn start_node(&self) -> Result<()> {
        // Build the node
        let config = self.build_config()?;
        let node = Node::build(config).await.context("failed to build node")?;

        if !self.args.silent {
            utils::print_intro(&self.args, &node.backend().chain_spec);
        }

        // Launch the node
        let handle = node.launch().await.context("failed to launch node")?;

        // Wait until an OS signal (ie SIGINT, SIGTERM) is received or the node is shutdown.
        tokio::select! {
            _ = katana_utils::wait_shutdown_signals() => {
                // Gracefully shutdown the node before exiting
                handle.stop().await?;
            },

            _ = handle.stopped() => { }
        }

        info!("Shutting down.");

        Ok(())
    }

    pub fn build_config(&self) -> Result<katana_node::config::Config> {
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
        let messaging =
            if cs_messaging.is_some() { cs_messaging } else { self.args.messaging.clone() };

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
            block_time: self.args.block_time,
            no_mining: self.args.no_mining,
            block_cairo_steps_limit: self.args.block_cairo_steps_limit,
        }
    }

    pub fn rpc_config(&self) -> Result<RpcConfig> {
        #[cfg(feature = "server")]
        {
            use std::time::Duration;

            #[allow(unused_mut)]
            let mut modules = if let Some(modules) = &self.args.server.http_modules {
                // TODO: This check should be handled in the `katana-node` level. Right now if you
                // instantiate katana programmatically, you can still add the dev module without
                // enabling dev mode.
                //
                // We only allow the `dev` module in dev mode (ie `--dev` flag)
                if !self.args.development.dev && modules.contains(&RpcModuleKind::Dev) {
                    anyhow::bail!(
                        "The `dev` module can only be enabled in dev mode (ie `--dev` flag)"
                    )
                }

                modules.clone()
            } else {
                // Expose the default modules if none is specified.
                let mut modules = RpcModulesList::default();

                // Ensures the `--dev` flag enabled the dev module.
                if self.args.development.dev {
                    modules.add(RpcModuleKind::Dev);
                }

                modules
            };

            // The cartridge rpc must be enabled if the paymaster is enabled.
            // We put it here so that even when the individual api are explicitly specified
            // (ie `--rpc.api`) we guarantee that the cartridge rpc is enabled.
            #[cfg(feature = "cartridge")]
            if self.args.cartridge.paymaster {
                modules.add(RpcModuleKind::Cartridge);
            }

            let cors_origins = self.args.server.http_cors_origins.clone();

            Ok(RpcConfig {
                apis: modules,
                port: self.args.server.http_port,
                addr: self.args.server.http_addr,
                max_connections: self.args.server.max_connections,
                max_concurrent_estimate_fee_requests: None,
                max_request_body_size: None,
                max_response_body_size: None,
                timeout: self.args.server.timeout.map(Duration::from_secs),
                cors_origins,
                #[cfg(feature = "explorer")]
                explorer: self.args.explorer.explorer,
                max_event_page_size: Some(self.args.server.max_event_page_size),
                max_proof_keys: Some(self.args.server.max_proof_keys),
                max_call_gas: Some(self.args.server.max_call_gas),
            })
        }

        #[cfg(not(feature = "server"))]
        {
            Ok(RpcConfig::default())
        }
    }

    fn chain_spec(&self) -> Result<(Arc<ChainSpec>, Option<MessagingConfig>)> {
        if let Some(path) = &self.args.chain {
            let mut cs = katana_chain_spec::rollup::read(path)?;
            cs.genesis.sequencer_address = *DEFAULT_SEQUENCER_ADDRESS;
            let messaging_config = MessagingConfig::from_chain_spec(&cs);
            Ok((Arc::new(ChainSpec::Rollup(cs)), Some(messaging_config)))
        }
        // exclusively for development mode
        else {
            let mut chain_spec = katana_chain_spec::dev::DEV_UNALLOCATED.clone();

            if let Some(id) = self.args.starknet.environment.chain_id {
                chain_spec.id = id;
            }

            if let Some(genesis) = &self.args.starknet.genesis {
                chain_spec.genesis = genesis.clone();
            } else {
                chain_spec.genesis.sequencer_address = *DEFAULT_SEQUENCER_ADDRESS;
            }

            // Generate dev accounts.
            // If `cartridge` is enabled, the first account will be the paymaster.
            let accounts = DevAllocationsGenerator::new(self.args.development.total_accounts)
                .with_seed(parse_seed(&self.args.development.seed))
                .with_balance(U256::from(DEFAULT_PREFUNDED_ACCOUNT_BALANCE))
                .generate();

            chain_spec.genesis.extend_allocations(accounts.into_iter().map(|(k, v)| (k, v.into())));

            #[cfg(feature = "cartridge")]
            if self.args.cartridge.controllers || self.args.cartridge.paymaster {
                katana_slot_controller::add_controller_classes(&mut chain_spec.genesis);
                katana_slot_controller::add_vrf_provider_class(&mut chain_spec.genesis);
            }

            Ok((Arc::new(ChainSpec::Dev(chain_spec)), None))
        }
    }

    fn dev_config(&self) -> DevConfig {
        let mut fixed_gas_prices = None;

        if let Some(eth) = self.args.gpo.l2_eth_gas_price {
            let prices = fixed_gas_prices.get_or_insert(FixedL1GasPriceConfig::default());
            prices.l2_gas_prices.eth = eth;
        }

        if let Some(strk) = self.args.gpo.l2_strk_gas_price {
            let prices = fixed_gas_prices.get_or_insert(FixedL1GasPriceConfig::default());
            prices.l2_gas_prices.strk = strk;
        }

        if let Some(eth) = self.args.gpo.l1_eth_gas_price {
            let prices = fixed_gas_prices.get_or_insert(FixedL1GasPriceConfig::default());
            prices.l1_gas_prices.eth = eth;
        }

        if let Some(strk) = self.args.gpo.l1_strk_gas_price {
            let prices = fixed_gas_prices.get_or_insert(FixedL1GasPriceConfig::default());
            prices.l1_gas_prices.strk = strk;
        }

        if let Some(eth) = self.args.gpo.l1_eth_data_gas_price {
            let prices = fixed_gas_prices.get_or_insert(FixedL1GasPriceConfig::default());
            prices.l1_data_gas_prices.eth = eth;
        }

        if let Some(strk) = self.args.gpo.l1_strk_data_gas_price {
            let prices = fixed_gas_prices.get_or_insert(FixedL1GasPriceConfig::default());
            prices.l1_data_gas_prices.strk = strk;
        }

        DevConfig {
            fixed_gas_prices,
            fee: !self.args.development.no_fee,
            account_validation: !self.args.development.no_account_validation,
        }
    }

    fn execution_config(&self) -> ExecutionConfig {
        ExecutionConfig {
            invocation_max_steps: self.args.starknet.environment.invoke_max_steps,
            validation_max_steps: self.args.starknet.environment.validate_max_steps,
            #[cfg(feature = "native")]
            compile_native: self.args.starknet.environment.compile_native,
            ..Default::default()
        }
    }

    fn forking_config(&self) -> Result<Option<ForkingConfig>> {
        if let Some(ref url) = self.args.forking.fork_provider {
            let cfg = ForkingConfig { url: url.clone(), block: self.args.forking.fork_block };
            return Ok(Some(cfg));
        }

        Ok(None)
    }

    fn db_config(&self) -> DbConfig {
        DbConfig { dir: self.args.db_dir.clone() }
    }

    fn metrics_config(&self) -> Option<MetricsConfig> {
        #[cfg(feature = "server")]
        if self.args.metrics.metrics {
            Some(MetricsConfig {
                addr: self.args.metrics.metrics_addr,
                port: self.args.metrics.metrics_port,
            })
        } else {
            None
        }

        #[cfg(not(feature = "server"))]
        None
    }

    #[cfg(feature = "cartridge")]
    fn cartridge_config(&self) -> Option<PaymasterConfig> {
        if self.args.cartridge.paymaster {
            Some(PaymasterConfig { cartridge_api_url: self.args.cartridge.api.clone() })
        } else {
            None
        }
    }

    fn tracer_config(&self) -> Option<katana_tracing::TracerConfig> {
        self.args.tracer.config()
    }
}
