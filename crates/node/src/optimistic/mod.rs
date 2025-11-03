use std::sync::Arc;

use anyhow::Result;
use http::header::CONTENT_TYPE;
use http::Method;
use jsonrpsee::http_client::HttpClientBuilder;
use jsonrpsee::RpcModule;
use katana_chain_spec::ChainSpec;
use katana_core::backend::storage::Blockchain;
use katana_core::backend::Backend;
use katana_core::env::BlockContextGenerator;
use katana_executor::implementation::blockifier::cache::ClassCache;
use katana_executor::implementation::blockifier::BlockifierFactory;
use katana_executor::{BlockLimits, ExecutionFlags};
use katana_gas_price_oracle::GasPriceOracle;
use katana_metrics::exporters::prometheus::PrometheusRecorder;
use katana_metrics::sys::DiskReporter;
use katana_metrics::{Report, Server as MetricsServer};
use katana_optimistic::executor::{OptimisticExecutor, OptimisticState};
use katana_optimistic::pool::{PoolValidator, TxPool};
use katana_pool::ordering::FiFo;
use katana_primitives::block::BlockIdOrTag;
use katana_primitives::env::{CfgEnv, FeeTokenAddressses};
use katana_provider::providers::db::cached::CachedDbProvider;
use katana_rpc::cors::Cors;
use katana_rpc::starknet::forking::ForkedClient;
use katana_rpc::starknet::{StarknetApi, StarknetApiConfig};
use katana_rpc::{RpcServer, RpcServerHandle};
use katana_rpc_api::starknet::{StarknetApiServer, StarknetTraceApiServer, StarknetWriteApiServer};
use katana_tasks::{JoinHandle, TaskManager};
use tracing::info;

mod config;

use config::Config;

use crate::config::rpc::RpcModuleKind;

#[derive(Debug)]
pub struct Node {
    config: Arc<Config>,
    pool: TxPool,
    db: katana_db::Db,
    rpc_server: RpcServer,
    task_manager: TaskManager,
    executor: OptimisticExecutor,
    backend: Arc<Backend<BlockifierFactory>>,
}

impl Node {
    pub async fn build(config: Config) -> Result<Node> {
        if config.metrics.is_some() {
            // Metrics recorder must be initialized before calling any of the metrics macros, in
            // order for it to be registered.
            let _ = PrometheusRecorder::install("katana")?;
        }

        // -- build task manager

        let task_manager = TaskManager::current();
        let task_spawner = task_manager.task_spawner();

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
            invoke_tx_max_n_steps: 10_000_000,
            validate_max_n_steps: 10_000_000,
            max_recursion_depth: 100,
        };

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
                ExecutionFlags::new(),
                BlockLimits::default(),
                global_class_cache,
            );

            Arc::new(factory)
        };

        // --- build backend

        let http_client = HttpClientBuilder::new().build(config.forking.url.as_str())?;
        let starknet_client = katana_rpc_client::starknet::Client::new(http_client);

        let db = katana_db::Db::in_memory()?;
        let forked_block_id = BlockIdOrTag::Latest;

        let database = CachedDbProvider::new(db.clone(), forked_block_id, starknet_client.clone());
        let blockchain = Blockchain::new(database.clone());

        let forked_client = ForkedClient::new(starknet_client.clone(), forked_block_id);

        let gpo = GasPriceOracle::sampled_starknet(config.forking.url.clone());

        let block_context_generator = BlockContextGenerator::default().into();
        let backend = Arc::new(Backend {
            gas_oracle: gpo.clone(),
            blockchain: blockchain.clone(),
            executor_factory: executor_factory.clone(),
            block_context_generator,
            chain_spec: config.chain.clone(),
        });

        // --- build transaction pool

        let pool_validator = PoolValidator::new(starknet_client.clone());
        let pool = TxPool::new(pool_validator, FiFo::new());

        // -- build executor

        let optimistic_state = OptimisticState::new(database.clone());

        let executor = OptimisticExecutor::new(
            pool.clone(),
            blockchain.clone(),
            optimistic_state.clone(),
            executor_factory.clone(),
            task_spawner.clone(),
        );

        // --- build rpc server

        let mut rpc_modules = RpcModule::new(());

        let cors = Cors::new()
        .allow_origins(config.rpc.cors_origins.clone())
        // Allow `POST` when accessing the resource
        .allow_methods([Method::POST, Method::GET])
        .allow_headers([CONTENT_TYPE, "argent-client".parse().unwrap(), "argent-version".parse().unwrap()]);

        // --- build starknet api

        let starknet_api_cfg = StarknetApiConfig {
            max_event_page_size: config.rpc.max_event_page_size,
            max_proof_keys: config.rpc.max_proof_keys,
            max_call_gas: config.rpc.max_call_gas,
            max_concurrent_estimate_fee_requests: config.rpc.max_concurrent_estimate_fee_requests,
            #[cfg(feature = "cartridge")]
            paymaster: None,
        };

        let starknet_api = StarknetApi::new_forked(
            backend.clone(),
            pool.clone(),
            forked_client,
            task_spawner.clone(),
            starknet_api_cfg,
            starknet_client.clone(),
            blockchain,
            optimistic_state.clone(),
        );

        if config.rpc.apis.contains(&RpcModuleKind::Starknet) {
            rpc_modules.merge(StarknetApiServer::into_rpc(starknet_api.clone()))?;
            rpc_modules.merge(StarknetWriteApiServer::into_rpc(starknet_api.clone()))?;
            rpc_modules.merge(StarknetTraceApiServer::into_rpc(starknet_api.clone()))?;
        }

        #[allow(unused_mut)]
        let mut rpc_server =
            RpcServer::new().metrics(true).health_check(true).cors(cors).module(rpc_modules)?;

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

        Ok(Node { db, pool, backend, rpc_server, config: config.into(), task_manager, executor })
    }

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

        // --- start the rpc server

        let rpc_handle = self.rpc_server.start(self.config.rpc.socket_addr()).await?;

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

        let executor_handle = self.executor.spawn();

        Ok(LaunchedNode {
            rpc: rpc_handle,
            backend: self.backend,
            config: self.config,
            db: self.db,
            executor: executor_handle,
            task_manager: self.task_manager,
            pool: self.pool,
            rpc_server: self.rpc_server,
        })
    }
}

#[derive(Debug)]
pub struct LaunchedNode {
    config: Arc<Config>,
    pool: TxPool,
    db: katana_db::Db,
    rpc_server: RpcServer,
    task_manager: TaskManager,
    backend: Arc<Backend<BlockifierFactory>>,
    rpc: RpcServerHandle,
    executor: JoinHandle<()>,
}
