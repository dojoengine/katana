//! Experimental full node implementation.

use std::future::IntoFuture;
use std::sync::Arc;

use anyhow::Result;
use http::header::CONTENT_TYPE;
use http::Method;
use jsonrpsee::RpcModule;
use katana_chain_spec::ChainSpec;
use katana_executor::ExecutionFlags;
use katana_gas_price_oracle::GasPriceOracle;
use katana_gateway_client::Client as SequencerGateway;
use katana_metrics::exporters::prometheus::PrometheusRecorder;
use katana_metrics::{Report, Server as MetricsServer};
use katana_pipeline::{Pipeline, PipelineHandle};
use katana_pool::ordering::TipOrdering;
use katana_provider::providers::db::DbProvider;
use katana_provider::BlockchainProvider;
use katana_rpc_api::starknet::{StarknetApiServer, StarknetTraceApiServer, StarknetWriteApiServer};
use katana_rpc_server::cors::Cors;
use katana_rpc_server::starknet::{StarknetApi, StarknetApiConfig};
use katana_rpc_server::{RpcServer, RpcServerHandle};
use katana_stage::blocks::BatchBlockDownloader;
use katana_stage::{Blocks, Classes, StateTrie};
use katana_tasks::TaskManager;
use tracing::{error, info};

use crate::config::db::DbConfig;
use crate::config::metrics::MetricsConfig;
use crate::full::pending::PreconfStateFactory;

mod exit;
mod pending;
mod pool;
pub mod tip_watcher;

use exit::NodeStoppedFuture;
use tip_watcher::ChainTipWatcher;

use crate::config::rpc::{RpcConfig, RpcModuleKind};
use crate::full::pool::{FullNodePool, GatewayProxyValidator};

#[derive(
    Debug,
    Copy,
    Clone,
    serde::Serialize,
    serde::Deserialize,
    PartialEq,
    Default,
    strum::Display,
    strum::EnumString,
)]
pub enum Network {
    #[default]
    Mainnet,
    Sepolia,
}

pub use katana_pipeline::PruningConfig;

#[derive(Debug)]
pub struct Config {
    pub db: DbConfig,
    pub rpc: RpcConfig,
    pub metrics: Option<MetricsConfig>,
    pub pruning: Option<PruningConfig>,
    pub gateway_api_key: Option<String>,
    pub network: Network,
}

#[derive(Debug)]
pub struct Node {
    pub db: katana_db::Db,
    pub pool: FullNodePool,
    pub config: Arc<Config>,
    pub task_manager: TaskManager,
    pub pipeline: Pipeline<DbProvider>,
    pub rpc_server: RpcServer,
    pub gateway_client: SequencerGateway,
    pub chain_tip_watcher: ChainTipWatcher<SequencerGateway>,
}

