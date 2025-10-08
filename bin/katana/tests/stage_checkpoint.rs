use anyhow::Result;
use clap::Parser;
use fixtures::TempDb;
use katana::cli::Cli;
use katana_db::abstraction::Database;
use katana_db::tables;
use rstest::*;

mod fixtures;

#[rstest]
fn set_and_get_checkpoint(empty_db: TempDb) -> Result<()> {
    let path = empty_db.path_str();
    let stage_id = "test_stage";
    let block_number = 42;

    // Set checkpoint
    Cli::parse_from([
        "katana",
        "stage",
        "checkpoint",
        "set",
        stage_id,
        &block_number.to_string(),
        "--db-path",
        path,
    ])
    .run()?;

    // Verify checkpoint was set by reading directly from db
    let db = empty_db.provider_ro();
    let tx = db.db().tx()?;
    let checkpoint = tx.get::<tables::StageCheckpoints>(stage_id.to_string())?;
    tx.commit()?;

    assert!(checkpoint.is_some(), "checkpoint should be set");
    assert_eq!(checkpoint.unwrap().block, block_number);

    // Get checkpoint via CLI
    Cli::parse_from(["katana", "stage", "checkpoint", "get", stage_id, "--db-path", path]).run()?;

    Ok(())
}

#[rstest]
fn get_nonexistent_checkpoint(empty_db: TempDb) -> Result<()> {
    let path = empty_db.path_str();
    let stage_id = "nonexistent_stage";

    // Try to get a checkpoint that doesn't exist - should not error
    Cli::parse_from(["katana", "stage", "checkpoint", "get", stage_id, "--db-path", path]).run()?;

    Ok(())
}

#[rstest]
fn update_existing_checkpoint(empty_db: TempDb) -> Result<()> {
    let path = empty_db.path_str();
    let stage_id = "update_stage";

    // Set initial checkpoint
    Cli::parse_from(["katana", "stage", "checkpoint", "set", stage_id, "100", "--db-path", path])
        .run()?;

    // Verify initial value
    let db = empty_db.provider_ro();
    let tx = db.db().tx()?;
    let checkpoint = tx.get::<tables::StageCheckpoints>(stage_id.to_string())?;
    tx.commit()?;
    assert_eq!(checkpoint.unwrap().block, 100);

    // Update to new value
    Cli::parse_from(["katana", "stage", "checkpoint", "set", stage_id, "200", "--db-path", path])
        .run()?;

    // Verify updated value
    let tx = db.db().tx()?;
    let checkpoint = tx.get::<tables::StageCheckpoints>(stage_id.to_string())?;
    tx.commit()?;
    assert_eq!(checkpoint.unwrap().block, 200);

    Ok(())
}

#[rstest]
fn multiple_stage_checkpoints(empty_db: TempDb) -> Result<()> {
    let path = empty_db.path_str();

    // Set checkpoints for different stages
    let stages = [("stage_a", 10), ("stage_b", 20), ("stage_c", 30)];

    for (stage_id, block_num) in stages {
        Cli::parse_from([
            "katana",
            "stage",
            "checkpoint",
            "set",
            stage_id,
            &block_num.to_string(),
            "--db-path",
            path,
        ])
        .run()?;
    }

    // Verify all checkpoints are set correctly
    let db = empty_db.provider_ro();
    let tx = db.db().tx()?;

    for (stage_id, expected_block) in stages {
        let checkpoint = tx.get::<tables::StageCheckpoints>(stage_id.to_string())?;
        assert!(checkpoint.is_some(), "checkpoint for {} should be set", stage_id);
        assert_eq!(
            checkpoint.unwrap().block,
            expected_block,
            "checkpoint for {} should be {}",
            stage_id,
            expected_block
        );
    }

    tx.commit()?;

    Ok(())
}

#[rstest]
fn set_checkpoint_zero(empty_db: TempDb) -> Result<()> {
    let path = empty_db.path_str();
    let stage_id = "zero_stage";

    // Set checkpoint to 0 (should be valid)
    Cli::parse_from(["katana", "stage", "checkpoint", "set", stage_id, "0", "--db-path", path])
        .run()?;

    // Verify checkpoint is set to 0
    let db = empty_db.provider_ro();
    let tx = db.db().tx()?;
    let checkpoint = tx.get::<tables::StageCheckpoints>(stage_id.to_string())?;
    tx.commit()?;

    assert!(checkpoint.is_some(), "checkpoint should be set");
    assert_eq!(checkpoint.unwrap().block, 0);

    Ok(())
}
