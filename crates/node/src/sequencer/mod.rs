pub mod forked;

use std::future::IntoFuture;
use std::sync::Arc;

use crate::config::rpc::RpcModuleKind;
use anyhow::{bail, Context, Result};
use http::header::CONTENT_TYPE;
use http::Method;
use jsonrpsee::RpcModule;
use katana_chain_spec::{ChainSpec, ChainSpecT, SettlementLayer};
use katana_core::backend::storage::{ProviderRO, ProviderRW};
use katana_core::backend::{Backend, GenesisInitializer};
use katana_core::env::BlockContextGenerator;
use katana_core::service::block_producer::BlockProducer;
use katana_executor::implementation::blockifier::cache::ClassCache;
use katana_executor::implementation::blockifier::BlockifierFactory;
use katana_executor::{ExecutionFlags, ExecutorFactory};
use katana_gas_price_oracle::{FixedPriceOracle, GasPriceOracle};
use katana_gateway_server::{GatewayServer, GatewayServerHandle};
use katana_metrics::exporters::prometheus::{Prometheus, PrometheusRecorder};
use katana_metrics::sys::DiskReporter;
use katana_metrics::{MetricsServer, MetricsServerHandle, Report};
use katana_pool::ordering::FiFo;
use katana_pool::TxPool;
use katana_primitives::block::{BlockHashOrNumber, GasPrices};
use katana_primitives::cairo::ShortString;
use katana_primitives::env::VersionedConstantsOverrides;
use katana_provider::{DbProviderFactory, ForkProviderFactory, ProviderFactory};
#[cfg(feature = "cartridge")]
use katana_rpc_api::cartridge::CartridgeApiServer;
use katana_rpc_api::dev::DevApiServer;
use katana_rpc_api::starknet::{StarknetApiServer, StarknetTraceApiServer, StarknetWriteApiServer};
#[cfg(feature = "explorer")]
use katana_rpc_api::starknet_ext::StarknetApiExtServer;
use katana_rpc_client::starknet::Client as StarknetClient;
#[cfg(feature = "cartridge")]
use katana_rpc_server::cartridge::CartridgeApi;
use katana_rpc_server::cors::Cors;
use katana_rpc_server::dev::DevApi;
#[cfg(feature = "cartridge")]
use katana_rpc_server::starknet::PaymasterConfig;
use katana_rpc_server::starknet::{StarknetApi, StarknetApiConfig};
use katana_rpc_server::{RpcServer, RpcServerHandle};
use katana_rpc_types::GetBlockWithTxHashesResponse;
use katana_stage::Sequencing;
use katana_tasks::TaskManager;
use num_traits::ToPrimitive;
use tracing::info;

use crate::config::sequencing::MiningMode;
use crate::config::NodeConfig;
use crate::exit::NodeStoppedFuture;

/// A node instance.
///
/// The struct contains the handle to all the components of the node.
#[must_use = "Node does nothing unless launched."]
#[derive(Debug)]
pub struct Sequencer<C, P, E = ()>
where
    C: ChainSpecT,
    P: ProviderFactory,
    <P as ProviderFactory>::Provider: ProviderRO,
    <P as ProviderFactory>::ProviderMut: ProviderRW,
{
    config: Arc<NodeConfig<C, E>>,
    db: katana_db::Db,
    storage: P,
    pool: TxPool,
    rpc_server: RpcServer,
    task_manager: TaskManager,
    backend: Arc<Backend<BlockifierFactory<C>, P, C>>,
    block_producer: BlockProducer<BlockifierFactory<C>, P, C>,
    gateway_server: Option<GatewayServer<TxPool, P, C>>,
    metrics_server: Option<MetricsServer<Prometheus>>,
}

