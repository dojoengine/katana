//! Experimental full node implementation.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]

pub mod config;

use std::collections::HashSet;
use std::future::IntoFuture;
use std::sync::Arc;

use anyhow::Result;
use config::build_info::BuildInfo;
use config::db::DbConfig;
use config::gateway::GatewayConfig;
use config::metrics::MetricsConfig;
use config::rpc::{RpcConfig, RpcModuleKind};
use http::header::CONTENT_TYPE;
use http::Method;
use jsonrpsee::RpcModule;
use katana_chain_spec::ChainSpec;
use katana_db::{migration, Db};
use katana_executor::blockifier::cache::ClassCache;
use katana_executor::ExecutionFlags;
use katana_gas_price_oracle::GasPriceOracle;
use katana_gateway_client::Client as SequencerGateway;
use katana_gateway_server::{GatewayServer, GatewayServerHandle};
use katana_metrics::exporters::prometheus::{Prometheus, PrometheusRecorder};
use katana_metrics::sys::DiskReporter;
use katana_metrics::{MetricsServer, MetricsServerHandle, Report};
use katana_pipeline::{Pipeline, PipelineHandle};
use katana_pool::ordering::TipOrdering;
use katana_provider::DbProviderFactory;
use katana_rpc_api::katana::KatanaApiServer;
use katana_rpc_api::node::NodeApiServer;
use katana_rpc_api::starknet::StarknetApiServer;
use katana_rpc_server::middleware::cors::Cors;
use katana_rpc_server::node::NodeApi;
use katana_rpc_server::starknet::{RpcCache, StarknetApi, StarknetApiConfig};
use katana_rpc_server::{RpcServer, RpcServerHandle};
use katana_rpc_types::node::NodeInfo;
use katana_stage::blocks::{BatchBlockDownloader, JsonRpcBlockDownloader};
use katana_stage::classes::{GatewayClassDownloader, JsonRpcClassDownloader};
use katana_stage::{Blocks, Classes, IndexHistory, StateTrie};
use katana_tasks::TaskManager;
use tracing::{error, info};
use url::Url;

use crate::pending::PreconfStateFactory;

mod exit;
mod pending;
mod pool;
pub mod tip_watcher;

use exit::NodeStoppedFuture;
use tip_watcher::ChainTipWatcher;

use crate::pool::{FullNodePool, GatewayProxyValidator};

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
#[strum(serialize_all = "lowercase", ascii_case_insensitive)]
pub enum Network {
    #[default]
    Mainnet,
    Sepolia,
}

pub use katana_pipeline::{PipelineConfig, PruningConfig};

/// Available sync pipeline stages.
#[derive(
    Debug,
    Copy,
    Clone,
    PartialEq,
    Eq,
    Hash,
    strum_macros::EnumString,
    strum_macros::Display,
    serde::Serialize,
    serde::Deserialize,
)]
#[strum(ascii_case_insensitive)]
pub enum SyncStageKind {
    Blocks,
    Classes,
    IndexHistory,
    StateTrie,
}

#[derive(Debug, thiserror::Error)]
#[error("invalid sync stage: {0}")]
pub struct InvalidSyncStageError(String);

/// A set of sync stages to run in the pipeline.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(transparent)]
pub struct SyncStagesList(HashSet<SyncStageKind>);

impl SyncStagesList {
    /// Creates an empty stages list.
    pub fn new() -> Self {
        Self(HashSet::new())
    }

    /// Creates a list with all available stages.
    pub fn all() -> Self {
        Self(HashSet::from([
            SyncStageKind::Blocks,
            SyncStageKind::Classes,
            SyncStageKind::IndexHistory,
            SyncStageKind::StateTrie,
        ]))
    }

    /// Returns `true` if the list contains the specified `stage`.
    pub fn contains(&self, stage: &SyncStageKind) -> bool {
        self.0.contains(stage)
    }

