use std::sync::{Arc, Mutex};

use katana_pipeline::Pipeline;
use katana_primitives::block::BlockNumber;
use katana_provider::api::stage::StageCheckpointProvider;
use katana_provider::test_utils::test_provider;
use katana_stage::{Stage, StageExecutionInput, StageResult};

/// Simple mock stage that does nothing
struct MockStage;

#[async_trait::async_trait]
impl Stage for MockStage {
    fn id(&self) -> &'static str {
        "Mock"
    }

    async fn execute(&mut self, _: &StageExecutionInput) -> StageResult {
        Ok(())
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

#[async_trait::async_trait]
impl Stage for TrackingStage {
    fn id(&self) -> &'static str {
        self.id
    }

    async fn execute(&mut self, input: &StageExecutionInput) -> StageResult {
        self.executions.lock().unwrap().push(ExecutionRecord { from: input.from, to: input.to });
        Ok(())
    }
}

/// Mock stage that fails on execution
struct FailingStage {
    id: &'static str,
}

impl FailingStage {
    fn new(id: &'static str) -> Self {
        Self { id }
    }
}

#[async_trait::async_trait]
impl Stage for FailingStage {
    fn id(&self) -> &'static str {
        self.id
    }

    async fn execute(&mut self, _: &StageExecutionInput) -> StageResult {
        Err(katana_stage::Error::Other(anyhow::anyhow!("Stage execution failed")))
    }
}

// ============================================================================
// run_to() - Single Stage Tests
// ============================================================================

#[tokio::test]
async fn run_to_executes_stage_to_target() {
    let provider = test_provider();
    let (mut pipeline, _handle) = Pipeline::new(provider.clone(), 10);

    let stage = TrackingStage::new("Stage1");
    let executions = stage.executions.clone();
    pipeline.add_stage(stage);

    let result = pipeline.run_to(5).await.unwrap();

    assert_eq!(result, 5);
    assert_eq!(provider.checkpoint("Stage1").unwrap(), Some(5));

    let execs = executions.lock().unwrap();
    assert_eq!(execs.len(), 1);
    assert_eq!(execs[0].from, 1); // checkpoint was 0, so from = 0 + 1
    assert_eq!(execs[0].to, 5);
}

#[tokio::test]
async fn run_to_skips_stage_when_checkpoint_equals_target() {
    let provider = test_provider();
    let (mut pipeline, _handle) = Pipeline::new(provider.clone(), 10);

    let stage = TrackingStage::new("Stage1");
    let executions = stage.executions.clone();
    pipeline.add_stage(stage);

    // Set initial checkpoint
    provider.set_checkpoint("Stage1", 5).unwrap();

    let result = pipeline.run_to(5).await.unwrap();

    assert_eq!(result, 5);
    assert_eq!(executions.lock().unwrap().len(), 0); // Not executed
}

#[tokio::test]
async fn run_to_skips_stage_when_checkpoint_exceeds_target() {
    let provider = test_provider();
    let (mut pipeline, _handle) = Pipeline::new(provider.clone(), 10);

    let stage = TrackingStage::new("Stage1");
    let executions = stage.executions.clone();
    pipeline.add_stage(stage);

    // Set checkpoint beyond target
    provider.set_checkpoint("Stage1", 10).unwrap();

    let result = pipeline.run_to(5).await.unwrap();

    assert_eq!(result, 10); // Returns the checkpoint
    assert_eq!(executions.lock().unwrap().len(), 0); // Not executed
}

#[tokio::test]
async fn run_to_uses_checkpoint_plus_one_as_from() {
    let provider = test_provider();
    let (mut pipeline, _handle) = Pipeline::new(provider.clone(), 10);

    let stage = TrackingStage::new("Stage1");
    let executions = stage.executions.clone();
    pipeline.add_stage(stage);

    // Set checkpoint to 3
    provider.set_checkpoint("Stage1", 3).unwrap();

    pipeline.run_to(10).await.unwrap();

    let execs = executions.lock().unwrap();
    assert_eq!(execs.len(), 1);
    assert_eq!(execs[0].from, 4); // 3 + 1
    assert_eq!(execs[0].to, 10);
}

// ============================================================================
// run_to() - Multiple Stages Tests
// ============================================================================

