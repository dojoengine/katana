use std::future::pending;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::anyhow;
use futures::future::BoxFuture;
use katana_pipeline::Pipeline;
use katana_primitives::block::BlockNumber;
use katana_provider::api::stage::StageCheckpointProvider;
use katana_provider::test_utils::test_provider;
use katana_provider::{MutableProvider, ProviderFactory};
use katana_stage::{
    PruneInput, PruneOutput, PruneResult, Stage, StageExecutionInput, StageExecutionOutput,
    StageResult,
};

/// Simple mock stage that does nothing
struct MockStage;

impl Stage for MockStage {
    fn id(&self) -> &'static str {
        "Mock"
    }

    fn execute<'a>(&'a mut self, input: &'a StageExecutionInput) -> BoxFuture<'a, StageResult> {
        Box::pin(async move { Ok(StageExecutionOutput { last_block_processed: input.to() }) })
    }

    fn prune<'a>(&'a mut self, input: &'a PruneInput) -> BoxFuture<'a, PruneResult> {
        let _ = input;
        Box::pin(async move { Ok(PruneOutput::default()) })
    }
}

/// Tracks execution calls with their inputs
#[derive(Debug, Clone)]
struct ExecutionRecord {
    from: BlockNumber,
    to: BlockNumber,
}

/// Mock stage that tracks execution
#[derive(Debug, Clone)]
struct TrackingStage {
    id: &'static str,
    /// Used to tracks how many times the stage has been executed
    executions: Arc<Mutex<Vec<ExecutionRecord>>>,
}

impl TrackingStage {
    fn new(id: &'static str) -> Self {
        Self { id, executions: Arc::new(Mutex::new(Vec::new())) }
    }

    fn executions(&self) -> Vec<ExecutionRecord> {
        self.executions.lock().unwrap().clone()
    }

    fn execution_count(&self) -> usize {
        self.executions.lock().unwrap().len()
    }
}

impl Stage for TrackingStage {
    fn id(&self) -> &'static str {
        self.id
    }

    fn execute<'a>(&'a mut self, input: &'a StageExecutionInput) -> BoxFuture<'a, StageResult> {
        Box::pin(async move {
            self.executions
                .lock()
                .unwrap()
                .push(ExecutionRecord { from: input.from(), to: input.to() });

            Ok(StageExecutionOutput { last_block_processed: input.to() })
        })
    }

    fn prune<'a>(&'a mut self, input: &'a PruneInput) -> BoxFuture<'a, PruneResult> {
        let _ = input;
        Box::pin(async move { Ok(PruneOutput::default()) })
    }
}

/// Mock stage that fails on execution
#[derive(Debug, Clone)]
struct FailingStage {
    id: &'static str,
}

impl FailingStage {
    fn new(id: &'static str) -> Self {
        Self { id }
    }
}

impl Stage for FailingStage {
    fn id(&self) -> &'static str {
        self.id
    }

    fn execute<'a>(&'a mut self, _: &'a StageExecutionInput) -> BoxFuture<'a, StageResult> {
        Box::pin(async { Err(katana_stage::Error::Other(anyhow!("Stage execution failed"))) })
    }

    fn prune<'a>(&'a mut self, input: &'a PruneInput) -> BoxFuture<'a, PruneResult> {
        let _ = input;
        Box::pin(async move { Ok(PruneOutput::default()) })
    }
}

/// Mock stage that always reports a fixed `last_block_processed`.
#[derive(Debug, Clone)]
struct FixedOutputStage {
    id: &'static str,
    last_block_processed: BlockNumber,
    executions: Arc<Mutex<Vec<ExecutionRecord>>>,
}

impl FixedOutputStage {
    fn new(id: &'static str, last_block_processed: BlockNumber) -> Self {
        Self { id, last_block_processed, executions: Arc::new(Mutex::new(Vec::new())) }
    }

    fn execution_count(&self) -> usize {
        self.executions.lock().unwrap().len()
    }
}

impl Stage for FixedOutputStage {
    fn id(&self) -> &'static str {
        self.id
    }

    fn execute<'a>(&'a mut self, input: &'a StageExecutionInput) -> BoxFuture<'a, StageResult> {
        let executions = self.executions.clone();
        let last_block_processed = self.last_block_processed;

        Box::pin(async move {
            executions.lock().unwrap().push(ExecutionRecord { from: input.from(), to: input.to() });

            assert!(
                last_block_processed <= input.to(),
                "Configured last block {last_block_processed} exceeds the provided end block {}",
                input.to()
            );

            Ok(StageExecutionOutput { last_block_processed })
        })
    }

    fn prune<'a>(&'a mut self, input: &'a PruneInput) -> BoxFuture<'a, PruneResult> {
        let _ = input;
        Box::pin(async move { Ok(PruneOutput::default()) })
    }
}

// ============================================================================
// run_to() - Single Stage Tests
// ============================================================================

