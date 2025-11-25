//! Metrics for the sync pipeline.
//!
//! This module provides comprehensive metrics collection for the synchronization pipeline,
//! enabling monitoring and visualization of both individual stages and overall pipeline progress.
//!
//! ## Pipeline Metrics
//!
//! Pipeline-level metrics track the overall synchronization process:
//!
//! - Total chunks processed across all stages
//! - Total blocks processed across all pipeline runs
//! - Total time spent syncing
//! - Current tip block being synced to
//! - Pipeline runs completed
//!
//! ## Stage Metrics
//!
//! Stage-level metrics are collected per stage and include:
//!
//! - Number of executions for each stage
//! - Total blocks processed by each stage
//! - Execution time for each stage execution
//! - Checkpoint updates for each stage

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use katana_metrics::metrics::{self, Counter, Gauge, Histogram};
use katana_metrics::Metrics;
use parking_lot::Mutex;

/// Metrics for the sync pipeline.
#[derive(Clone)]
pub struct PipelineMetrics {
    inner: Arc<PipelineMetricsInner>,
}

impl PipelineMetrics {
    /// Creates a new instance of `PipelineMetrics`.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(PipelineMetricsInner {
                pipeline: PipelineOverallMetrics::default(),
                stages: Default::default(),
            }),
        }
    }

    /// Get or create metrics for a specific stage.
    pub fn stage(&self, stage_id: &'static str) -> StageMetrics {
        let mut stages = self.inner.stages.lock().unwrap();
        stages
            .entry(stage_id)
            .or_insert_with(|| StageMetrics::new_with_labels(&[("stage", stage_id)]))
            .clone()
    }

    /// Record a chunk being processed by the pipeline.
    pub fn record_chunk(&self, blocks_in_chunk: u64) {
        self.inner.pipeline.chunks_processed_total.increment(1);
        self.inner.pipeline.blocks_processed_total.increment(blocks_in_chunk);
    }

    /// Update the current tip being synced to.
    pub fn set_tip(&self, tip: u64) {
        self.inner.pipeline.current_tip.set(tip as f64);
    }

    /// Record a pipeline run completing.
    pub fn record_run_complete(&self) {
        self.inner.pipeline.runs_completed_total.increment(1);
    }

    /// Record the time taken for a pipeline iteration.
    pub fn record_iteration_time(&self, duration_seconds: f64) {
        self.inner.pipeline.iteration_time_seconds.record(duration_seconds);
    }

    /// Update the lowest checkpoint across all stages.
    pub fn set_lowest_checkpoint(&self, checkpoint: u64) {
        self.inner.pipeline.lowest_checkpoint.set(checkpoint as f64);
    }

    /// Update the highest checkpoint across all stages.
    pub fn set_highest_checkpoint(&self, checkpoint: u64) {
        self.inner.pipeline.highest_checkpoint.set(checkpoint as f64);
    }
}

impl Default for PipelineMetrics {
    fn default() -> Self {
        Self::new()
    }
}

struct PipelineMetricsInner {
    /// Overall pipeline metrics
    pipeline: PipelineOverallMetrics,
    /// Per-stage metrics
    stages: Mutex<HashMap<&'static str, StageMetrics>>,
}

/// Metrics for the overall pipeline execution.
#[derive(Metrics, Clone)]
#[metrics(scope = "sync.pipeline")]
struct PipelineOverallMetrics {
    /// Total number of chunks processed by the pipeline
    chunks_processed_total: Counter,
    /// Total number of blocks processed by the pipeline
    blocks_processed_total: Counter,
    /// Total number of pipeline runs completed
    runs_completed_total: Counter,
    /// Current tip block being synced to
    current_tip: Gauge,
    /// Lowest checkpoint across all stages
    lowest_checkpoint: Gauge,
    /// Highest checkpoint across all stages
    highest_checkpoint: Gauge,
    /// Time taken for each pipeline iteration
    iteration_time_seconds: Histogram,
}

/// Metrics for individual stage execution.
#[derive(Metrics, Clone)]
#[metrics(scope = "sync.stage")]
pub struct StageMetrics {
    /// Number of times the stage has been executed
    executions_total: Counter,
    /// Total number of blocks processed by this stage
    blocks_processed_total: Counter,
    /// Number of times the stage was skipped (checkpoint >= target)
    skipped_total: Counter,
    /// Time taken for each stage execution
    execution_time_seconds: Histogram,
    /// Current checkpoint for this stage
    checkpoint: Gauge,
}

impl StageMetrics {
    /// Record a stage execution starting.
    pub fn execution_started(&self) -> StageExecutionGuard {
        self.executions_total.increment(1);
        StageExecutionGuard { metrics: self.clone(), started_at: Instant::now() }
    }

    /// Record blocks processed by this stage.
    pub fn record_blocks_processed(&self, count: u64) {
        self.blocks_processed_total.increment(count);
    }

    /// Record a stage being skipped.
    pub fn record_skipped(&self) {
        self.skipped_total.increment(1);
    }

    /// Update the checkpoint for this stage.
    pub fn set_checkpoint(&self, checkpoint: u64) {
        self.checkpoint.set(checkpoint as f64);
    }
}

/// Guard that records the execution time when dropped.
pub struct StageExecutionGuard {
    metrics: StageMetrics,
    started_at: Instant,
}

impl Drop for StageExecutionGuard {
    fn drop(&mut self) {
        let duration = self.started_at.elapsed().as_secs_f64();
        self.metrics.execution_time_seconds.record(duration);
    }
}