#[tokio::test]
async fn run_to_executes_all_stages_in_order() {
    let provider = test_provider();
    let (mut pipeline, _handle) = Pipeline::new(provider.clone(), 10);

    let stage1 = TrackingStage::new("Stage1");
    let stage2 = TrackingStage::new("Stage2");
    let stage3 = TrackingStage::new("Stage3");

    let execs1 = stage1.executions.clone();
    let execs2 = stage2.executions.clone();
    let execs3 = stage3.executions.clone();

    pipeline.add_stage(stage1);
    pipeline.add_stage(stage2);
    pipeline.add_stage(stage3);

    pipeline.run_to(5).await.unwrap();

    // All stages should execute
    assert_eq!(execs1.lock().unwrap().len(), 1);
    assert_eq!(execs2.lock().unwrap().len(), 1);
    assert_eq!(execs3.lock().unwrap().len(), 1);

    // All checkpoints should be set
    assert_eq!(provider.checkpoint("Stage1").unwrap(), Some(5));
    assert_eq!(provider.checkpoint("Stage2").unwrap(), Some(5));
    assert_eq!(provider.checkpoint("Stage3").unwrap(), Some(5));
}

#[tokio::test]
async fn run_to_with_mixed_checkpoints() {
    let provider = test_provider();
    let (mut pipeline, _handle) = Pipeline::new(provider.clone(), 10);

    let stage1 = TrackingStage::new("Stage1");
    let stage2 = TrackingStage::new("Stage2");
    let stage3 = TrackingStage::new("Stage3");

    let execs1 = stage1.executions.clone();
    let execs2 = stage2.executions.clone();
    let execs3 = stage3.executions.clone();

    pipeline.add_stage(stage1);
    pipeline.add_stage(stage2);
    pipeline.add_stage(stage3);

    // Stage1 already at checkpoint 10 (should skip)
    provider.set_checkpoint("Stage1", 10).unwrap();
    // Stage2 at checkpoint 3 (should execute)
    provider.set_checkpoint("Stage2", 3).unwrap();
    // Stage3 at checkpoint 0 (should execute)

    pipeline.run_to(10).await.unwrap();

    // Stage1 should be skipped
    assert_eq!(execs1.lock().unwrap().len(), 0);

    // Stage2 should execute from 4 to 10
    let e2 = execs2.lock().unwrap();
    assert_eq!(e2.len(), 1);
    assert_eq!(e2[0].from, 4);
    assert_eq!(e2[0].to, 10);

    // Stage3 should execute from 1 to 10
    let e3 = execs3.lock().unwrap();
    assert_eq!(e3.len(), 1);
    assert_eq!(e3[0].from, 1);
    assert_eq!(e3[0].to, 10);
}

#[tokio::test]
async fn run_to_last_stage_skipped_returns_checkpoint() {
    let provider = test_provider();
    let (mut pipeline, _handle) = Pipeline::new(provider.clone(), 10);

    let stage1 = TrackingStage::new("Stage1");
    let stage2 = TrackingStage::new("Stage2");

    pipeline.add_stage(stage1);
    pipeline.add_stage(stage2);

    provider.set_checkpoint("Stage1", 5).unwrap();
    provider.set_checkpoint("Stage2", 15).unwrap();

    let result = pipeline.run_to(10).await.unwrap();

    // Should return stage2's checkpoint since it's the last stage and was skipped
    assert_eq!(result, 15);
}

#[tokio::test]
async fn run_to_middle_stage_skip_continues() {
    let provider = test_provider();
    let (mut pipeline, _handle) = Pipeline::new(provider.clone(), 10);

    let stage1 = TrackingStage::new("Stage1");
    let stage2 = TrackingStage::new("Stage2");
    let stage3 = TrackingStage::new("Stage3");

    let execs1 = stage1.executions.clone();
    let execs2 = stage2.executions.clone();
    let execs3 = stage3.executions.clone();

    pipeline.add_stage(stage1);
    pipeline.add_stage(stage2);
    pipeline.add_stage(stage3);

    // Middle stage already complete
    provider.set_checkpoint("Stage2", 10).unwrap();

    pipeline.run_to(10).await.unwrap();

    // Stage1 and Stage3 should execute
    assert_eq!(execs1.lock().unwrap().len(), 1);
    assert_eq!(execs2.lock().unwrap().len(), 0); // Skipped
    assert_eq!(execs3.lock().unwrap().len(), 1);
}