#[tokio::test]
async fn run_to_executes_stage_to_target() {
    let provider_factory = test_provider();
    let (mut pipeline, handle) = Pipeline::new(provider_factory.clone(), 10);

    let stage = TrackingStage::new("Stage1");
    let stage_clone = stage.clone();

    pipeline.add_stage(stage);
    handle.set_tip(5);
    let result = pipeline.execute(5).await.unwrap();

    let provider = provider_factory.provider_mut();
    assert_eq!(result, 5);
    assert_eq!(provider.execution_checkpoint(stage_clone.id()).unwrap(), Some(5));

    let execs = stage_clone.executions();
    assert_eq!(execs.len(), 1);
    assert_eq!(execs[0].from, 0); // checkpoint was 0, so from = 0
    assert_eq!(execs[0].to, 5);
}

#[tokio::test]
async fn run_to_skips_stage_when_checkpoint_equals_target() {
    let provider_factory = test_provider();
    let (mut pipeline, handle) = Pipeline::new(provider_factory.clone(), 10);

    let stage = TrackingStage::new("Stage1");
    let stage_clone = stage.clone();

    // Set initial checkpoint
    let provider = provider_factory.provider_mut();
    provider.set_execution_checkpoint(stage.id(), 5).unwrap();
    provider.commit().unwrap();

    pipeline.add_stage(stage);

    handle.set_tip(5);
    let result = pipeline.execute(5).await.unwrap();

    assert_eq!(result, 5);
    assert_eq!(stage_clone.executions().len(), 0); // Not executed
}

#[tokio::test]
async fn run_to_skips_stage_when_checkpoint_exceeds_target() {
    let provider_factory = test_provider();
    let (mut pipeline, handle) = Pipeline::new(provider_factory.clone(), 10);

    let stage = TrackingStage::new("Stage1");
    let stage_clone = stage.clone();

    // Set checkpoint beyond target
    let provider = provider_factory.provider_mut();
    provider.set_execution_checkpoint("Stage1", 10).unwrap();
    provider.commit().unwrap();

    pipeline.add_stage(stage);

    handle.set_tip(10);
    let result = pipeline.execute(5).await.unwrap();

    assert_eq!(result, 10); // Returns the checkpoint
    assert_eq!(stage_clone.executions().len(), 0); // Not executed
}

#[tokio::test]
async fn run_to_uses_checkpoint_plus_one_as_from() {
    let provider_factory = test_provider();
    let (mut pipeline, handle) = Pipeline::new(provider_factory.clone(), 10);

    let stage = TrackingStage::new("Stage1");
    let stage_clone = stage.clone();

    // Set checkpoint to 3
    let provider = provider_factory.provider_mut();
    provider.set_execution_checkpoint(stage.id(), 3).unwrap();
    provider.commit().unwrap();

    pipeline.add_stage(stage);
    handle.set_tip(10);
    pipeline.execute(10).await.unwrap();

    let execs = stage_clone.executions();
    assert_eq!(execs.len(), 1);

    // stage execution from block 4 (block after the checkpoint) to 10
    assert_eq!(execs[0].from, 4); // 3 + 1
    assert_eq!(execs[0].to, 10);
}

// ============================================================================
// run_to() - Multiple Stages Tests
// ============================================================================

#[tokio::test]
async fn run_to_executes_all_stages_in_order() {
    let provider_factory = test_provider();
    let (mut pipeline, handle) = Pipeline::new(provider_factory.clone(), 10);

    let stage1 = TrackingStage::new("Stage1");
    let stage2 = TrackingStage::new("Stage2");
    let stage3 = TrackingStage::new("Stage3");

    let stage1_clone = stage1.clone();
    let stage2_clone = stage2.clone();
    let stage3_clone = stage3.clone();

    pipeline.add_stages([
        Box::new(stage1) as Box<dyn Stage>,
        Box::new(stage2) as Box<dyn Stage>,
        Box::new(stage3) as Box<dyn Stage>,
    ]);

    handle.set_tip(5);
    pipeline.execute(5).await.unwrap();

    // All stages should be executed once because the tip is 5 and the chunk size is 10
    assert_eq!(stage1_clone.execution_count(), 1);
    assert_eq!(stage2_clone.execution_count(), 1);
    assert_eq!(stage3_clone.execution_count(), 1);

    // All checkpoints should be set
    let provider = provider_factory.provider_mut();
    assert_eq!(provider.execution_checkpoint(stage1_clone.id()).unwrap(), Some(5));
    assert_eq!(provider.execution_checkpoint(stage2_clone.id()).unwrap(), Some(5));
    assert_eq!(provider.execution_checkpoint(stage3_clone.id()).unwrap(), Some(5));
}