    /// Used as the value parser for `clap`.
    pub fn parse(value: &str) -> Result<Self, InvalidSyncStageError> {
        if value.is_empty() {
            return Ok(Self::new());
        }

        let mut stages = HashSet::new();
        for name in value.split(',') {
            let trimmed = name.trim();
            if trimmed.is_empty() {
                continue;
            }

            let stage: SyncStageKind =
                trimmed.parse().map_err(|_| InvalidSyncStageError(trimmed.to_string()))?;

            stages.insert(stage);
        }

        Ok(Self(stages))
    }
}

impl Default for SyncStagesList {
    fn default() -> Self {
        Self::all()
    }
}

#[derive(Debug)]
pub struct Config {
    pub db: DbConfig,
    pub rpc: RpcConfig,
    pub pruning: PruningConfig,
    pub metrics: Option<MetricsConfig>,
    pub gateway_api_key: Option<String>,
    pub network: Network,
    pub gateway: Option<GatewayConfig>,
    pub sync: SyncConfig,
    /// Build-time identity. See [`BuildInfo`] docs.
    pub build_info: BuildInfo,
}

/// Configuration for the sync pipeline.
#[derive(Debug, Default)]
pub struct SyncConfig {
    /// The maximum block number the pipeline will sync to. When set, the pipeline
    /// will stop syncing after reaching this block while the node remains running.
    pub max_tip: Option<u64>,
    /// The source to sync blocks and classes from.
    pub source: Option<SyncSource>,
    /// Maximum number of blocks to process per pipeline iteration.
    /// Defaults to 256 if not set.
    pub chunk_size: Option<u64>,
    /// Which pipeline stages to run. Defaults to all stages.
    pub stages: SyncStagesList,
    /// Per-stage configuration.
    pub stage: StageConfig,
}

/// Per-stage configuration options.
#[derive(Debug, Clone)]
pub struct StageConfig {
    /// Number of blocks to download concurrently within each chunk.
    pub blocks_batch_size: usize,
    /// Number of classes to download concurrently within each chunk.
    pub classes_batch_size: usize,
}

impl Default for StageConfig {
    fn default() -> Self {
        Self {
            blocks_batch_size: DEFAULT_BLOCKS_BATCH_SIZE,
            classes_batch_size: DEFAULT_CLASSES_BATCH_SIZE,
        }
    }
}

pub const DEFAULT_SYNC_CHUNK_SIZE: u64 = 256;
pub const DEFAULT_BLOCKS_BATCH_SIZE: usize = 20;
pub const DEFAULT_CLASSES_BATCH_SIZE: usize = 20;

/// The source from which the node downloads blocks and classes.
#[derive(Debug, Clone)]
pub enum SyncSource {
    /// Custom feeder gateway base URL instead of the default network gateway.
    Gateway(Url),
    /// JSON-RPC endpoint URL.
    JsonRpc(Url),
}

#[derive(Debug)]
pub struct Node {
    pub provider: DbProviderFactory,
    pub db: Db,
    pub pool: FullNodePool,
    pub config: Arc<Config>,
    pub task_manager: TaskManager,
    pub pipeline: Pipeline,
    pub rpc_server: RpcServer,
    pub gateway_server: Option<GatewayServer<FullNodePool, PreconfStateFactory, DbProviderFactory>>,
    pub gateway_client: SequencerGateway,
    pub metrics_server: Option<MetricsServer<Prometheus>>,
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

        let db = Db::new(path)?;

        // --- Perform database migration, if needed
        if config.db.migrate {
            migration::Migration::new_v9(&db).run()?;
        }

        let storage_provider = DbProviderFactory::new(db.clone());

        // --- build gateway client

