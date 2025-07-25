// #![cfg_attr(not(test), warn(unused_crate_dependencies))]

#[cfg(feature = "full-node")]
pub mod full;

pub mod config;
pub mod exit;

use std::future::IntoFuture;
use std::sync::Arc;

use anyhow::{anyhow, bail, Context, Result};
#[cfg(feature = "cartridge")]
use cartridge::paymaster::layer::PaymasterLayer;
#[cfg(feature = "cartridge")]
use cartridge::paymaster::Paymaster;
#[cfg(feature = "cartridge")]
use cartridge::rpc::{CartridgeApi, CartridgeApiServer};
use config::rpc::RpcModuleKind;
use config::Config;
use http::header::CONTENT_TYPE;
use http::Method;
use jsonrpsee::core::middleware::layer::Either;
use jsonrpsee::RpcModule;
use katana_chain_spec::{ChainSpec, SettlementLayer};
use katana_core::backend::storage::Blockchain;
use katana_core::backend::Backend;
use katana_core::env::BlockContextGenerator;
use katana_core::service::block_producer::BlockProducer;
use katana_db::Db;
use katana_executor::implementation::blockifier::cache::ClassCache;
use katana_executor::implementation::blockifier::BlockifierFactory;
use katana_executor::ExecutionFlags;
use katana_gas_price_oracle::{FixedPriceOracle, GasPriceOracle};
use katana_metrics::exporters::prometheus::PrometheusRecorder;
use katana_metrics::sys::DiskReporter;
use katana_metrics::{Report, Server as MetricsServer};
use katana_pool::ordering::FiFo;
use katana_pool::TxPool;
use katana_primitives::env::{CfgEnv, FeeTokenAddressses};
use katana_primitives::genesis::allocation::GenesisAccountAlloc;
use katana_rpc::cors::Cors;
use katana_rpc::dev::DevApi;
use katana_rpc::logger::RpcLoggerLayer;
use katana_rpc::metrics::RpcServerMetricsLayer;
use katana_rpc::starknet::forking::ForkedClient;
use katana_rpc::starknet::{StarknetApi, StarknetApiConfig};
use katana_rpc::{RpcServer, RpcServerHandle, RpcServiceBuilder};
use katana_rpc_api::dev::DevApiServer;
use katana_rpc_api::starknet::{StarknetApiServer, StarknetTraceApiServer, StarknetWriteApiServer};
use katana_stage::Sequencing;
use katana_tasks::TaskManager;
use starknet::signers::SigningKey;
use tower::layer::util::{Identity, Stack};
use tracing::info;

use crate::exit::NodeStoppedFuture;

/// The concrete type of of the RPC middleware stack used by the node.
type NodeRpcMiddleware = Stack<
    Either<PaymasterLayer<BlockifierFactory>, Identity>,
    Stack<RpcLoggerLayer, Stack<RpcServerMetricsLayer, Identity>>,
>;

pub type NodeRpcServer = RpcServer<NodeRpcMiddleware>;

/// A node instance.
///
/// The struct contains the handle to all the components of the node.
#[must_use = "Node does nothing unless launched."]
#[derive(Debug)]
pub struct Node {
    config: Arc<Config>,
    pool: TxPool,
    db: katana_db::Db,
    rpc_server: NodeRpcServer,
    task_manager: TaskManager,
    backend: Arc<Backend<BlockifierFactory>>,
    block_producer: BlockProducer<BlockifierFactory>,
}

impl Node {
    /// Build the node components from the given [`Config`].
    ///
    /// This returns a [`Node`] instance which can be launched with the all the necessary components
    /// configred.
    pub async fn build(config: Config) -> Result<Node> {
        let mut config = config;

        if config.metrics.is_some() {
            // Metrics recorder must be initialized before calling any of the metrics macros, in
            // order for it to be registered.
            let _ = PrometheusRecorder::install("katana")?;
        }

        // --- build executor factory

        let fee_token_addresses = match config.chain.as_ref() {
            ChainSpec::Dev(cs) => {
                FeeTokenAddressses { eth: cs.fee_contracts.eth, strk: cs.fee_contracts.strk }
            }
            ChainSpec::Rollup(cs) => {
                FeeTokenAddressses { eth: cs.fee_contract.strk, strk: cs.fee_contract.strk }
            }
        };

        let cfg_env = CfgEnv {
            fee_token_addresses,
            chain_id: config.chain.id(),
            invoke_tx_max_n_steps: config.execution.invocation_max_steps,
            validate_max_n_steps: config.execution.validation_max_steps,
            max_recursion_depth: config.execution.max_recursion_depth,
        };

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
                cfg_env,
                execution_flags,
                config.sequencing.block_limits(),
                global_class_cache,
            );

