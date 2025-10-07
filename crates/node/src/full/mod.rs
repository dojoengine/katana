//! Experimental full node implementation.

mod exit;
mod pool;
mod tip_watcher;

use std::future::IntoFuture;
use std::sync::Arc;

use anyhow::Result;
use exit::NodeStoppedFuture;
use http::header::CONTENT_TYPE;
use http::Method;
use katana_gateway::client::Client as SequencerGateway;
use katana_metrics::exporters::prometheus::PrometheusRecorder;
use katana_metrics::{Report, Server as MetricsServer};
use katana_pipeline::{Pipeline, PipelineHandle};
use katana_pool::ordering::FiFo;
use katana_pool::pool::Pool;
use katana_pool::validation::NoopValidator;
use katana_pool::TxPool;
use katana_primitives::transaction::ExecutableTxWithHash;
use katana_provider::providers::db::DbProvider;
use katana_rpc::cors::Cors;
use katana_rpc::{RpcServer, RpcServerHandle};
use katana_stage::blocks::BatchBlockDownloader;
use katana_stage::{Blocks, Classes};
use katana_tasks::TaskManager;
use tip_watcher::ChainTipWatcher;
use tracing::info;

use crate::config::db::DbConfig;
use crate::config::metrics::MetricsConfig;
use crate::config::rpc::RpcConfig;
use crate::full::pool::{FullNodePool, GatewayProxyValidator};

#[derive(Debug, Clone)]
pub enum Network {
    Mainnet,
    Sepolia,
}

#[derive(Debug)]
pub struct Config {
    pub db: DbConfig,
    pub rpc: RpcConfig,
    pub metrics: Option<MetricsConfig>,
    pub gateway_api_key: Option<String>,
    pub eth_rpc_url: String,
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

        // -- build db and storage provider

        let path = config.db.dir.clone().expect("database path must exist");

        info!(target: "node", path = %path.display(), "Initializing database.");
        let db = katana_db::Db::new(path)?;

        let provider = DbProvider::new(db.clone());

        // --- build gateway client

        let gateway_client = if let Some(ref key) = config.gateway_api_key {
            SequencerGateway::sepolia().with_api_key(key.clone())
        } else {
            SequencerGateway::sepolia()
        };

        // --- build transaction pool

        let validator = GatewayProxyValidator::new(gateway_client.clone());
        let pool = FullNodePool::new(validator, FiFo::new());

        // --- build pipeline

        let (mut pipeline, _) = Pipeline::new(provider.clone(), 64);
        let block_downloader = BatchBlockDownloader::new_gateway(gateway_client.clone(), 3);
        pipeline.add_stage(Blocks::new(provider.clone(), block_downloader));
        pipeline.add_stage(Classes::new(provider, gateway_client.clone(), 3));

        let cors = Cors::new()
        .allow_origins(config.rpc.cors_origins.clone())
        // Allow `POST` when accessing the resource
        .allow_methods([Method::POST, Method::GET])
        .allow_headers([CONTENT_TYPE, "argent-client".parse().unwrap(), "argent-version".parse().unwrap()]);

        let rpc_server = RpcServer::new().metrics(true).health_check(true).cors(cors);

        Ok(Node {
            db,
            pool,
            pipeline,
            rpc_server,
            task_manager,
            gateway_client,
            config: Arc::new(config),
        })
    }

    pub async fn launch(self) -> Result<LaunchedNode> {
        println!("Launching node");

        if let Some(ref cfg) = self.config.metrics {
            let reports: Vec<Box<dyn Report>> = vec![Box::new(self.db.clone()) as Box<dyn Report>];
            let exporter = PrometheusRecorder::current().expect("qed; should exist at this point");

            let addr = cfg.socket_addr();
            let server = MetricsServer::new(exporter).with_process_metrics().with_reports(reports);
            self.task_manager.task_spawner().build_task().spawn(server.start(addr));

            info!(%addr, "Metrics server started.");
        }

        let pipeline_handle = self.pipeline.handle();

        let core_contract = match self.config.network {
            Network::Mainnet => {
                katana_starknet::StarknetCore::new_http_mainnet(&self.config.eth_rpc_url).await?
            }
            Network::Sepolia => {
                katana_starknet::StarknetCore::new_http_sepolia(&self.config.eth_rpc_url).await?
            }
        };

        let tip_watcher = ChainTipWatcher::new(core_contract, pipeline_handle.clone());

        self.task_manager
            .task_spawner()
            .build_task()
            .graceful_shutdown()
            .name("Chain tip watcher")
            .spawn(tip_watcher.into_future());

        self.task_manager
            .task_spawner()
            .build_task()
            .graceful_shutdown()
            .name("Pipeline")
            .spawn(self.pipeline.into_future());

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
        self.task_manager.shutdown().await;
        Ok(())
    }

    pub fn stopped(&self) -> NodeStoppedFuture<'_> {
        NodeStoppedFuture::new(self)
    }
}