        let gateway_client = if let Some(SyncSource::Gateway(ref base_url)) = config.sync.source {
            let gateway = base_url.join("gateway").expect("valid URL join");
            let feeder_gateway = base_url.join("feeder_gateway").expect("valid URL join");
            SequencerGateway::new(gateway, feeder_gateway)
        } else {
            match config.network {
                Network::Mainnet => SequencerGateway::mainnet(),
                Network::Sepolia => SequencerGateway::sepolia(),
            }
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

        let chunk_size = config.sync.chunk_size.unwrap_or(DEFAULT_SYNC_CHUNK_SIZE);
        let blocks_batch_size = config.sync.stage.blocks_batch_size;
        let classes_batch_size = config.sync.stage.classes_batch_size;

        let (mut pipeline, pipeline_handle) = Pipeline::new(storage_provider.clone(), chunk_size);

        // Configure pipeline
        pipeline.set_config(PipelineConfig {
            max_sync_tip: config.sync.max_tip,
            pruning: config.pruning.clone(),
        });

        let chain_id = match config.network {
            Network::Mainnet => katana_primitives::chain::ChainId::MAINNET,
            Network::Sepolia => katana_primitives::chain::ChainId::SEPOLIA,
        };

        let stages = &config.sync.stages;

        if let Some(SyncSource::JsonRpc(ref rpc_url)) = config.sync.source {
            let rpc_client = katana_starknet::rpc::StarknetRpcClient::new(rpc_url.clone());

            if stages.contains(&SyncStageKind::Blocks) {
                let block_downloader =
                    JsonRpcBlockDownloader::new_json_rpc(rpc_client.clone(), blocks_batch_size);
                pipeline.add_stage(Blocks::new(
                    storage_provider.clone(),
                    block_downloader,
                    chain_id,
                    task_spawner.clone(),
                ));
            }

            if stages.contains(&SyncStageKind::Classes) {
                let class_downloader = JsonRpcClassDownloader::new(rpc_client, classes_batch_size);
                pipeline.add_stage(Classes::new(storage_provider.clone(), class_downloader));
            }
        } else {
            if stages.contains(&SyncStageKind::Blocks) {
                let block_downloader =
                    BatchBlockDownloader::new_gateway(gateway_client.clone(), blocks_batch_size);
                pipeline.add_stage(Blocks::new(
                    storage_provider.clone(),
                    block_downloader,
                    chain_id,
                    task_spawner.clone(),
                ));
            }

            if stages.contains(&SyncStageKind::Classes) {
                let class_downloader =
                    GatewayClassDownloader::new(gateway_client.clone(), classes_batch_size);
                pipeline.add_stage(Classes::new(storage_provider.clone(), class_downloader));
            }
        }

        if stages.contains(&SyncStageKind::IndexHistory) {
            pipeline.add_stage(IndexHistory::new(storage_provider.clone(), task_spawner.clone()));
        }

        if stages.contains(&SyncStageKind::StateTrie) {
            pipeline.add_stage(StateTrie::new(storage_provider.clone(), task_spawner.clone()));
        }

        // -- build chain tip watcher using gateway client

        let chain_tip_watcher = ChainTipWatcher::new(gateway_client.clone());

        let preconf_factory = PreconfStateFactory::new(
            storage_provider.clone(),
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
        };

        let chain_spec = match config.network {
            Network::Mainnet => ChainSpec::mainnet(),
            Network::Sepolia => ChainSpec::sepolia(),
        };
        let chain_spec = Arc::new(chain_spec);

        let starknet_api = StarknetApi::new(
            chain_spec.clone(),
            pool.clone(),
            task_spawner.clone(),
            preconf_factory,
            GasPriceOracle::create_for_testing(),
            starknet_api_cfg,
            storage_provider.clone(),
            RpcCache::new(),
            ClassCache::new()?,
        );

        if config.rpc.apis.contains(&RpcModuleKind::Starknet) {
            #[cfg(feature = "explorer")]
            if config.rpc.explorer {
                use katana_rpc_api::starknet_ext::StarknetApiExtServer;
                rpc_modules.merge(StarknetApiExtServer::into_rpc(starknet_api.clone()))?;
            }

            rpc_modules.merge(StarknetApiServer::into_rpc(starknet_api.clone()))?;
            rpc_modules.merge(KatanaApiServer::into_rpc(starknet_api.clone()))?;
        }

        if config.rpc.apis.contains(&RpcModuleKind::TxPool) {
            use katana_rpc_api::txpool::TxPoolApiServer;
            let api = katana_rpc_server::txpool::TxPoolApi::new(pool.clone());
            rpc_modules.merge(TxPoolApiServer::into_rpc(api))?;
        }

        if config.rpc.apis.contains(&RpcModuleKind::Node) {
            let info = NodeInfo::from_parts(&config.build_info, chain_spec.as_ref());
            rpc_modules.merge(NodeApiServer::into_rpc(NodeApi::new(info)))?;
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

        Ok(Node {
            db,
            provider: storage_provider,
            pool,
            pipeline,
            rpc_server,
            gateway_server,
            task_manager,
            gateway_client,
            metrics_server,
            chain_tip_watcher,
            config: Arc::new(config),
        })
    }

    pub async fn launch(self) -> Result<LaunchedNode> {
        // --- start the metrics server (if configured)

        let metrics_handle = if let Some(ref server) = self.metrics_server {
            // safe to unwrap here because metrics_server can only be Some if the metrics config
            // exists
            let cfg = self.config.metrics.as_ref().expect("qed; must exist");
            let addr = cfg.socket_addr();
            Some(server.start(addr)?)
        } else {
            None
        };

        let chain_tip_watcher = self.chain_tip_watcher;
        let mut tip_subscription = chain_tip_watcher.subscribe();

        let pipeline_handle = self.pipeline.handle();
        let pipeline_handle_clone = pipeline_handle.clone();

        // -- start syncing pipeline task

        self.task_manager
            .task_spawner()
            .build_task()
            .graceful_shutdown()
            .name("Pipeline")
            .spawn(self.pipeline.into_future());

        // -- start chain tip watcher task

        self.task_manager
            .task_spawner()
            .build_task()
            .graceful_shutdown()
            .name("Chain tip watcher")
            .spawn(async move {
                loop {
                    if let Err(error) = chain_tip_watcher.run().await {
                        error!(%error, "Tip watcher failed. Restarting task.");
                    }
                }
            });

        // -- start a task for updating the pipeline's tip based on chain tip changes

        self.task_manager.task_spawner().spawn(async move {
            loop {
                match tip_subscription.changed().await {
                    Ok(new_tip) => pipeline_handle_clone.set_tip(new_tip),
                    Err(error) => {
                        error!(?error, "Error updating pipeline tip.");
                        break;
                    }
                }
            }
        });

        // --- start the rpc server

        let rpc = self.rpc_server.start(self.config.rpc.socket_addr()).await?;

        // --- start the feeder gateway server (if configured)

        let gateway_handle = match &self.gateway_server {
            Some(server) => {
                let config = self.config.gateway.as_ref().expect("qed; must exist");
                Some(server.start(config.socket_addr()).await?)
            }
            None => None,
        };

        Ok(LaunchedNode {
            db: self.db,
            config: self.config,
            task_manager: self.task_manager,
            pipeline: pipeline_handle,
            metrics: metrics_handle,
            rpc,
            gateway: gateway_handle,
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
    /// Handle to the gateway server (if enabled).
    pub gateway: Option<GatewayServerHandle>,
    /// Handle to the metrics server (if enabled).
    pub metrics: Option<MetricsServerHandle>,
}

impl LaunchedNode {
    pub async fn stop(&self) -> Result<()> {
        self.rpc.stop()?;

        // Stop feeder gateway server if it's running
        if let Some(handle) = &self.gateway {
            handle.stop()?;
        }

        self.pipeline.stop();

        self.pipeline.stopped().await;
        self.task_manager.shutdown().await;

        Ok(())
    }

    pub fn stopped(&self) -> NodeStoppedFuture<'_> {
        NodeStoppedFuture::new(self)
    }
}
