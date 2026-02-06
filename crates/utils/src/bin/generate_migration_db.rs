/// Binary to generate pre-migrated database snapshots for testing.
///
/// These snapshots are loaded by `TestNode::new_with_spawn_and_move_db()` and
/// `TestNode::new_with_simple_db()` to avoid the slow migration process (git clone + scarb
/// build + sozo migrate) in each test run.
///
/// Usage:
///   cargo run --bin generate_migration_db --features node -- --example spawn-and-move
/// --output tests/fixtures/db/spawn_and_move.tar.gz   cargo run --bin generate_migration_db
/// --features node -- --example simple --output tests/fixtures/db/simple.tar.gz
use std::path::{Path, PathBuf};
use std::process::Command;

use clap::Parser;
use katana_utils::node::{test_config, TestNode};

#[derive(Parser)]
#[command(about = "Generate pre-migrated database snapshots for testing")]
struct Args {
    /// The dojo example to migrate (e.g. "spawn-and-move" or "simple").
    #[arg(long)]
    example: String,

    /// Output path for the .tar.gz archive.
    #[arg(long)]
    output: PathBuf,
}

fn create_tar_gz(db_dir: &Path, output: &Path) -> std::io::Result<()> {
    // Derive the directory name from the output file stem (e.g., "spawn_and_move" from
    // "spawn_and_move.tar.gz")
    let stem = output
        .file_stem()
        .and_then(|s| s.to_str())
        .and_then(|s| s.strip_suffix(".tar"))
        .unwrap_or_else(|| {
            output.file_stem().and_then(|s| s.to_str()).expect("invalid output file name")
        });

    let parent = output.parent().expect("output path must have a parent directory");
    std::fs::create_dir_all(parent)?;

    // Create the archive directory structure expected by the Makefile extraction targets.
    // The tar should contain `<stem>/mdbx.dat`, `<stem>/db.version`, etc.
    let staging_dir = tempfile::tempdir()?;
    let inner_dir = staging_dir.path().join(stem);
    std::fs::create_dir_all(&inner_dir)?;

    // Copy db files into the staging directory
    for entry in std::fs::read_dir(db_dir)? {
        let entry = entry?;
        if entry.file_type()?.is_file() {
            std::fs::copy(entry.path(), inner_dir.join(entry.file_name()))?;
        }
    }

    let status = Command::new("tar")
        .args(["-czf"])
        .arg(output)
        .arg("-C")
        .arg(staging_dir.path())
        .arg(stem)
        .status()?;

    if !status.success() {
        return Err(std::io::Error::other("tar command failed"));
    }

    Ok(())
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    println!("Starting node with test_config()...");
    let node = TestNode::new_with_config(test_config()).await;

    println!("Migrating example '{}'...", args.example);
    match args.example.as_str() {
        "spawn-and-move" => {
            node.migrate_spawn_and_move().await.expect("failed to migrate spawn-and-move");
        }
        "simple" => {
            node.migrate_simple().await.expect("failed to migrate simple");
        }
        other => {
            eprintln!("Unknown example: {other}. Supported: spawn-and-move, simple");
            std::process::exit(1);
        }
    }

    // Get the database path from the running node
    let db_path = node.handle().node().db().path().to_path_buf();

    println!("Creating archive at {}...", args.output.display());
    let output_abs = std::env::current_dir().unwrap().join(&args.output);
    create_tar_gz(&db_path, &output_abs).expect("failed to create tar.gz archive");

    println!("Done! Snapshot saved to {}", args.output.display());
}
