use std::{path::PathBuf, time::Duration};

use anyhow::Result;
use clap::{Parser, Subcommand};
use generate_pie::types::OsHintsConfiguration;
use saya_core::{
    block_ingestor::PollingBlockIngestorBuilder,
    data_availability::CelestiaDataAvailabilityBackendBuilder,
    orchestrator::Genesis,
    prover::{BlockOrdererBuilder, PipelineChainBuilder},
    service::Daemon,
    storage::{InMemoryStorageBackend, SqliteDb},
    ChainId,
};

use crate::{
    atlantic::AtlanticSnosProverBuilder,
    common::{calculate_workers_per_stage, SAYA_DB_PATH},
    orchestrator::SovereignOrchestratorBuilder,
    snos_pie_generator::SnosPieGeneratorBuilder,
};
use starknet::{
    core::utils::parse_cairo_short_string,
    providers::{jsonrpc::HttpTransport, JsonRpcClient, Provider},
};
use url::Url;

/// 10 seconds.
const GRACEFUL_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Parser)]
pub struct Sovereign {
    #[clap(subcommand)]
    command: Subcommands,
}

#[derive(Debug, Subcommand)]
enum Subcommands {
    /// Start Saya in sovereign mode.
    Start(Start),
}

#[derive(Debug, Parser)]
struct Start {
    /// Starknet JSON-RPC URL (v0.7.1)
    #[clap(long, env)]
    starknet_rpc: Url,
    /// Whether to mock the SNOS proof by extracting the output from the PIE and using it from a proof.
    #[clap(long)]
    mock_snos_from_pie: bool,
    /// Atlantic prover API key
    #[clap(long, env)]
    atlantic_key: String,
    /// Celestia RPC endpoint URL
    #[clap(long, env)]
    celestia_rpc: Url,
    /// Celestia RPC node auth token
    #[clap(long, env)]
    celestia_token: String,
    /// Celestia key name
    #[clap(long, env)]
    celestia_key_name: Option<String>,
    /// Celestia namespace
    #[clap(long, env)]
    #[clap(default_value = "sayaproofs")]
    #[clap(value_parser = validate_non_empty)]
    celestia_namespace: String,
    /// Genesis options
    #[clap(flatten)]
    genesis: GenesisOptions,
    /// Number of blocks to process in parallel
    #[clap(long, env)]
    blocks_processed_in_parallel: usize,
    /// Path to the database directory
    #[clap(long, env)]
    db_dir: Option<PathBuf>,
}

/// Validate that the value is not empty.
pub fn validate_non_empty(s: &str) -> Result<String, String> {
    if s.trim().is_empty() {
        Err("Value cannot be empty".to_string())
    } else {
        Ok(s.to_string())
    }
}

#[derive(Debug, Parser)]
struct GenesisOptions {
    #[clap(
        long = "genesis.first-block-number",
        env = "GENESIS_FIRST_BLOCK_NUMBER"
    )]
    first_block_number: Option<u64>,
}

impl Sovereign {
    pub async fn run(self) -> Result<()> {
        match self.command {
            Subcommands::Start(start) => start.run().await,
        }
    }
}

impl Start {
    pub async fn run(self) -> Result<()> {
        let saya_path = self
            .db_dir
            .map(|db_dir| format!("{}/{}", db_dir.display(), SAYA_DB_PATH))
            .unwrap_or_else(|| SAYA_DB_PATH.to_string());
        let db = SqliteDb::new(&saya_path).await?;

        let workers_distribution: [usize; 3] =
            calculate_workers_per_stage(self.blocks_processed_in_parallel);
        let [snos_worker_count, _layout_bridge_workers_count, ingestor_worker_count] =
            workers_distribution;

        let chain_id = parse_cairo_short_string(
            &JsonRpcClient::new(HttpTransport::new(self.starknet_rpc.clone()))
                .chain_id()
                .await?,
        )?;

        let block_ingestor_builder = PollingBlockIngestorBuilder::new(
            self.starknet_rpc.clone(),
            db.clone(),
            ingestor_worker_count,
        );

        let pie_gen_builder = SnosPieGeneratorBuilder::new(
            self.starknet_rpc,
            db.clone(),
            ingestor_worker_count,
            OsHintsConfiguration {
                debug_mode: false,
                full_output: false,
                use_kzg_da: false,
            },
            ChainId::Other(chain_id),
        );

        let pipeline_builder = PipelineChainBuilder::new(
            PipelineChainBuilder::new(
                pie_gen_builder,
                AtlanticSnosProverBuilder::new(
                    self.atlantic_key,
                    self.mock_snos_from_pie,
                    db.clone(),
                    snos_worker_count,
                ),
            ),
            BlockOrdererBuilder::new(),
        );
        let da_builder = CelestiaDataAvailabilityBackendBuilder::new(
            self.celestia_rpc,
            self.celestia_token,
            self.celestia_namespace,
            self.celestia_key_name,
        )?;
        let storage = InMemoryStorageBackend::new();

        let orchestrator = SovereignOrchestratorBuilder::new(
            block_ingestor_builder,
            pipeline_builder,
            da_builder,
            storage,
            self.genesis.into(),
        )
        .build()
        .await?;
        let orchestrator_shutdown = orchestrator.shutdown_handle();
        orchestrator.start();

        let mut sigterm_handle =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        let ctrl_c_handle = tokio::signal::ctrl_c();

        tokio::select! {
            _ = sigterm_handle.recv() => {},
            _ = ctrl_c_handle => {},
            _ = orchestrator_shutdown.finished() => {},
        }

        // Graceful shutdown
        orchestrator_shutdown.shutdown();
        tokio::select! {
            _ = tokio::time::sleep(GRACEFUL_SHUTDOWN_TIMEOUT) => {
                Err(anyhow::anyhow!("timeout waiting for graceful shutdown"))
            },
            _ = orchestrator_shutdown.finished() => {
                Ok(())
            },
        }
    }
}

impl From<GenesisOptions> for Option<Genesis> {
    fn from(value: GenesisOptions) -> Self {
        value.first_block_number.map(|num| Genesis {
            first_block_number: num,
        })
    }
}