#[tokio::test]
async fn run_to_with_mixed_checkpoints() {
    let provider_factory = test_provider();
    let (mut pipeline, handle) = Pipeline::new(provider_factory.clone(), 10);

    let stage1 = TrackingStage::new("Stage1");
    let stage2 = TrackingStage::new("Stage2");
    let stage3 = TrackingStage::new("Stage3");

    let stage1_clone = stage1.clone();
    let stage2_clone = stage2.clone();
    let stage3_clone = stage3.clone();

    pipeline.add_stages([
        Box::new(stage1) as Box<dyn Stage>,
        Box::new(stage2) as Box<dyn Stage>,
        Box::new(stage3) as Box<dyn Stage>,
    ]);

    let provider = provider_factory.provider_mut();
    // Stage1 already at checkpoint 10 (should skip)
    provider.set_execution_checkpoint(stage1_clone.id(), 10).unwrap();
    // Stage2 at checkpoint 3 (should execute)
    provider.set_execution_checkpoint(stage2_clone.id(), 3).unwrap();
    provider.commit().unwrap();

    handle.set_tip(10);
    pipeline.execute(10).await.unwrap();

    // Stage1 should be skipped because its checkpoint (10) >= than the tip (10)
    assert_eq!(stage1_clone.execution_count(), 0);

    // Stage2 should be executed once from 4 to 10 because its checkpoint (3) < tip (10)
    let e2 = stage2_clone.executions();
    assert_eq!(e2.len(), 1);
    assert_eq!(e2[0].from, 4);
    assert_eq!(e2[0].to, 10);

    // Stage3 should be executed once from 0 to 10 because it has no checkpoint (0) < tip (10)
    let e3 = stage3_clone.executions();
    assert_eq!(e3.len(), 1);
    assert_eq!(e3[0].from, 0);
    assert_eq!(e3[0].to, 10);
}

#[tokio::test]
async fn run_to_returns_minimum_last_block_processed() {
    let provider_factory = test_provider();
    let (mut pipeline, handle) = Pipeline::new(provider_factory.clone(), 10);

    let stage1 = FixedOutputStage::new("Stage1", 10);
    let stage2 = FixedOutputStage::new("Stage2", 5);
    let stage3 = FixedOutputStage::new("Stage3", 20);

    let stage1_clone = stage1.clone();
    let stage2_clone = stage2.clone();
    let stage3_clone = stage3.clone();

    pipeline.add_stages([
        Box::new(stage1) as Box<dyn Stage>,
        Box::new(stage2) as Box<dyn Stage>,
        Box::new(stage3) as Box<dyn Stage>,
    ]);

    handle.set_tip(20);
    let result = pipeline.execute(20).await.unwrap();

    // make sure that all the stages were executed once
    assert_eq!(stage1_clone.execution_count(), 1);
    assert_eq!(stage2_clone.execution_count(), 1);
    assert_eq!(stage3_clone.execution_count(), 1);

    let provider = provider_factory.provider_mut();
    assert_eq!(result, 5);
    assert_eq!(provider.execution_checkpoint(stage1_clone.id()).unwrap(), Some(10));
    assert_eq!(provider.execution_checkpoint(stage2_clone.id()).unwrap(), Some(5));
    assert_eq!(provider.execution_checkpoint(stage3_clone.id()).unwrap(), Some(20));
}

#[tokio::test]
async fn run_to_middle_stage_skip_continues() {
    let provider_factory = test_provider();
    let (mut pipeline, handle) = Pipeline::new(provider_factory.clone(), 10);

    let stage1 = TrackingStage::new("Stage1");
    let stage2 = TrackingStage::new("Stage2");
    let stage3 = TrackingStage::new("Stage3");

    let stage1_clone = stage1.clone();
    let stage2_clone = stage2.clone();
    let stage3_clone = stage3.clone();

    pipeline.add_stages([
        Box::new(stage1) as Box<dyn Stage>,
        Box::new(stage2) as Box<dyn Stage>,
        Box::new(stage3) as Box<dyn Stage>,
    ]);

    // stage in the middle of the sequence already complete
    let provider = provider_factory.provider_mut();
    provider.set_execution_checkpoint(stage2_clone.id(), 10).unwrap();
    provider.commit().unwrap();

    handle.set_tip(10);
    pipeline.execute(10).await.unwrap();

    // Stage1 and Stage3 should execute
    assert_eq!(stage1_clone.execution_count(), 1);
    assert_eq!(stage2_clone.execution_count(), 0); // Skipped
    assert_eq!(stage3_clone.execution_count(), 1);
}

// ============================================================================
// run() Loop - Tip Processing Tests
// ============================================================================

#[tokio::test]
async fn run_processes_single_chunk_to_tip() {
    let provider_factory = test_provider();

    let (mut pipeline, handle) = Pipeline::new(provider_factory.clone(), 100);

    let stage = TrackingStage::new("Stage1");
    let stage_clone = stage.clone();

    pipeline.add_stage(stage);

    // Set tip to 50 (within one chunk)
    handle.set_tip(50);

    let task_handle = tokio::spawn(async move { pipeline.run().await });
    tokio::time::sleep(Duration::from_millis(100)).await;

    handle.stop();

    let result = task_handle.await.unwrap();
    assert!(result.is_ok());

    // Stage1 should be executed once from 0 to 50 because it's within a pipeline chunk (100)
    let execs = stage_clone.executions();
    assert_eq!(execs.len(), 1);
    assert_eq!(execs[0].from, 0);
    assert_eq!(execs[0].to, 50);

    assert_eq!(provider_factory.provider_mut().execution_checkpoint("Stage1").unwrap(), Some(50));
}

