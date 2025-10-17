use std::future::pending;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::anyhow;
use futures::future::BoxFuture;
use katana_pipeline::Pipeline;
use katana_primitives::block::BlockNumber;
use katana_provider::api::stage::StageCheckpointProvider;
use katana_provider::test_utils::test_provider;
use katana_stage::{Stage, StageExecutionInput, StageExecutionOutput, StageResult};

/// Simple mock stage that does nothing
struct MockStage;

impl Stage for MockStage {
    fn id(&self) -> &'static str {
        "Mock"
    }

    fn execute<'a>(&'a mut self, input: &'a StageExecutionInput) -> BoxFuture<'a, StageResult> {
        Box::pin(async move { Ok(StageExecutionOutput { last_block_processed: input.to() }) })
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
}

// ============================================================================
// run_to() - Single Stage Tests
// ============================================================================

#[tokio::test]
async fn run_to_executes_stage_to_target() {
    let provider = test_provider();
    let (mut pipeline, handle) = Pipeline::new(provider.clone(), 10);

    let stage = TrackingStage::new("Stage1");
    let stage_clone = stage.clone();

    pipeline.add_stage(stage);
    handle.set_tip(5);
    let result = pipeline.execute_once(5).await.unwrap();

    assert_eq!(result, 5);
    assert_eq!(provider.checkpoint(stage_clone.id()).unwrap(), Some(5));

    let execs = stage_clone.executions();
    assert_eq!(execs.len(), 1);
    assert_eq!(execs[0].from, 0); // checkpoint was 0, so from = 0
    assert_eq!(execs[0].to, 5);
}

#[tokio::test]
async fn run_to_skips_stage_when_checkpoint_equals_target() {
    let provider = test_provider();
    let (mut pipeline, handle) = Pipeline::new(provider.clone(), 10);

    let stage = TrackingStage::new("Stage1");
    let stage_clone = stage.clone();

    // Set initial checkpoint
    provider.set_checkpoint(stage.id(), 5).unwrap();
    pipeline.add_stage(stage);

    handle.set_tip(5);
    let result = pipeline.execute_once(5).await.unwrap();

    assert_eq!(result, 5);
    assert_eq!(stage_clone.executions().len(), 0); // Not executed
}

#[tokio::test]
async fn run_to_skips_stage_when_checkpoint_exceeds_target() {
    let provider = test_provider();
    let (mut pipeline, handle) = Pipeline::new(provider.clone(), 10);

    let stage = TrackingStage::new("Stage1");
    let stage_clone = stage.clone();

    // Set checkpoint beyond target
    provider.set_checkpoint("Stage1", 10).unwrap();
    pipeline.add_stage(stage);

    handle.set_tip(10);
    let result = pipeline.execute_once(5).await.unwrap();

    assert_eq!(result, 10); // Returns the checkpoint
    assert_eq!(stage_clone.executions().len(), 0); // Not executed
}

#[tokio::test]
async fn run_to_uses_checkpoint_plus_one_as_from() {
    let provider = test_provider();
    let (mut pipeline, handle) = Pipeline::new(provider.clone(), 10);

    let stage = TrackingStage::new("Stage1");
    let stage_clone = stage.clone();

    // Set checkpoint to 3
    provider.set_checkpoint(stage.id(), 3).unwrap();
    pipeline.add_stage(stage);
    handle.set_tip(10);
    pipeline.execute_once(10).await.unwrap();

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
    let provider = test_provider();
    let (mut pipeline, handle) = Pipeline::new(provider.clone(), 10);

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
    pipeline.execute_once(5).await.unwrap();

    // All stages should be executed once because the tip is 5 and the chunk size is 10
    assert_eq!(stage1_clone.execution_count(), 1);
    assert_eq!(stage2_clone.execution_count(), 1);
    assert_eq!(stage3_clone.execution_count(), 1);

    // All checkpoints should be set
    assert_eq!(provider.checkpoint(stage1_clone.id()).unwrap(), Some(5));
    assert_eq!(provider.checkpoint(stage2_clone.id()).unwrap(), Some(5));
    assert_eq!(provider.checkpoint(stage3_clone.id()).unwrap(), Some(5));
}

#[tokio::test]
async fn run_to_with_mixed_checkpoints() {
    let provider = test_provider();
    let (mut pipeline, handle) = Pipeline::new(provider.clone(), 10);

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

    // Stage1 already at checkpoint 10 (should skip)
    provider.set_checkpoint(stage1_clone.id(), 10).unwrap();
    // Stage2 at checkpoint 3 (should execute)
    provider.set_checkpoint(stage2_clone.id(), 3).unwrap();

    handle.set_tip(10);
    pipeline.execute_once(10).await.unwrap();

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
    let provider = test_provider();
    let (mut pipeline, handle) = Pipeline::new(provider.clone(), 10);

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
    let result = pipeline.execute_once(20).await.unwrap();

    // make sure that all the stages were executed once
    assert_eq!(stage1_clone.execution_count(), 1);
    assert_eq!(stage2_clone.execution_count(), 1);
    assert_eq!(stage3_clone.execution_count(), 1);

    assert_eq!(result, 5);
    assert_eq!(provider.checkpoint(stage1_clone.id()).unwrap(), Some(10));
    assert_eq!(provider.checkpoint(stage2_clone.id()).unwrap(), Some(5));
    assert_eq!(provider.checkpoint(stage3_clone.id()).unwrap(), Some(20));
}

