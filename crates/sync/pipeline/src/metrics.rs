//! Metrics for the sync pipeline.
//!
//! This module provides metrics collection for the synchronization pipeline,
//! enabling monitoring and visualization of both individual stages and overall pipeline progress.
//!
//! ## Pipeline Metrics
//!
//! Pipeline-level metrics track the overall synchronization process:
//!
//! - Current sync target (tip block)
//! - Current sync position (lowest checkpoint across all stages)
//! - Iteration duration
//! - Error count
//!
//! ## Stage Metrics
//!
//! Stage-level metrics are collected per stage and include:
//!
//! - Blocks processed by each stage
//! - Execution duration for each stage
//! - Current checkpoint for each stage
//! - Error count for each stage
//!
//! ## Derived Metrics
//!
//! From these base metrics, the following can be derived:
//!
//! - **Sync progress**: `sync_position / sync_target`
//! - **Blocks behind**: `sync_target - sync_position`
//! - **Stage progress**: `stage.checkpoint / sync_target`
//! - **Stage throughput**: `rate(stage.blocks_processed_total)`

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use katana_metrics::metrics::{self, Counter, Gauge, Histogram};
use katana_metrics::Metrics;
use parking_lot::Mutex;

/// Metrics for the sync pipeline.
#[allow(missing_debug_implementations)]
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
        let mut stages = self.inner.stages.lock();
        stages
            .entry(stage_id)
            .or_insert_with(|| StageMetrics::new_with_labels(&[("stage", stage_id)]))
            .clone()
    }

    /// Update the sync target (tip block).
    pub fn set_sync_target(&self, tip: u64) {
        self.inner.pipeline.sync_target.set(tip as f64);
    }

    /// Update the sync position (lowest checkpoint across all stages).
    pub fn set_sync_position(&self, position: u64) {
        self.inner.pipeline.sync_position.set(position as f64);
    }

    /// Record the duration of a pipeline iteration.
    pub fn record_iteration_duration(&self, duration_seconds: f64) {
        self.inner.pipeline.iteration_duration_seconds.record(duration_seconds);
    }

    /// Record a pipeline error.
    pub fn record_error(&self) {
        self.inner.pipeline.errors_total.increment(1);
    }
}

impl Default for PipelineMetrics {
    fn default() -> Self {
        Self::new()
    }
}

#[allow(missing_debug_implementations)]
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
    /// Target tip block being synced to
    sync_target: Gauge,
    /// Current fully-synced position (minimum checkpoint across all stages)
    sync_position: Gauge,
    /// Duration of each pipeline iteration
    iteration_duration_seconds: Histogram,
    /// Total number of pipeline errors
    errors_total: Counter,
}

/// Metrics for individual stage execution.
#[derive(Metrics, Clone)]
#[metrics(scope = "sync.stage")]
pub struct StageMetrics {
    /// Current checkpoint for this stage
    checkpoint: Gauge,
    /// Total number of blocks processed by this stage
    blocks_processed_total: Counter,
    /// Duration of each stage execution
    execution_duration_seconds: Histogram,
    /// Total number of errors encountered by this stage
    errors_total: Counter,
}

impl StageMetrics {
    /// Record a stage execution starting. Returns a guard that records
    /// the execution duration when dropped.
    pub fn execution_started(&self) -> StageExecutionGuard {
        StageExecutionGuard { metrics: self.clone(), started_at: Instant::now() }
    }

    /// Record blocks processed by this stage.
    pub fn record_blocks_processed(&self, count: u64) {
        self.blocks_processed_total.increment(count);
    }

    /// Update the checkpoint for this stage.
    pub fn set_checkpoint(&self, checkpoint: u64) {
        self.checkpoint.set(checkpoint as f64);
    }

    /// Record a stage error.
    pub fn record_error(&self) {
        self.errors_total.increment(1);
    }
}

/// Guard that records the execution duration when dropped.
#[allow(missing_debug_implementations)]
pub struct StageExecutionGuard {
    metrics: StageMetrics,
    started_at: Instant,
}

impl Drop for StageExecutionGuard {
    fn drop(&mut self) {
        let duration = self.started_at.elapsed().as_secs_f64();
        self.metrics.execution_duration_seconds.record(duration);
    }
}
