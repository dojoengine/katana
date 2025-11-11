#![cfg_attr(not(test), warn(unused_crate_dependencies))]

pub mod full;

pub mod config;
pub mod exit;

use std::future::IntoFuture;
use std::sync::Arc;

use anyhow::{Context, Result};
use config::rpc::RpcModuleKind;
use config::Config;
use http::header::CONTENT_TYPE;
use http::Method;
use jsonrpsee::http_client::HttpClientBuilder;
use jsonrpsee::RpcModule;
use katana_chain_spec::{ChainSpec, SettlementLayer};
use katana_core::backend::storage::Blockchain;
use katana_core::backend::Backend;
use katana_core::env::BlockContextGenerator;
use katana_core::service::block_producer::BlockProducer;
use katana_db::Db;
use katana_executor::implementation::blockifier::cache::ClassCache;
use katana_executor::implementation::blockifier::BlockifierFactory;
use katana_executor::{ExecutionFlags, ExecutorFactory};
use katana_gas_price_oracle::{FixedPriceOracle, GasPriceOracle};
use katana_gateway_server::{GatewayServer, GatewayServerHandle};
use katana_metrics::exporters::prometheus::PrometheusRecorder;
use katana_metrics::sys::DiskReporter;
use katana_metrics::{Report, Server as MetricsServer};
use katana_pool::ordering::FiFo;
use katana_pool::TxPool;
use katana_primitives::env::VersionedConstantsOverrides;
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
use katana_rpc_server::starknet::forking::ForkedClient;
#[cfg(feature = "cartridge")]
use katana_rpc_server::starknet::PaymasterConfig;
use katana_rpc_server::starknet::{StarknetApi, StarknetApiConfig};
use katana_rpc_server::{RpcServer, RpcServerHandle};
use katana_stage::Sequencing;
use katana_tasks::TaskManager;
use tracing::info;

use crate::exit::NodeStoppedFuture;

/// A node instance.
///
/// The struct contains the handle to all the components of the node.
#[must_use = "Node does nothing unless launched."]
#[derive(Debug)]
pub struct Node {
    config: Arc<Config>,
    pool: TxPool,
    db: katana_db::Db,
    rpc_server: RpcServer,
    task_manager: TaskManager,
    backend: Arc<Backend<BlockifierFactory>>,
    block_producer: BlockProducer<BlockifierFactory>,
    gateway_server: Option<GatewayServer>,
}