// ============================================================================
// run() Loop - Tip Processing Tests
// ============================================================================

#[tokio::test]
async fn run_processes_single_chunk_to_tip() {
    let provider = Arc::new(test_provider());
    let (mut pipeline, handle) = Pipeline::new(provider.clone(), 100);

    let stage = TrackingStage::new("Stage1");
    let executions = stage.executions.clone();
    pipeline.add_stage(stage);

    // Set tip to 50 (within one chunk)
    handle.set_tip(50);

    // Run in background
    let pipeline_handle = tokio::spawn(async move { pipeline.run().await });

    // Give it time to process
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Drop handle to close channel and stop pipeline
    drop(handle);

    let result = pipeline_handle.await.unwrap();
    assert!(result.is_ok());

    // Should execute once from 1 to 50
    let execs = executions.lock().unwrap();
    assert_eq!(execs.len(), 1);
    assert_eq!(execs[0].from, 1);
    assert_eq!(execs[0].to, 50);
    assert_eq!(provider.checkpoint("Stage1").unwrap(), Some(50));
}

#[tokio::test]
async fn run_processes_multiple_chunks_to_tip() {
    let provider = Arc::new(test_provider());
    let (mut pipeline, handle) = Pipeline::new(provider.clone(), 10); // Small chunk size

    let stage = TrackingStage::new("Stage1");
    let executions = stage.executions.clone();
    pipeline.add_stage(stage);

    // Set tip to 25 (requires 3 chunks: 10, 20, 25)
    handle.set_tip(25);

    let pipeline_handle = tokio::spawn(async move { pipeline.run().await });

    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    drop(handle);

    let result = pipeline_handle.await.unwrap();
    assert!(result.is_ok());

    // Should execute 3 times
    let execs = executions.lock().unwrap();
    assert_eq!(execs.len(), 3);
    assert_eq!(execs[0].from, 1);
    assert_eq!(execs[0].to, 10);
    assert_eq!(execs[1].from, 11);
    assert_eq!(execs[1].to, 20);
    assert_eq!(execs[2].from, 21);
    assert_eq!(execs[2].to, 25);
}

#[tokio::test]
async fn run_waits_when_no_tip_set() {
    let provider = Arc::new(test_provider());
    let (mut pipeline, handle) = Pipeline::new(provider.clone(), 10);

    let stage = TrackingStage::new("Stage1");
    let executions = stage.executions.clone();
    pipeline.add_stage(stage);

    // Don't set tip
    let pipeline_handle = tokio::spawn(async move { pipeline.run().await });

    // Give it time - should be waiting
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // No executions should have happened
    assert_eq!(executions.lock().unwrap().len(), 0);

    drop(handle);
    let result = pipeline_handle.await.unwrap();
    assert!(result.is_ok());
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

    let pipeline_handle = tokio::spawn(async move { pipeline.run().await });

    // Wait for first tip to process
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Set new tip
    handle.set_tip(25);

    // Wait for second tip to process
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    drop(handle);
    let result = pipeline_handle.await.unwrap();
    assert!(result.is_ok());

    // Should have processed both tips
    let execs = executions.lock().unwrap();
    assert!(execs.len() >= 3); // 1-10, 11-20, 21-25
    assert_eq!(provider.checkpoint("Stage1").unwrap(), Some(25));
}

// ============================================================================
// Chunking Algorithm Tests
// ============================================================================

#[tokio::test]
async fn chunking_first_chunk_respects_chunk_size() {
    let provider = test_provider();
    let (mut pipeline, _handle) = Pipeline::new(provider.clone(), 10);

    let stage = TrackingStage::new("Stage1");
    let executions = stage.executions.clone();
    pipeline.add_stage(stage);

    // Manually test chunk logic by calling run_to with chunk boundary
    pipeline.run_to(10).await.unwrap();

    let execs = executions.lock().unwrap();
    assert_eq!(execs[0].to, 10);
}

#[tokio::test]
async fn chunking_uses_min_of_chunk_size_and_tip() {
    let provider = test_provider();
    let (mut pipeline, handle) = Pipeline::new(provider.clone(), 100);

    let stage = TrackingStage::new("Stage1");
    let executions = stage.executions.clone();
    pipeline.add_stage(stage);

    // Tip is smaller than chunk size
    handle.set_tip(50);

    let pipeline_handle = tokio::spawn(async move { pipeline.run().await });

    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    drop(handle);
    pipeline_handle.await.unwrap().unwrap();

    let execs = executions.lock().unwrap();
    assert_eq!(execs.len(), 1);
    assert_eq!(execs[0].to, 50); // Should not execute to 100
}