#[tokio::test]
async fn run_processes_multiple_chunks_to_tip() {
    let provider_factory = test_provider();
    let (mut pipeline, handle) = Pipeline::new(provider_factory.clone(), 10); // Small chunk size

    let stage = TrackingStage::new("Stage1");
    let stage_clone = stage.clone();

    pipeline.add_stage(stage);

    // Set tip to 25 (requires 3 chunks: 10, 20, 25)
    handle.set_tip(25);

    let pipeline_handle = tokio::spawn(async move { pipeline.run().await });
    tokio::time::sleep(Duration::from_millis(100)).await;

    handle.stop();

    let result = pipeline_handle.await.unwrap();
    assert!(result.is_ok());

    // Should execute 3 times:
    // * 1st chunk: 0-10
    // * 2nd chunk: 11-20
    // * 3rd chunk: 21-25

    let execs = stage_clone.executions();
    assert_eq!(execs.len(), 3);

    assert_eq!(execs[0].from, 0);
    assert_eq!(execs[0].to, 10);

    assert_eq!(execs[1].from, 11);
    assert_eq!(execs[1].to, 20);

    assert_eq!(execs[2].from, 21);
    assert_eq!(execs[2].to, 25);
}

#[tokio::test]
async fn run_processes_new_tip_after_completing_previous() {
    let provider_factory = test_provider();
    let (mut pipeline, handle) = Pipeline::new(provider_factory.clone(), 10);

    let stage = TrackingStage::new("Stage1");
    let executions = stage.executions.clone();
    pipeline.add_stage(stage);

    // Set initial tip
    handle.set_tip(10);

    let task_handle = tokio::spawn(async move { pipeline.run().await });

    // Wait for first tip to process
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Set new tip
    handle.set_tip(25);

    // Wait for second tip to process
    tokio::time::sleep(Duration::from_millis(100)).await;

    handle.stop();
    let result = task_handle.await.unwrap();
    assert!(result.is_ok());

    // Should have processed both tips
    let execs = executions.lock().unwrap();
    assert!(execs.len() >= 3); // 1-10, 11-20, 21-25
    let provider = provider_factory.provider_mut();
    assert_eq!(provider.execution_checkpoint("Stage1").unwrap(), Some(25));
}

/// This test ensures that the pipeline will immediately stop its execution if the stop signal
/// is received - the pipeline should not get blocked by the main execution loop on receiving
/// signals.
#[tokio::test]
async fn run_should_be_cancelled_if_stop_requested() {
    #[derive(Default, Clone)]
    struct PendingStage {
        executed: Arc<Mutex<bool>>,
    }

    impl Stage for PendingStage {
        fn id(&self) -> &'static str {
            "Pending"
        }

        fn execute<'a>(&'a mut self, _: &'a StageExecutionInput) -> BoxFuture<'a, StageResult> {
            Box::pin(async {
                let () = pending().await;
                *self.executed.lock().unwrap() = true;
                Ok(StageExecutionOutput { last_block_processed: 100 })
            })
        }

        fn prune<'a>(&'a mut self, input: &'a PruneInput) -> BoxFuture<'a, PruneResult> {
            let _ = input;
            Box::pin(async move { Ok(PruneOutput::default()) })
        }
    }

    let provider = test_provider();
    let (mut pipeline, handle) = Pipeline::new(provider.clone(), 100);

    let stage = PendingStage::default();
    pipeline.add_stage(stage.clone());

    // Set tip to 50 (within one chunk)
    handle.set_tip(50);

    let task_handle = tokio::spawn(async move { pipeline.run().await });
    tokio::time::sleep(Duration::from_millis(100)).await;

    handle.stop();

    let result = task_handle.await.unwrap();

    assert!(result.is_ok());
    assert_eq!(*stage.executed.lock().unwrap(), false);
}

// ============================================================================
// Error Propagation Tests
// ============================================================================

#[tokio::test]
async fn stage_execution_error_stops_pipeline() {
    let provider_factory = test_provider();
    let (mut pipeline, handle) = Pipeline::new(provider_factory.clone(), 10);

    let stage = FailingStage::new("Stage1");
    let stage_clone = stage.clone();

    pipeline.add_stage(stage);

    handle.set_tip(10);
    let result = pipeline.execute(10).await;
    assert!(result.is_err());

    // Checkpoint should not be set after failure
    let provider = provider_factory.provider_mut();
    assert_eq!(provider.execution_checkpoint(stage_clone.id()).unwrap(), None);
}

/// If a stage fails, all subsequent stages should not execute and the pipeline should stop.
#[tokio::test]
async fn stage_error_doesnt_affect_subsequent_runs() {
    let provider_factory = test_provider();
    let (mut pipeline, handle) = Pipeline::new(provider_factory, 10);

    let stage1 = FailingStage::new("FailStage");
    let stage2 = TrackingStage::new("Stage2");

    let stage1_clone = stage1.clone();
    let stage2_clone = stage2.clone();

    pipeline.add_stage(stage1);
    pipeline.add_stage(stage2);

    handle.set_tip(10);
    let error = pipeline.execute(10).await.unwrap_err();

    let katana_pipeline::Error::StageExecution { id, error } = error else {
        panic!("Unexpected error type");
    };

    assert_eq!(id, stage1_clone.id());
    assert!(error.to_string().contains("Stage execution failed")); // the error returned by the failing stage

    // Stage2 should not execute
    assert_eq!(stage2_clone.execution_count(), 0);
}

