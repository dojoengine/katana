//! `persistent-tee tee start` — runs the TEE pipeline end-to-end.

use std::{path::PathBuf, time::Duration};

use anyhow::Result;
use clap::{Parser, Subcommand};
use saya_core::{
    block_ingestor::BatchingPollingBlockIngestorBuilder, orchestrator::TeeOrchestratorBuilder,
    service::Daemon, storage::SqliteDb,
};

use crate::settlement::TeePiltoverSettlementBackendBuilder;
use starknet_types_core::felt::Felt;
use url::Url;

use crate::attestor::TeeAttestorBuilder;
use crate::common::SAYA_DB_PATH;
use crate::prover::TeeProverBuilder;

/// 10 seconds.
const GRACEFUL_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);

/// Default attestor poll interval.
const DEFAULT_ATTESTOR_POLL_INTERVAL_MS: u64 = 1_000;

#[derive(Debug, Parser)]
pub struct Tee {
    #[clap(subcommand)]
    command: Subcommands,
}

#[derive(Debug, Subcommand)]
enum Subcommands {
    /// Start Saya in TEE mode.
    Start(Start),
}

#[derive(Debug, Parser, Clone)]
struct Start {
    /// Rollup network Starknet JSON-RPC URL (v0.7.1)
    #[clap(long, env)]
    rollup_rpc: Url,
    /// Settlement network Starknet JSON-RPC URL (v0.7.1)
    #[clap(long, env)]
    settlement_rpc: Url,
    /// Settlement network piltover contract address
    #[clap(long, env)]
    settlement_piltover_address: Felt,
    /// Settlement network account contract address
    #[clap(long, env)]
    settlement_account_address: Felt,
    /// Settlement network account private key
    #[clap(long, env)]
    settlement_account_private_key: Felt,
    /// TEE registry contract address on the prover network
    #[clap(long, env)]
    tee_registry_address: Felt,
    /// Private key for the prover network account
    #[clap(long, env)]
    prover_private_key: String,
    /// Attestor poll interval in milliseconds
    #[clap(long, env, default_value_t = DEFAULT_ATTESTOR_POLL_INTERVAL_MS)]
    attestor_poll_interval_ms: u64,
    /// Number of blocks to accumulate per TEE attestation batch
    #[clap(long, env, default_value_t = 10)]
    batch_size: usize,
    /// Flush a partial batch after this many seconds without a new block
    #[clap(long, env, default_value_t = 120)]
    idle_timeout_secs: u64,
    /// Path to the database directory
    #[clap(long, env)]
    db_dir: Option<PathBuf>,
    /// Skip the entire SP1 / AMD KDS / cert chain pipeline and synthesize a
    /// stub `VerifierJournal` whose `report_data` encodes the Poseidon
    /// commitment Piltover would otherwise extract from a real attestation.
    ///
    /// Requires the on-chain TEE registry pointed at by
    /// `--tee-registry-address` to be a permissive `IAMDTeeRegistry` mock
    /// (e.g. `piltover_mock_amd_tee_registry`, deployable via
    /// `saya-ops core-contract declare-and-deploy-tee-registry-mock`).
    ///
    /// Intended exclusively for end-to-end tests on machines without AMD
    /// SEV-SNP hardware. Do not use in production.
    #[clap(long, env)]
    mock_prove: bool,
}

impl Tee {
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

        let block_ingestor_builder = BatchingPollingBlockIngestorBuilder::new(
            self.rollup_rpc.clone(),
            db.clone(),
            self.batch_size,
            Duration::from_secs(self.idle_timeout_secs),
        );

        let attestor_builder = TeeAttestorBuilder::new(
            self.rollup_rpc.clone(),
            Duration::from_millis(self.attestor_poll_interval_ms),
        );

        let prover_builder = TeeProverBuilder::new(
            self.settlement_rpc.to_string(),
            self.tee_registry_address,
            self.prover_private_key,
            self.mock_prove,
        );

        let settlement_builder = TeePiltoverSettlementBackendBuilder::new(
            self.settlement_rpc,
            self.settlement_piltover_address,
            self.settlement_account_address,
            self.settlement_account_private_key,
            self.mock_prove,
        );

        let orchestrator = TeeOrchestratorBuilder::new(
            block_ingestor_builder,
            attestor_builder,
            prover_builder,
            settlement_builder,
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