impl Node {
    /// Build the node components from the given [`Config`].
    ///
    /// This returns a [`Node`] instance which can be launched with the all the necessary components
    /// configured.
    pub async fn build(config: Config) -> Result<Node> {
        let mut config = config;

        if config.metrics.is_some() {
            // Metrics recorder must be initialized before calling any of the metrics macros, in
            // order for it to be registered.
            let _ = PrometheusRecorder::install("katana")?;
        }

        // -- build task manager

        let task_manager = TaskManager::current();
        let task_spawner = task_manager.task_spawner();

        // --- build backend

        let (blockchain, db, forked_client) = if let Some(cfg) = &config.forking {
            // NOTE: because the chain spec will be cloned for the BlockifierFactory (see below),
            // this mutation must be performed before the chain spec is cloned. Otherwise
            // this will panic.
            let chain_spec = Arc::get_mut(&mut config.chain).expect("get mut Arc");

            let ChainSpec::Dev(chain_spec) = chain_spec else {
                return Err(anyhow::anyhow!("Forking is only supported in dev mode for now"));
            };

            let db = katana_db::Db::in_memory()?;
            let (bc, block_num) =
                Blockchain::new_from_forked(db.clone(), cfg.url.clone(), cfg.block, chain_spec)
                    .await?;

            // TODO: it'd bee nice if the client can be shared on both the rpc and forked backend
            // side
            let http_client = HttpClientBuilder::new().build(cfg.url.as_ref())?;
            let rpc_client = StarknetClient::new(http_client);
            let forked_client = ForkedClient::new(rpc_client, block_num);

            (bc, db, Some(forked_client))
        } else if let Some(db_path) = &config.db.dir {
            let db = katana_db::Db::new(db_path)?;
            (Blockchain::new_with_db(db.clone()), db, None)
        } else {
            let db = katana_db::Db::in_memory()?;
            (Blockchain::new_with_db(db.clone()), db, None)
        };

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

        let block_context_generator = BlockContextGenerator::default().into();
        let backend = Arc::new(Backend {
            gas_oracle: gas_oracle.clone(),
            blockchain,
            executor_factory,
            block_context_generator,
            chain_spec: config.chain.clone(),
        });

        backend.init_genesis().context("failed to initialize genesis")?;

        // --- build block producer

        let block_producer =
            if config.sequencing.block_time.is_some() || config.sequencing.no_mining {
                if let Some(interval) = config.sequencing.block_time {
                    BlockProducer::interval(Arc::clone(&backend), interval)
                } else {
                    BlockProducer::on_demand(Arc::clone(&backend))
                }
            } else {
                BlockProducer::instant(Arc::clone(&backend))
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

        let storage_provider = backend.blockchain.provider().clone();
        let chain_spec = backend.chain_spec.clone();

        let starknet_api = if let Some(forked_client) = forked_client {
            StarknetApi::new_forked(
                chain_spec.clone(),
                storage_provider.clone(),
                pool.clone(),
                forked_client,
                task_spawner.clone(),
                block_producer.clone(),
                gas_oracle.clone(),
                starknet_api_cfg,
            )
        } else {
            StarknetApi::new(
                chain_spec.clone(),
                storage_provider.clone(),
                pool.clone(),
                task_spawner.clone(),
                block_producer.clone(),
                gas_oracle.clone(),
                starknet_api_cfg,
            )
        };

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

        Ok(Node {
            db,
            pool,
            backend,
            rpc_server,
            gateway_server,
            block_producer,
            config: Arc::new(config),
            task_manager,
        })
    }

    /// Start the node.
    ///
    /// This method will start all the node process, running them until the node is stopped.
    pub async fn launch(self) -> Result<LaunchedNode> {
        let chain = self.backend.chain_spec.id();
        info!(%chain, "Starting node.");

        // TODO: maybe move this to the build stage
        if let Some(ref cfg) = self.config.metrics {
            let db_metrics = Box::new(self.db.clone()) as Box<dyn Report>;
            let disk_metrics = Box::new(DiskReporter::new(self.db.path())?) as Box<dyn Report>;
            let reports: Vec<Box<dyn Report>> = vec![db_metrics, disk_metrics];

            let exporter = PrometheusRecorder::current().expect("qed; should exist at this point");
            let server = MetricsServer::new(exporter).with_process_metrics().with_reports(reports);

            let addr = cfg.socket_addr();
            self.task_manager.task_spawner().build_task().spawn(server.start(addr));
            info!(%addr, "Metrics server started.");
        }

        let pool = self.pool.clone();
        let backend = self.backend.clone();
        let block_producer = self.block_producer.clone();

        // --- build and run sequencing task

        let sequencing = Sequencing::new(
            pool.clone(),
            backend.clone(),
            self.task_manager.task_spawner(),
            block_producer.clone(),
            self.config.messaging.clone(),
        );

        self.task_manager
            .task_spawner()
            .build_task()
            .graceful_shutdown()
            .name("Sequencing")
            .spawn(sequencing.into_future());

        // --- start the rpc server

        let rpc_handle = self.rpc_server.start(self.config.rpc.socket_addr()).await?;

        // --- start the feeder gateway server (if configured)

        let gateway_handle = match &self.gateway_server {
            Some(server) => {
                let config = self.config().gateway.as_ref().expect("qed; must exist");
                Some(server.start(config.socket_addr()).await?)
            }
            None => None,
        };

        // --- start the gas oracle worker task

        if let Some(worker) = self.backend.gas_oracle.run_worker() {
            self.task_manager
                .task_spawner()
                .build_task()
                .graceful_shutdown()
                .name("gas oracle")
                .spawn(worker);
        }

        info!(target: "node", "Gas price oracle worker started.");

        Ok(LaunchedNode { node: self, rpc: rpc_handle, gateway: gateway_handle })
    }

    /// Returns a reference to the node's database environment (if any).
    pub fn db(&self) -> &Db {
        &self.db
    }

    pub fn backend(&self) -> &Arc<Backend<BlockifierFactory>> {
        &self.backend
    }

    /// Returns a reference to the node's transaction pool.
    pub fn pool(&self) -> &TxPool {
        &self.pool
    }

    /// Returns a reference to the node's JSON-RPC server.
    pub fn rpc(&self) -> &RpcServer {
        &self.rpc_server
    }

    /// Returns a reference to the node's configuration.
    pub fn config(&self) -> &Config {
        &self.config
    }
}

/// A handle to the launched node.
#[derive(Debug)]
pub struct LaunchedNode {
    node: Node,
    /// Handle to the rpc server.
    rpc: RpcServerHandle,
    /// Handle to the gateway server (if enabled).
    gateway: Option<GatewayServerHandle>,
}

impl LaunchedNode {
    /// Returns a reference to the [`Node`] handle.
    pub fn node(&self) -> &Node {
        &self.node
    }

    /// Returns a reference to the rpc server handle.
    pub fn rpc(&self) -> &RpcServerHandle {
        &self.rpc
    }

    /// Returns a reference to the gateway server handle (if enabled).
    pub fn gateway(&self) -> Option<&GatewayServerHandle> {
        self.gateway.as_ref()
    }

    /// Stops the node.
    ///
    /// This will instruct the node to stop and wait until it has actually stop.
    pub async fn stop(self) -> Result<()> {
        // TODO: wait for the rpc server to stop instead of just stopping it.
        self.rpc.stop()?;

        // Stop feeder gateway server if it's running
        if let Some(handle) = self.gateway {
            handle.stop()?;
        }

        self.node.task_manager.shutdown().await;
        Ok(())
    }

    /// Returns a future which resolves only when the node has stopped.
    pub fn stopped(&self) -> NodeStoppedFuture<'_> {
        NodeStoppedFuture::new(self)
    }
}