// ============================================================================
// Edge Cases Tests
// ============================================================================

#[tokio::test]
async fn empty_pipeline_returns_target() {
    let provider_factory = test_provider();
    let (mut pipeline, handle) = Pipeline::new(provider_factory, 10);

    // No stages added
    handle.set_tip(10);
    let result = pipeline.execute(10).await.unwrap();

    assert_eq!(result, 10);
}

#[tokio::test]
async fn tip_equals_checkpoint_no_execution() {
    let provider_factory = test_provider();
    let (mut pipeline, handle) = Pipeline::new(provider_factory.clone(), 10);

    let stage = TrackingStage::new("Stage1");
    let executions = stage.executions.clone();

    // set checkpoint for Stage1 stage
    let provider = provider_factory.provider_mut();
    provider.set_execution_checkpoint(stage.id(), 10).unwrap();
    provider.commit().unwrap();

    pipeline.add_stage(stage);

    handle.set_tip(10);
    pipeline.execute(10).await.unwrap();

    assert_eq!(executions.lock().unwrap().len(), 0, "Stage1 should not be executed");
}

/// If a stage's checkpoint (eg 20) is greater than the tip (eg 10), then the stage should be
/// skipped, and the [`Pipeline::run_once`] should return the checkpoint of the last stage executed
#[tokio::test]
async fn tip_less_than_checkpoint_skip_all() {
    let provider_factory = test_provider();
    let (mut pipeline, handle) = Pipeline::new(provider_factory.clone(), 10);

    let stage = TrackingStage::new("Stage1");
    let executions = stage.executions.clone();

    // set checkpoint for Stage1 stage
    let provider = provider_factory.provider_mut();
    let checkpoint = 20;
    provider.set_execution_checkpoint(stage.id(), checkpoint).unwrap();
    provider.commit().unwrap();

    pipeline.add_stage(stage);

    handle.set_tip(20);
    let result = pipeline.execute(10).await.unwrap();

    assert_eq!(result, checkpoint);
    assert_eq!(executions.lock().unwrap().len(), 0, "Stage1 should not be executed");
}

#[tokio::test]
async fn chunk_size_one_executes_block_by_block() {
    let provider_factory = test_provider();
    let (mut pipeline, handle) = Pipeline::new(provider_factory.clone(), 1);

    let stage = TrackingStage::new("Stage1");
    let stage_clone = stage.clone();

    pipeline.add_stage(stage);
    handle.set_tip(3);

    let pipeline_handle = tokio::spawn(async move { pipeline.run().await });
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    handle.stop();
    pipeline_handle.await.unwrap().unwrap();

    let execs = stage_clone.executions();
    assert_eq!(execs.len(), 3);

    assert_eq!(execs[0].from, 0);
    assert_eq!(execs[0].to, 1);

    assert_eq!(execs[1].from, 2);
    assert_eq!(execs[1].to, 2);

    assert_eq!(execs[2].from, 3);
    assert_eq!(execs[2].to, 3);
}

#[tokio::test]
async fn stage_checkpoint() {
    let provider_factory = test_provider();

    let (mut pipeline, handle) = Pipeline::new(provider_factory.clone(), 10);
    pipeline.add_stage(MockStage);

    // check that the checkpoint was set
    let initial_checkpoint = provider_factory.provider_mut().execution_checkpoint("Mock").unwrap();
    assert_eq!(initial_checkpoint, None);

    handle.set_tip(5);
    pipeline.execute(5).await.expect("failed to run the pipeline once");

    // check that the checkpoint was set
    let actual_checkpoint = provider_factory.provider_mut().execution_checkpoint("Mock").unwrap();
    assert_eq!(actual_checkpoint, Some(5));

    handle.set_tip(10);
    pipeline.execute(10).await.expect("failed to run the pipeline once");

    // check that the checkpoint was set
    let actual_checkpoint = provider_factory.provider_mut().execution_checkpoint("Mock").unwrap();
    assert_eq!(actual_checkpoint, Some(10));

    pipeline.execute(10).await.expect("failed to run the pipeline once");

    // check that the checkpoint doesn't change
    let actual_checkpoint = provider_factory.provider_mut().execution_checkpoint("Mock").unwrap();
    assert_eq!(actual_checkpoint, Some(10));
}

// ============================================================================
// Pruning Tests
// ============================================================================

use katana_pipeline::PruningConfig;
use katana_stage::PruningMode;

/// Pruning record to track what blocks were pruned
#[derive(Debug, Clone)]
struct PruneRecord {
    from: BlockNumber,
    to: BlockNumber,
}