            Arc::new(factory)
        };

        // --- build backend

        let (blockchain, db, forked_client) = if let Some(cfg) = &config.forking {
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
            let forked_client = ForkedClient::new_http(cfg.url.clone(), block_num);

            (bc, db, Some(forked_client))
        } else if let Some(db_path) = &config.db.dir {
            let db = katana_db::Db::new(db_path)?;
            (Blockchain::new_with_db(db.clone()), db, None)
        } else {
            let db = katana_db::Db::in_memory()?;
            (Blockchain::new_with_db(db.clone()), db, None)
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

        let block_context_generator = BlockContextGenerator::default().into();
        let backend = Arc::new(Backend {
            gas_oracle,
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

        let starknet_api_cfg = StarknetApiConfig {
            max_call_gas: config.rpc.max_call_gas,
            max_proof_keys: config.rpc.max_proof_keys,
            max_event_page_size: config.rpc.max_event_page_size,
            max_concurrent_estimate_fee_requests: config.rpc.max_concurrent_estimate_fee_requests,
        };

        let starknet_api = if let Some(client) = forked_client {
            StarknetApi::new_forked(
                backend.clone(),
                pool.clone(),
                block_producer.clone(),
                client,
                starknet_api_cfg,
            )
        } else {
            StarknetApi::new(
                backend.clone(),
                pool.clone(),
                Some(block_producer.clone()),
                starknet_api_cfg,
            )
        };

        if config.rpc.apis.contains(&RpcModuleKind::Starknet) {
            rpc_modules.merge(StarknetApiServer::into_rpc(starknet_api.clone()))?;
            rpc_modules.merge(StarknetWriteApiServer::into_rpc(starknet_api.clone()))?;
            rpc_modules.merge(StarknetTraceApiServer::into_rpc(starknet_api.clone()))?;
        }

        if config.rpc.apis.contains(&RpcModuleKind::Dev) {
            let api = DevApi::new(backend.clone(), block_producer.clone());
            rpc_modules.merge(DevApiServer::into_rpc(api))?;
        }

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
                paymaster.cartridge_api_url.clone(),
            );

            rpc_modules.merge(CartridgeApiServer::into_rpc(api))?;

            // build cartridge paymaster

            let cartridge_api_client = cartridge::Client::new(paymaster.cartridge_api_url.clone());

            // For now, we use the first predeployed account in the genesis as the paymaster
            // account.
            let (pm_address, pm_acc) = config
                .chain
                .genesis()
                .accounts()
                .nth(0)
                .ok_or(anyhow!("Cartridge paymaster account doesn't exist"))?;

            // TODO: create a dedicated types for aux accounts (eg paymaster)
            let pm_private_key = if let GenesisAccountAlloc::DevAccount(pm) = pm_acc {
                pm.private_key
            } else {
                bail!("Paymaster is not a dev account")
            };

            Some(Paymaster::new(
                starknet_api,
                cartridge_api_client,
                pool.clone(),
                config.chain.id(),
                *pm_address,
                SigningKey::from_secret_scalar(pm_private_key),
            ))
        } else {
            None
        };

        // build rpc middleware

        let rpc_middleware = RpcServiceBuilder::new()
            .layer(RpcServerMetricsLayer::new(&rpc_modules))
            .layer(katana_rpc::logger::RpcLoggerLayer::new())
            .option_layer(paymaster.map(|p| p.layer()));

        let mut rpc_server = katana_rpc::RpcServer::new()
            .rpc_middleware(rpc_middleware)
            .health_check(true)
            .cors(cors)
            .module(rpc_modules)?;

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

        Ok(Node {
            db,
            pool,
            backend,
            rpc_server,
            block_producer,
            config: Arc::new(config),
            task_manager: TaskManager::current(),
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
            .critical()
            .name("Sequencing")
            .spawn(sequencing.into_future());

        // --- start the rpc server

        let rpc_handle = self.rpc_server.start(self.config.rpc.socket_addr()).await?;

        // --- start the gas oracle worker task

        if let Some(worker) = self.backend.gas_oracle.run_worker() {
            self.task_manager
                .task_spawner()
                .build_task()
                .critical()
                .name("gas oracle")
                .spawn(worker);
        }

        info!(target: "node", "Gas price oracle worker started.");

        Ok(LaunchedNode { node: self, rpc: rpc_handle })
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
    pub fn rpc(&self) -> &NodeRpcServer {
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

    /// Stops the node.
    ///
    /// This will instruct the node to stop and wait until it has actually stop.
    pub async fn stop(&self) -> Result<()> {
        // TODO: wait for the rpc server to stop instead of just stopping it.
        self.rpc.stop()?;
        self.node.task_manager.shutdown().await;
        Ok(())
    }

    /// Returns a future which resolves only when the node has stopped.
    pub fn stopped(&self) -> NodeStoppedFuture<'_> {
        NodeStoppedFuture::new(self)
    }
}