#[tokio::test]
async fn run_to_middle_stage_skip_continues() {
    let provider = test_provider();
    let (mut pipeline, handle) = Pipeline::new(provider.clone(), 10);

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
    provider.set_checkpoint(stage2_clone.id(), 10).unwrap();

    handle.set_tip(10);
    pipeline.execute_once(10).await.unwrap();

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
    let provider = Arc::new(test_provider());
    let (mut pipeline, handle) = Pipeline::new(provider.clone(), 100);

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

    assert_eq!(provider.checkpoint("Stage1").unwrap(), Some(50));
}

#[tokio::test]
async fn run_processes_multiple_chunks_to_tip() {
    let provider = Arc::new(test_provider());
    let (mut pipeline, handle) = Pipeline::new(provider.clone(), 10); // Small chunk size

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
    let provider = test_provider();
    let (mut pipeline, handle) = Pipeline::new(provider.clone(), 10);

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
    assert_eq!(provider.checkpoint("Stage1").unwrap(), Some(25));
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
    }

    let provider = Arc::new(test_provider());
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
    let provider = test_provider();
    let (mut pipeline, handle) = Pipeline::new(provider.clone(), 10);

    let stage = FailingStage::new("Stage1");
    let stage_clone = stage.clone();

    pipeline.add_stage(stage);

    handle.set_tip(10);
    let result = pipeline.execute_once(10).await;
    assert!(result.is_err());

    // Checkpoint should not be set after failure
    assert_eq!(provider.checkpoint(stage_clone.id()).unwrap(), None);
}

/// If a stage fails, all subsequent stages should not execute and the pipeline should stop.
#[tokio::test]
async fn stage_error_doesnt_affect_subsequent_runs() {
    let provider = test_provider();
    let (mut pipeline, handle) = Pipeline::new(provider.clone(), 10);

    let stage1 = FailingStage::new("FailStage");
    let stage2 = TrackingStage::new("Stage2");

    let stage1_clone = stage1.clone();
    let stage2_clone = stage2.clone();

    pipeline.add_stage(stage1);
    pipeline.add_stage(stage2);

    handle.set_tip(10);
    let error = pipeline.execute_once(10).await.unwrap_err();

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
    let provider = test_provider();
    let (mut pipeline, handle) = Pipeline::new(provider.clone(), 10);

    // No stages added
    handle.set_tip(10);
    let result = pipeline.execute_once(10).await.unwrap();

    assert_eq!(result, 10);
}

#[tokio::test]
async fn tip_equals_checkpoint_no_execution() {
    let provider = test_provider();
    let (mut pipeline, handle) = Pipeline::new(provider.clone(), 10);

    let stage = TrackingStage::new("Stage1");
    let executions = stage.executions.clone();

    // set checkpoint for Stage1 stage
    provider.set_checkpoint(stage.id(), 10).unwrap();
    pipeline.add_stage(stage);

    handle.set_tip(10);
    pipeline.execute_once(10).await.unwrap();

    assert_eq!(executions.lock().unwrap().len(), 0, "Stage1 should not be executed");
}

/// If a stage's checkpoint (eg 20) is greater than the tip (eg 10), then the stage should be
/// skipped, and the [`Pipeline::run_once`] should return the checkpoint of the last stage executed
#[tokio::test]
async fn tip_less_than_checkpoint_skip_all() {
    let provider = test_provider();
    let (mut pipeline, handle) = Pipeline::new(provider.clone(), 10);

    let stage = TrackingStage::new("Stage1");
    let executions = stage.executions.clone();

    // set checkpoint for Stage1 stage
    let checkpoint = 20;
    provider.set_checkpoint(stage.id(), checkpoint).unwrap();
    pipeline.add_stage(stage);

    handle.set_tip(20);
    let result = pipeline.execute_once(10).await.unwrap();

    assert_eq!(result, checkpoint);
    assert_eq!(executions.lock().unwrap().len(), 0, "Stage1 should not be executed");
}

#[tokio::test]
async fn chunk_size_one_executes_block_by_block() {
    let provider = test_provider();
    let (mut pipeline, handle) = Pipeline::new(provider.clone(), 1);

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
    let provider = test_provider();

    let (mut pipeline, handle) = Pipeline::new(provider.clone(), 10);
    pipeline.add_stage(MockStage);

    // check that the checkpoint was set
    let initial_checkpoint = provider.checkpoint("Mock").unwrap();
    assert_eq!(initial_checkpoint, None);

    handle.set_tip(5);
    pipeline.execute_once(5).await.expect("failed to run the pipeline once");

    // check that the checkpoint was set
    let actual_checkpoint = provider.checkpoint("Mock").unwrap();
    assert_eq!(actual_checkpoint, Some(5));

    handle.set_tip(10);
    pipeline.execute_once(10).await.expect("failed to run the pipeline once");

    // check that the checkpoint was set
    let actual_checkpoint = provider.checkpoint("Mock").unwrap();
    assert_eq!(actual_checkpoint, Some(10));

    pipeline.execute_once(10).await.expect("failed to run the pipeline once");

    // check that the checkpoint doesn't change
    let actual_checkpoint = provider.checkpoint("Mock").unwrap();
    assert_eq!(actual_checkpoint, Some(10));
}