/// Mock stage that tracks pruning calls
#[derive(Debug, Clone)]
struct PruningTrackingStage {
    id: &'static str,
    executions: Arc<Mutex<Vec<ExecutionRecord>>>,
    prunes: Arc<Mutex<Vec<PruneRecord>>>,
}

impl PruningTrackingStage {
    fn new(id: &'static str) -> Self {
        Self {
            id,
            executions: Arc::new(Mutex::new(Vec::new())),
            prunes: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn prune_records(&self) -> Vec<PruneRecord> {
        self.prunes.lock().unwrap().clone()
    }

    fn prune_count(&self) -> usize {
        self.prunes.lock().unwrap().len()
    }
}

impl Stage for PruningTrackingStage {
    fn id(&self) -> &'static str {
        self.id
    }

    fn execute<'a>(&'a mut self, input: &'a StageExecutionInput) -> BoxFuture<'a, StageResult> {
        Box::pin(async move {
            self.executions
                .lock()
                .unwrap()
                .push(ExecutionRecord { from: input.from(), to: input.to() });
            Ok(StageExecutionOutput { last_block_processed: input.to() })
        })
    }

    fn prune<'a>(&'a mut self, input: &'a PruneInput) -> BoxFuture<'a, PruneResult> {
        Box::pin(async move {
            if let Some(range) = input.prune_range() {
                self.prunes.lock().unwrap().push(PruneRecord { from: range.start, to: range.end });
                Ok(PruneOutput { pruned_count: range.end - range.start })
            } else {
                Ok(PruneOutput::default())
            }
        })
    }
}

#[tokio::test]
async fn prune_skips_when_no_execution_checkpoint() {
    let provider_factory = test_provider();
    let (mut pipeline, _handle) = Pipeline::new(provider_factory.clone(), 10);

    pipeline.set_pruning_config(PruningConfig::new(PruningMode::Full(10), Some(10)));

    let stage = PruningTrackingStage::new("Stage1");
    let stage_clone = stage.clone();
    pipeline.add_stage(stage);

    let provider = provider_factory.provider_mut();

    // Verify we don't have an execution checkpoint for the stage yet
    let execution_checkpoint = provider.execution_checkpoint(stage_clone.id()).unwrap();
    assert_eq!(execution_checkpoint, None);

    // No checkpoint set - stage has no data to prune
    pipeline.prune().await.unwrap();

    // Should not prune when there's no execution checkpoint
    assert_eq!(stage_clone.prune_count(), 0);
}

#[tokio::test]
async fn prune_skips_when_archive_mode() {
    let provider_factory = test_provider();
    let (mut pipeline, handle) = Pipeline::new(provider_factory.clone(), 10);

    // Archive mode is the default, but let's be explicit
    pipeline.set_pruning_config(PruningConfig::new(PruningMode::Archive, Some(10)));

    let stage = PruningTrackingStage::new("Stage1");
    let stage_clone = stage.clone();
    pipeline.add_stage(stage);

    // Set checkpoint to simulate execution having completed
    let provider = provider_factory.provider_mut();
    provider.set_execution_checkpoint(stage_clone.id(), 100).unwrap();
    provider.commit().unwrap();

    handle.set_tip(100);
    pipeline.prune().await.unwrap();

    // Archive mode should not prune anything
    assert_eq!(stage_clone.prune_count(), 0);
}

/// Tests different pruning modes and verifies the correct prune range is calculated:
/// - Full mode: keeps last N blocks, prunes everything before tip - N
/// - Minimal mode: keeps only latest state, prunes everything before tip - 1
/// - Full mode with insufficient blocks: skips pruning when tip < keep_blocks
#[tokio::test]
async fn prune_mode_behavior() {
    // Test case: Full mode keeps last N blocks
    // tip=100, keep=50 -> prune 0..50
    {
        let provider_factory = test_provider();
        let (mut pipeline, handle) = Pipeline::new(provider_factory.clone(), 10);
        pipeline.set_pruning_config(PruningConfig::new(PruningMode::Full(50), Some(10)));

        let stage = PruningTrackingStage::new("Stage1");
        let stage_clone = stage.clone();
        pipeline.add_stage(stage);

        let provider = provider_factory.provider_mut();
        provider.set_execution_checkpoint(stage_clone.id(), 100).unwrap();
        provider.commit().unwrap();

        handle.set_tip(100);
        pipeline.prune().await.unwrap();

        let records = stage_clone.prune_records();
        assert_eq!(records.len(), 1, "Full mode: expected one prune operation");
        assert_eq!(records[0].from, 0, "Full mode: prune range start mismatch");
        assert_eq!(records[0].to, 50, "Full mode: prune range end mismatch");
    }

    // Test case: Minimal mode keeps only latest
    // tip=100 -> prune 0..99
    {
        let provider_factory = test_provider();
        let (mut pipeline, handle) = Pipeline::new(provider_factory.clone(), 10);
        pipeline.set_pruning_config(PruningConfig::new(PruningMode::Minimal, Some(10)));

        let stage = PruningTrackingStage::new("Stage1");
        let stage_clone = stage.clone();
        pipeline.add_stage(stage);

        let provider = provider_factory.provider_mut();
        provider.set_execution_checkpoint(stage_clone.id(), 100).unwrap();
        provider.commit().unwrap();

        handle.set_tip(100);
        pipeline.prune().await.unwrap();

        let records = stage_clone.prune_records();
        assert_eq!(records.len(), 1, "Minimal mode: expected one prune operation");
        assert_eq!(records[0].from, 0, "Minimal mode: prune range start mismatch");
        assert_eq!(records[0].to, 99, "Minimal mode: prune range end mismatch");
    }

    // Test case: Full mode skips when not enough blocks
    // tip=50, keep=100 -> no pruning
    {
        let provider_factory = test_provider();
        let (mut pipeline, handle) = Pipeline::new(provider_factory.clone(), 10);
        pipeline.set_pruning_config(PruningConfig::new(PruningMode::Full(100), Some(10)));

        let stage = PruningTrackingStage::new("Stage1");
        let stage_clone = stage.clone();
        pipeline.add_stage(stage);

        let provider = provider_factory.provider_mut();
        provider.set_execution_checkpoint(stage_clone.id(), 50).unwrap();
        provider.commit().unwrap();

        handle.set_tip(50);
        pipeline.prune().await.unwrap();

        assert_eq!(
            stage_clone.prune_count(),
            0,
            "Full mode insufficient blocks: expected no pruning"
        );
    }
}

#[tokio::test]
async fn prune_uses_checkpoint_to_avoid_re_pruning() {
    let provider_factory = test_provider();
    let (mut pipeline, handle) = Pipeline::new(provider_factory.clone(), 10);

    pipeline.set_pruning_config(PruningConfig::new(PruningMode::Full(50), Some(10)));

    let stage = PruningTrackingStage::new("Stage1");
    let stage_clone = stage.clone();
    pipeline.add_stage(stage);

    let provider = provider_factory.provider_mut();
    // Set execution checkpoint to 200
    provider.set_execution_checkpoint(stage_clone.id(), 200).unwrap();
    // Set prune checkpoint - already pruned up to block 100
    provider.set_prune_checkpoint(stage_clone.id(), 100).unwrap();
    provider.commit().unwrap();

    handle.set_tip(200);
    pipeline.prune().await.unwrap();

    // Should only prune blocks 101-149 (from last_pruned+1 to tip-keep_blocks)
    let records = stage_clone.prune_records();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].from, 101); // last_pruned + 1
    assert_eq!(records[0].to, 150); // tip (200) - keep_blocks (50) = 150
}