impl Node {
    pub fn build(config: Config) -> Result<Self> {
        if config.metrics.is_some() {
            // Metrics recorder must be initialized before calling any of the metrics macros, in
            // order for it to be registered.
            let _ = PrometheusRecorder::install("katana")?;
        }

        // -- build task manager

        let task_manager = TaskManager::current();
        let task_spawner = task_manager.task_spawner();

        // -- build db and storage provider

        let path = config.db.dir.clone().expect("database path must exist");

        info!(target: "node", path = %path.display(), "Initializing database.");
        let db = katana_db::Db::new(path)?;

        let provider = DbProvider::new(db.clone());

        // --- build gateway client

        let gateway_client = match config.network {
            Network::Mainnet => SequencerGateway::mainnet(),
            Network::Sepolia => SequencerGateway::sepolia(),
        };

        let gateway_client = if let Some(ref key) = config.gateway_api_key {
            gateway_client.with_api_key(key.clone())
        } else {
            gateway_client
        };

        // --- build transaction pool

        let validator = GatewayProxyValidator::new(gateway_client.clone());
        let pool = FullNodePool::new(validator, TipOrdering::new());

        // --- build pipeline

        let (mut pipeline, pipeline_handle) = Pipeline::new(provider.clone(), 50);

        // Configure pruning if specified
        if let Some(pruning_config) = config.pruning {
            pipeline.set_pruning_config(pruning_config);
        }

        let block_downloader = BatchBlockDownloader::new_gateway(gateway_client.clone(), 8);
        pipeline.add_stage(Blocks::new(provider.clone(), block_downloader));
        pipeline.add_stage(Classes::new(provider.clone(), gateway_client.clone(), 8));
        pipeline.add_stage(StateTrie::new(provider.clone()));

        // -- build chain tip watcher using gateway client

        let chain_tip_watcher = ChainTipWatcher::new(gateway_client.clone());

        let preconf_factory = PreconfStateFactory::new(
            provider.clone(),
            gateway_client.clone(),
            pipeline_handle.subscribe_blocks(),
            chain_tip_watcher.subscribe(),
        );

        // --- build rpc server

        let mut rpc_modules = RpcModule::new(());

        let cors = Cors::new()
	       	.allow_origins(config.rpc.cors_origins.clone())
	       	// Allow `POST` when accessing the resource
	       	.allow_methods([Method::POST, Method::GET])
	       	.allow_headers([CONTENT_TYPE, "argent-client".parse().unwrap(), "argent-version".parse().unwrap()]);

        // // --- build starknet api

        let starknet_api_cfg = StarknetApiConfig {
            max_event_page_size: config.rpc.max_event_page_size,
            max_proof_keys: config.rpc.max_proof_keys,
            max_call_gas: config.rpc.max_call_gas,
            max_concurrent_estimate_fee_requests: config.rpc.max_concurrent_estimate_fee_requests,
            simulation_flags: ExecutionFlags::default(),
            versioned_constant_overrides: None,
            #[cfg(feature = "cartridge")]
            paymaster: None,
        };

        let chain_spec = match config.network {
            Network::Mainnet => ChainSpec::mainnet(),
            Network::Sepolia => ChainSpec::sepolia(),
        };

        let starknet_api = StarknetApi::new(
            Arc::new(chain_spec),
            BlockchainProvider::new(Box::new(provider.clone())),
            pool.clone(),
            task_spawner.clone(),
            preconf_factory,
            GasPriceOracle::create_for_testing(),
            starknet_api_cfg,
        );

        if config.rpc.apis.contains(&RpcModuleKind::Starknet) {
            #[cfg(feature = "explorer")]
            if config.rpc.explorer {
                use katana_rpc_api::starknet_ext::StarknetApiExtServer;
                rpc_modules.merge(StarknetApiExtServer::into_rpc(starknet_api.clone()))?;
            }

            rpc_modules.merge(StarknetApiServer::into_rpc(starknet_api.clone()))?;
            rpc_modules.merge(StarknetWriteApiServer::into_rpc(starknet_api.clone()))?;
            rpc_modules.merge(StarknetTraceApiServer::into_rpc(starknet_api.clone()))?;
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

        Ok(Node {
            db,
            pool,
            pipeline,
            rpc_server,
            task_manager,
            gateway_client,
            chain_tip_watcher,
            config: Arc::new(config),
        })
    }

    pub async fn launch(self) -> Result<LaunchedNode> {
        if let Some(ref cfg) = self.config.metrics {
            let reports: Vec<Box<dyn Report>> = vec![Box::new(self.db.clone()) as Box<dyn Report>];
            let exporter = PrometheusRecorder::current().expect("qed; should exist at this point");

            let addr = cfg.socket_addr();
            let server = MetricsServer::new(exporter).with_process_metrics().with_reports(reports);
            self.task_manager.task_spawner().build_task().spawn(server.start(addr));

            info!(%addr, "Metrics server started.");
        }

        let pipeline_handle = self.pipeline.handle();

        let mut tip_subscription = self.chain_tip_watcher.subscribe();
        let pipeline_handle_clone = pipeline_handle.clone();

        self.task_manager
            .task_spawner()
            .build_task()
            .name("Pipeline")
            .spawn(self.pipeline.into_future());

        self.task_manager
            .task_spawner()
            .build_task()
            .graceful_shutdown()
            .name("Chain tip watcher")
            .spawn(self.chain_tip_watcher.into_future());

        // spawn a task for updating the pipeline's tip based on chain tip changes
        self.task_manager.task_spawner().spawn(async move {
            loop {
                match tip_subscription.changed().await {
                    Ok(new_tip) => pipeline_handle_clone.set_tip(new_tip),
                    Err(err) => {
                        error!(error = ?err, "Error updating pipeline tip.");
                        break;
                    }
                }
            }
        });

        // --- start the rpc server

        let rpc = self.rpc_server.start(self.config.rpc.socket_addr()).await?;

        Ok(LaunchedNode {
            db: self.db,
            config: self.config,
            task_manager: self.task_manager,
            pipeline: pipeline_handle,
            rpc,
        })
    }
}

#[derive(Debug)]
pub struct LaunchedNode {
    pub db: katana_db::Db,
    pub task_manager: TaskManager,
    pub config: Arc<Config>,
    pub rpc: RpcServerHandle,
    pub pipeline: PipelineHandle,
}

impl LaunchedNode {
    pub async fn stop(&self) -> Result<()> {
        self.rpc.stop()?;
        self.pipeline.stop();

        self.pipeline.stopped().await;
        self.task_manager.shutdown().await;

        Ok(())
    }

    pub fn stopped(&self) -> NodeStoppedFuture<'_> {
        NodeStoppedFuture::new(self)
    }
}