impl<C, P, E> Sequencer<C, P, E>
where
    C: ChainSpecT,
    P: ProviderFactory + Clone,
    <P as ProviderFactory>::Provider: ProviderRO,
    <P as ProviderFactory>::ProviderMut: ProviderRW,
{
    /// Build the node components from the given [`Config`].
    ///
    /// This returns a [`Node`] instance which can be launched with the all the necessary components
    /// configured.
    pub fn build_with_provider(
        db: katana_db::Db,
        storage: P,
        config: NodeConfig<C, E>,
    ) -> Result<Sequencer<C, P, E>> {
        if config.metrics.is_some() {
            // Metrics recorder must be initialized before calling any of the metrics macros, in
            // order for it to be registered.
            let _ = PrometheusRecorder::install("katana")?;
        }

        // -- build task manager

        let task_manager = TaskManager::current();
        let task_spawner = task_manager.task_spawner();

        // --- build executor factory

        // Create versioned constants overrides from config
        let overrides = Some(VersionedConstantsOverrides {
            invoke_tx_max_n_steps: Some(config.execution.invocation_max_steps),
            validate_max_n_steps: Some(config.execution.validation_max_steps),
            max_recursion_depth: Some(config.execution.max_recursion_depth),
        });

        let execution_flags = ExecutionFlags::new()
            .with_account_validation(config.dev.account_validation)
            .with_fee(config.dev.fee);

        let executor_factory = {
            #[allow(unused_mut)]
            let mut class_cache = ClassCache::builder();

            #[cfg(feature = "native")]
            {
                info!(enabled = config.execution.compile_native, "Cairo native compilation");
                class_cache = class_cache.compile_native(config.execution.compile_native);
            }

            let global_class_cache = class_cache.build_global()?;

            let factory = BlockifierFactory::new(
                overrides,
                execution_flags.clone(),
                config.sequencing.block_limits(),
                global_class_cache,
                config.chain.clone(),
            );

            Arc::new(factory)
        };

        // --- build l1 gas oracle

        // Check if the user specify a fixed gas price in the dev config.
        let gas_oracle = if let Some(prices) = &config.dev.fixed_gas_prices {
            GasPriceOracle::fixed(
                prices.l2_gas_prices.clone(),
                prices.l1_gas_prices.clone(),
                prices.l1_data_gas_prices.clone(),
            )
        } else if let Some(settlement) = config.chain.settlement() {
            match settlement {
                SettlementLayer::Starknet { rpc_url, .. } => {
                    GasPriceOracle::sampled_starknet(rpc_url.clone())
                }
                SettlementLayer::Ethereum { rpc_url, .. } => {
                    GasPriceOracle::sampled_ethereum(rpc_url.clone())
                }
                SettlementLayer::Sovereign { .. } => {
                    GasPriceOracle::Fixed(FixedPriceOracle::default())
                }
            }
        } else {
            GasPriceOracle::Fixed(FixedPriceOracle::default())
        };

        // Get cfg_env before moving executor_factory into Backend
        let versioned_constant_overrides = executor_factory.overrides().cloned();

        // --- build backend

        let block_context_generator = BlockContextGenerator::default().into();
        let backend = Arc::new(Backend {
            gas_oracle: gas_oracle.clone(),
            storage: storage.clone(),
            executor_factory,
            block_context_generator,
            chain_spec: config.chain.clone(),
        });

        // For forking, the genesis is already set up in ForkedSequencer::build
        backend.init_genesis(false).context("failed to initialize genesis")?;

        // --- build block producer

        let block_producer = match config.sequencing.mining {
            MiningMode::Instant => BlockProducer::instant(Arc::clone(&backend)),
            MiningMode::Interval(interval) => {
                BlockProducer::interval(Arc::clone(&backend), interval)
            }
            MiningMode::Manual => BlockProducer::on_demand(Arc::clone(&backend)),
        };

        // --- build transaction pool

        let validator = block_producer.validator();
        let pool = TxPool::new(validator.clone(), FiFo::new());

        // --- build rpc server

        let mut rpc_modules = RpcModule::new(());

        let cors = Cors::new()
        .allow_origins(config.rpc.cors_origins.clone())
        // Allow `POST` when accessing the resource
        .allow_methods([Method::POST, Method::GET])
        .allow_headers([CONTENT_TYPE, "argent-client".parse().unwrap(), "argent-version".parse().unwrap()]);

        #[cfg(feature = "cartridge")]
        let paymaster = if let Some(paymaster) = &config.paymaster {
            anyhow::ensure!(
                config.rpc.apis.contains(&RpcModuleKind::Cartridge),
                "Cartridge API should be enabled when paymaster is set"
            );

            let api = CartridgeApi::new(
                backend.clone(),
                block_producer.clone(),
                pool.clone(),
                task_spawner.clone(),
                paymaster.cartridge_api_url.clone(),
            );

            rpc_modules.merge(CartridgeApiServer::into_rpc(api))?;

            Some(PaymasterConfig { cartridge_api_url: paymaster.cartridge_api_url.clone() })
        } else {
            None
        };

        // --- build starknet api

        let starknet_api_cfg = StarknetApiConfig {
            max_event_page_size: config.rpc.max_event_page_size,
            max_proof_keys: config.rpc.max_proof_keys,
            max_call_gas: config.rpc.max_call_gas,
            max_concurrent_estimate_fee_requests: config.rpc.max_concurrent_estimate_fee_requests,
            simulation_flags: execution_flags,
            versioned_constant_overrides,
            #[cfg(feature = "cartridge")]
            paymaster,
        };

        let chain_spec = backend.chain_spec.clone();

        let starknet_api = StarknetApi::new(
            chain_spec.clone(),
            pool.clone(),
            task_spawner.clone(),
            block_producer.clone(),
            gas_oracle.clone(),
            starknet_api_cfg,
            storage.clone(),
        );

        if config.rpc.apis.contains(&RpcModuleKind::Starknet) {
            #[cfg(feature = "explorer")]
            if config.rpc.explorer {
                rpc_modules.merge(StarknetApiExtServer::into_rpc(starknet_api.clone()))?;
            }

            rpc_modules.merge(StarknetApiServer::into_rpc(starknet_api.clone()))?;
            rpc_modules.merge(StarknetWriteApiServer::into_rpc(starknet_api.clone()))?;
            rpc_modules.merge(StarknetTraceApiServer::into_rpc(starknet_api.clone()))?;
        }

        if config.rpc.apis.contains(&RpcModuleKind::Dev) {
            let api = DevApi::new(backend.clone(), block_producer.clone());
            rpc_modules.merge(DevApiServer::into_rpc(api))?;
        }

        #[allow(unused_mut)]
        let mut rpc_server =
            RpcServer::new().metrics(true).health_check(true).cors(cors).module(rpc_modules)?;

        #[cfg(feature = "explorer")]
        {
            rpc_server = rpc_server.explorer(config.rpc.explorer);
        }

        if let Some(timeout) = config.rpc.timeout {
            rpc_server = rpc_server.timeout(timeout);
        };

        if let Some(max_connections) = config.rpc.max_connections {
            rpc_server = rpc_server.max_connections(max_connections);
        }

        if let Some(max_request_body_size) = config.rpc.max_request_body_size {
            rpc_server = rpc_server.max_request_body_size(max_request_body_size);
        }

        if let Some(max_response_body_size) = config.rpc.max_response_body_size {
            rpc_server = rpc_server.max_response_body_size(max_response_body_size);
        }

        // --- build feeder gateway server (optional)

        let gateway_server = if let Some(gw_config) = &config.gateway {
            let mut server = GatewayServer::new(starknet_api)
                .health_check(true)
                .metered(config.metrics.is_some());

            if let Some(timeout) = gw_config.timeout {
                server = server.timeout(timeout);
            }

            Some(server)
        } else {
            None
        };

        // --- build metrics server (optional)

        let metrics_server = if config.metrics.is_some() {
            let db_metrics = Box::new(db.clone()) as Box<dyn Report>;
            let disk_metrics = Box::new(DiskReporter::new(db.path())?) as Box<dyn Report>;
            let reports: Vec<Box<dyn Report>> = vec![db_metrics, disk_metrics];

            let exporter = PrometheusRecorder::current().expect("qed; should exist at this point");
            let server = MetricsServer::new(exporter).with_process_metrics().reports(reports);

            Some(server)
        } else {
            None
        };

        Ok(Sequencer {
            db,
            storage,
            pool,
            backend,
            rpc_server,
            gateway_server,
            block_producer,
            metrics_server,
            config: Arc::new(config),
            task_manager,
        })
    }
}