#[tokio::test]
async fn prune_skips_when_already_caught_up() {
    let provider_factory = test_provider();
    let (mut pipeline, handle) = Pipeline::new(provider_factory.clone(), 10);

    pipeline.set_pruning_config(PruningConfig::new(PruningMode::Full(50), Some(10)));

    let stage = PruningTrackingStage::new("Stage1");
    let stage_clone = stage.clone();
    pipeline.add_stage(stage);

    let provider = provider_factory.provider_mut();
    // Set execution checkpoint to 100
    provider.set_execution_checkpoint(stage_clone.id(), 100).unwrap();
    // Already pruned up to block 49 (which is tip - keep_blocks - 1)
    provider.set_prune_checkpoint(stage_clone.id(), 49).unwrap();
    provider.commit().unwrap();

    handle.set_tip(100);
    pipeline.prune().await.unwrap();

    // Should not prune - already caught up
    assert_eq!(stage_clone.prune_count(), 0);
}

#[tokio::test]
async fn prune_multiple_stages_independently() {
    let provider_factory = test_provider();
    let (mut pipeline, handle) = Pipeline::new(provider_factory.clone(), 10);

    pipeline.set_pruning_config(PruningConfig::new(PruningMode::Full(50), Some(10)));

    let stage1 = PruningTrackingStage::new("Stage1");
    let stage2 = PruningTrackingStage::new("Stage2");
    let stage1_clone = stage1.clone();
    let stage2_clone = stage2.clone();

    pipeline.add_stage(stage1);
    pipeline.add_stage(stage2);

    let provider = provider_factory.provider_mut();
    // Stage1 at checkpoint 100
    provider.set_execution_checkpoint(stage1_clone.id(), 100).unwrap();
    // Stage2 at checkpoint 200
    provider.set_execution_checkpoint(stage2_clone.id(), 200).unwrap();
    provider.commit().unwrap();

    handle.set_tip(200);
    pipeline.prune().await.unwrap();

    // Stage1: prune 0-49 (tip=100, keep=50)
    let records1 = stage1_clone.prune_records();
    assert_eq!(records1.len(), 1);
    assert_eq!(records1[0].from, 0);
    assert_eq!(records1[0].to, 50);

    // Stage2: prune 0-149 (tip=200, keep=50)
    let records2 = stage2_clone.prune_records();
    assert_eq!(records2.len(), 1);
    assert_eq!(records2[0].from, 0);
    assert_eq!(records2[0].to, 150);

    // Verify prune checkpoints were set independently
    let provider = provider_factory.provider_mut();
    assert_eq!(provider.prune_checkpoint(stage1_clone.id()).unwrap(), Some(49));
    assert_eq!(provider.prune_checkpoint(stage2_clone.id()).unwrap(), Some(149));
}

