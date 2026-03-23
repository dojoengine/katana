//! Standalone binary to train zstd compression dictionaries from an existing Katana database.
//!
//! Usage:
//!   cargo run --bin train-dict --features cli -- --path /data/katana-mainnet-data2/

use std::path::PathBuf;

use clap::Parser;

use katana_db::abstraction::{Database, DbCursor, DbTx};
use katana_db::codecs::Compress;
use katana_db::{tables, Db};

/// Train zstd compression dictionaries from an existing Katana database.
#[derive(Parser)]
#[command(name = "train-dict")]
struct Args {
    /// Path to the Katana database directory.
    #[arg(long)]
    path: PathBuf,

    /// Output directory for trained dictionaries.
    #[arg(long, default_value = "./dictionaries")]
    output_dir: PathBuf,

    /// Target dictionary size in bytes.
    #[arg(long, default_value_t = 65536)]
    dict_size: usize,

    /// Maximum number of samples to collect per table.
    #[arg(long, default_value_t = 100_000)]
    max_samples: usize,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    std::fs::create_dir_all(&args.output_dir)?;

    println!("Opening database at {} (read-only)...", args.path.display());
    let db = Db::open_ro(&args.path)?;

    // Collect receipt samples: read envelope values, extract inner receipt, re-serialize to
    // postcard bytes (the raw payload that will be compressed).
    println!("\n=== Receipts ===");
    {
        let tx = db.tx()?;
        let mut cursor = tx.cursor::<tables::Receipts>()?;
        let mut samples = Vec::new();
        let mut walker = cursor.walk(Some(0))?;
        while let Some(Ok((_key, envelope))) = walker.next() {
            if samples.len() >= args.max_samples {
                break;
            }
            // Re-serialize the inner receipt to postcard bytes (the compress trait impl).
            if let Ok(bytes) = envelope.inner.compress() {
                samples.push(bytes.into());
            }
        }
        drop(walker);
        drop(cursor);
        drop(tx);

        println!("Collected {} samples", samples.len());
        if samples.is_empty() {
            println!("No receipt samples found, skipping.");
        } else {
            train_and_write(&samples, args.dict_size, &args.output_dir, "receipts_v1.dict")?;
        }
    }

    // Collect transaction samples
    println!("\n=== Transactions ===");
    {
        let tx = db.tx()?;
        let mut cursor = tx.cursor::<tables::Transactions>()?;
        let mut samples = Vec::new();
        let mut walker = cursor.walk(Some(0))?;
        while let Some(Ok((_key, envelope))) = walker.next() {
            if samples.len() >= args.max_samples {
                break;
            }
            if let Ok(bytes) = envelope.inner.compress() {
                samples.push(bytes.into());
            }
        }
        drop(walker);
        drop(cursor);
        drop(tx);

        println!("Collected {} samples", samples.len());
        if samples.is_empty() {
            println!("No transaction samples found, skipping.");
        } else {
            train_and_write(
                &samples,
                args.dict_size,
                &args.output_dir,
                "transactions_v1.dict",
            )?;
        }
    }

    println!("\nDone.");
    Ok(())
}

fn train_and_write(
    samples: &[Vec<u8>],
    dict_size: usize,
    output_dir: &PathBuf,
    filename: &str,
) -> anyhow::Result<()> {
    let sample_refs: Vec<&[u8]> = samples.iter().map(|s| s.as_slice()).collect();

    println!("Training dictionary (size={dict_size}, samples={})...", samples.len());
    let dict = zstd::dict::from_samples(&sample_refs, dict_size)?;

    let out_path = output_dir.join(filename);
    std::fs::write(&out_path, &dict)?;
    println!("Wrote dictionary to {} ({} bytes)", out_path.display(), dict.len());

    // Print compression stats
    print_stats(samples, &dict);

    Ok(())
}

fn print_stats(samples: &[Vec<u8>], dict: &[u8]) {
    let total_raw: usize = samples.iter().map(|s| s.len()).sum();

    // Compress without dictionary
    let total_zstd: usize = samples
        .iter()
        .map(|s| zstd::encode_all(s.as_slice(), 0).map(|c| c.len()).unwrap_or(s.len()))
        .sum();

    // Compress with dictionary
    let encoder = zstd::dict::EncoderDictionary::copy(dict, 0);
    let total_dict: usize = samples
        .iter()
        .map(|s| {
            let mut output = Vec::new();
            let mut enc =
                zstd::stream::Encoder::with_prepared_dictionary(&mut output, &encoder).unwrap();
            std::io::copy(&mut std::io::Cursor::new(s), &mut enc).unwrap();
            enc.finish().unwrap();
            output.len()
        })
        .sum();

    let ratio_zstd = total_raw as f64 / total_zstd as f64;
    let ratio_dict = total_raw as f64 / total_dict as f64;

    println!("  Raw total:       {total_raw:>10} bytes");
    println!("  Zstd (no dict):  {total_zstd:>10} bytes  (ratio: {ratio_zstd:.2}x)");
    println!("  Zstd (w/ dict):  {total_dict:>10} bytes  (ratio: {ratio_dict:.2}x)");
    println!(
        "  Dictionary gain: {:.1}%",
        (1.0 - (total_dict as f64 / total_zstd as f64)) * 100.0
    );
}