#[tokio::test]
async fn chunking_final_chunk_exact_tip() {
    let provider = test_provider();
    let (mut pipeline, handle) = Pipeline::new(provider.clone(), 10);

    let stage = TrackingStage::new("Stage1");
    let executions = stage.executions.clone();
    pipeline.add_stage(stage);

    // Tip that's not a multiple of chunk_size
    handle.set_tip(23);

    let pipeline_handle = tokio::spawn(async move { pipeline.run().await });

    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    drop(handle);
    pipeline_handle.await.unwrap().unwrap();

    let execs = executions.lock().unwrap();
    // Last chunk should be exactly to 23, not beyond
    assert_eq!(execs.last().unwrap().to, 23);
}

#[tokio::test]
async fn chunking_single_chunk_when_chunk_size_exceeds_tip() {
    let provider = test_provider();
    let (mut pipeline, handle) = Pipeline::new(provider.clone(), 1000);

    let stage = TrackingStage::new("Stage1");
    let executions = stage.executions.clone();
    pipeline.add_stage(stage);

    handle.set_tip(50);

    let pipeline_handle = tokio::spawn(async move { pipeline.run().await });

    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    drop(handle);
    pipeline_handle.await.unwrap().unwrap();

    let execs = executions.lock().unwrap();
    assert_eq!(execs.len(), 1); // Single chunk
    assert_eq!(execs[0].to, 50);
}

// ============================================================================
// Stage Execution Contract Tests
// ============================================================================

#[tokio::test]
async fn stage_receives_correct_input_from_checkpoint_plus_one() {
    let provider = test_provider();
    let (mut pipeline, _handle) = Pipeline::new(provider.clone(), 10);

    let stage = TrackingStage::new("Stage1");
    let executions = stage.executions.clone();
    pipeline.add_stage(stage);

    provider.set_checkpoint("Stage1", 7).unwrap();

    pipeline.run_to(15).await.unwrap();

    let execs = executions.lock().unwrap();
    assert_eq!(execs[0].from, 8); // 7 + 1
}

#[tokio::test]
async fn stage_receives_correct_to_parameter() {
    let provider = test_provider();
    let (mut pipeline, _handle) = Pipeline::new(provider.clone(), 10);

    let stage = TrackingStage::new("Stage1");
    let executions = stage.executions.clone();
    pipeline.add_stage(stage);

    pipeline.run_to(42).await.unwrap();

    let execs = executions.lock().unwrap();
    assert_eq!(execs[0].to, 42);
}

#[tokio::test]
async fn checkpoint_saved_after_successful_execution() {
    let provider = test_provider();
    let (mut pipeline, _handle) = Pipeline::new(provider.clone(), 10);

    pipeline.add_stage(TrackingStage::new("Stage1"));

    assert_eq!(provider.checkpoint("Stage1").unwrap(), None);

    pipeline.run_to(20).await.unwrap();

    assert_eq!(provider.checkpoint("Stage1").unwrap(), Some(20));
}

#[tokio::test]
async fn checkpoint_equals_to_parameter() {
    let provider = test_provider();
    let (mut pipeline, _handle) = Pipeline::new(provider.clone(), 10);

    pipeline.add_stage(TrackingStage::new("Stage1"));

    pipeline.run_to(17).await.unwrap();

    assert_eq!(provider.checkpoint("Stage1").unwrap(), Some(17));
}

// ============================================================================
// Error Propagation Tests
// ============================================================================

#[tokio::test]
async fn stage_execution_error_stops_pipeline() {
    let provider = test_provider();
    let (mut pipeline, _handle) = Pipeline::new(provider.clone(), 10);

    pipeline.add_stage(FailingStage::new("Stage1"));

    let result = pipeline.run_to(10).await;

    assert!(result.is_err());
    // Checkpoint should not be set after failure
    assert_eq!(provider.checkpoint("Stage1").unwrap(), None);
}