/// Tests incremental pruning across multiple runs and verifies checkpoint persistence.
///
/// This test covers:
/// 1. Initial pruning sets the checkpoint correctly
/// 2. Subsequent pruning uses the checkpoint to avoid re-pruning
/// 3. Checkpoint is updated after each prune operation
#[tokio::test]
async fn prune_incremental_with_checkpoint_persistence() {
    let provider_factory = test_provider();
    let (mut pipeline, handle) = Pipeline::new(provider_factory.clone(), 10);

    pipeline.set_pruning_config(PruningConfig::new(PruningMode::Full(50), Some(10)));

    let stage = PruningTrackingStage::new("Stage1");
    let stage_clone = stage.clone();
    pipeline.add_stage(stage);

    // Verify no prune checkpoint initially
    let initial_prune_checkpoint =
        provider_factory.provider_mut().prune_checkpoint(stage_clone.id()).unwrap();
    assert_eq!(initial_prune_checkpoint, None);

    // First run: execution at 100
    let provider = provider_factory.provider_mut();
    provider.set_execution_checkpoint(stage_clone.id(), 100).unwrap();
    provider.commit().unwrap();

    handle.set_tip(100);
    pipeline.prune().await.unwrap();

    // Verify first prune operation
    let records = stage_clone.prune_records();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].from, 0);
    assert_eq!(records[0].to, 50);

    // Verify prune checkpoint was set after first prune
    let prune_checkpoint =
        provider_factory.provider_mut().prune_checkpoint(stage_clone.id()).unwrap();
    assert_eq!(prune_checkpoint, Some(49)); // 50 - 1 = 49

    // Second run: execution advanced to 200
    let provider = provider_factory.provider_mut();
    provider.set_execution_checkpoint(stage_clone.id(), 200).unwrap();
    provider.commit().unwrap();

    handle.set_tip(200);
    pipeline.prune().await.unwrap();

    // Should have two prune records now
    let records = stage_clone.prune_records();
    assert_eq!(records.len(), 2);
    // Second prune should start from 50 (previous prune checkpoint + 1)
    assert_eq!(records[1].from, 50);
    assert_eq!(records[1].to, 150);

    // Verify final prune checkpoint
    let provider = provider_factory.provider_mut();
    assert_eq!(provider.prune_checkpoint(stage_clone.id()).unwrap(), Some(149));
}

/// Mock stage that fails during pruning
#[derive(Debug, Clone)]
struct FailingPruneStage {
    id: &'static str,
}

impl FailingPruneStage {
    fn new(id: &'static str) -> Self {
        Self { id }
    }
}

impl Stage for FailingPruneStage {
    fn id(&self) -> &'static str {
        self.id
    }

    fn execute<'a>(&'a mut self, input: &'a StageExecutionInput) -> BoxFuture<'a, StageResult> {
        Box::pin(async move { Ok(StageExecutionOutput { last_block_processed: input.to() }) })
    }

    fn prune<'a>(&'a mut self, _: &'a PruneInput) -> BoxFuture<'a, PruneResult> {
        Box::pin(async { Err(katana_stage::Error::Other(anyhow!("Pruning failed"))) })
    }
}

#[tokio::test]
async fn prune_error_stops_pipeline() {
    let provider_factory = test_provider();
    let (mut pipeline, handle) = Pipeline::new(provider_factory.clone(), 10);

    pipeline.set_pruning_config(PruningConfig::new(PruningMode::Full(50), Some(10)));

    let failing_stage = FailingPruneStage::new("FailingStage");
    let stage2 = PruningTrackingStage::new("Stage2");
    let stage2_clone = stage2.clone();

    pipeline.add_stage(failing_stage);
    pipeline.add_stage(stage2);

    let provider = provider_factory.provider_mut();
    provider.set_execution_checkpoint("FailingStage", 100).unwrap();
    provider.set_execution_checkpoint(stage2_clone.id(), 100).unwrap();
    provider.commit().unwrap();

    handle.set_tip(100);
    let result = pipeline.prune().await;

    // Should return an error
    assert!(result.is_err());

    let katana_pipeline::Error::StagePruning { id, error } = result.unwrap_err() else {
        panic!("Unexpected error type");
    };

    assert_eq!(id, "FailingStage");
    assert!(error.to_string().contains("Pruning failed"));

    // Stage2 should not have been pruned since Stage1 failed
    assert_eq!(stage2_clone.prune_count(), 0);
}

#[tokio::test]
async fn prune_empty_pipeline_succeeds() {
    let provider_factory = test_provider();
    let (mut pipeline, _handle) = Pipeline::new(provider_factory, 10);

    pipeline.set_pruning_config(PruningConfig::new(PruningMode::Full(50), Some(10)));

    // No stages added
    let result = pipeline.prune().await;
    assert!(result.is_ok());
}