#[tokio::test]
async fn stage_error_doesnt_affect_subsequent_runs() {
    let provider = test_provider();
    let (mut pipeline, _handle) = Pipeline::new(provider.clone(), 10);

    pipeline.add_stage(FailingStage::new("FailStage"));
    pipeline.add_stage(TrackingStage::new("Stage2"));

    let result = pipeline.run_to(10).await;

    assert!(result.is_err());
    // Stage2 should not execute
    assert_eq!(provider.checkpoint("Stage2").unwrap(), None);
}

// ============================================================================
// Edge Cases Tests
// ============================================================================

#[tokio::test]
async fn empty_pipeline_returns_target() {
    let provider = test_provider();
    let (mut pipeline, _handle) = Pipeline::new(provider.clone(), 10);

    // No stages added
    let result = pipeline.run_to(10).await.unwrap();

    assert_eq!(result, 10);
}

#[tokio::test]
async fn tip_equals_checkpoint_no_execution() {
    let provider = test_provider();
    let (mut pipeline, _handle) = Pipeline::new(provider.clone(), 10);

    let stage = TrackingStage::new("Stage1");
    let executions = stage.executions.clone();

    // set checkpoint for Stage1 stage
    provider.set_checkpoint(stage.id(), 10).unwrap();
    pipeline.add_stage(stage);

    pipeline.run_to(10).await.unwrap();

    assert_eq!(executions.lock().unwrap().len(), 0, "Stage1 should not be executed");
}

/// If a stage's checkpoint (eg 20) is greater than the tip (eg 10), then the stage should be
/// skipped, and the [`Pipeline::run_to`] should return the checkpoint of the last stage executed
#[tokio::test]
async fn tip_less_than_checkpoint_skip_all() {
    let provider = test_provider();
    let (mut pipeline, _handle) = Pipeline::new(provider.clone(), 10);

    let stage = TrackingStage::new("Stage1");
    let executions = stage.executions.clone();

    // set checkpoint for Stage1 stage
    let checkpoint = 20;
    provider.set_checkpoint(stage.id(), checkpoint).unwrap();
    pipeline.add_stage(stage);

    let result = pipeline.run_to(10).await.unwrap();

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

    assert_eq!(execs[0].from, 1);
    assert_eq!(execs[0].to, 1);

    assert_eq!(execs[1].from, 2);
    assert_eq!(execs[1].to, 2);

    assert_eq!(execs[2].from, 3);
    assert_eq!(execs[2].to, 3);
}

/// The pipeline will be signaled to stop when all
/// [`PipelineHandle`](katana_pipeline::PipelineHandle)s associated with it have been dropped.
#[tokio::test]
async fn pipeline_stop_on_all_handle_dropped() {
    let provider = test_provider();
    let (mut pipeline, handle) = Pipeline::new(provider.clone(), 10);

    let stage = TrackingStage::new("Stage1");
    pipeline.add_stage(stage);

    let handle2 = handle.clone();

    // Drop first handle - pipeline should continue running
    drop(handle);

    let task_handle = tokio::spawn(async move { pipeline.run().await });
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

    assert!(!task_handle.is_finished(), "Pipeline should not have completed yet");

    // Drop the last handle - now pipeline should stop and the task should complete
    drop(handle2);

    // In the opposite case, the task should not complete if the pipeline is still running
    let result = task_handle.await.unwrap();

    assert!(result.is_ok());
}

#[tokio::test]
async fn stage_checkpoint() {
    let provider = test_provider();

    let (mut pipeline, _handle) = Pipeline::new(provider.clone(), 10);
    pipeline.add_stage(MockStage);

    // check that the checkpoint was set
    let initial_checkpoint = provider.checkpoint("Mock").unwrap();
    assert_eq!(initial_checkpoint, None);

    pipeline.run_to(5).await.expect("failed to run the pipeline once");

    // check that the checkpoint was set
    let actual_checkpoint = provider.checkpoint("Mock").unwrap();
    assert_eq!(actual_checkpoint, Some(5));

    pipeline.run_to(10).await.expect("failed to run the pipeline once");

    // check that the checkpoint was set
    let actual_checkpoint = provider.checkpoint("Mock").unwrap();
    assert_eq!(actual_checkpoint, Some(10));

    pipeline.run_to(10).await.expect("failed to run the pipeline once");

    // check that the checkpoint doesn't change
    let actual_checkpoint = provider.checkpoint("Mock").unwrap();
    assert_eq!(actual_checkpoint, Some(10));
}
