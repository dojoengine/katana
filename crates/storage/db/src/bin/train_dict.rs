//! Standalone binary to train and benchmark zstd compression dictionaries from an existing Katana
//! database.
//!
//! **Subcommands:**
//!
//! - `train` — Train dictionaries from random samples and write them to disk.
//! - `pareto` — Systematic exploration: vary training range, dict size, and sample count, then
//!   evaluate every combination against a single held-out random test set.
//!
//! Usage:
//!   cargo run --release --bin train-dict --features cli -- train  --path /data/katana-mainnet-data2/
//!   cargo run --release --bin train-dict --features cli -- pareto --path /data/katana-mainnet-data2/

use std::collections::HashSet;
use std::path::PathBuf;

use clap::{Parser, Subcommand};
use rand::seq::SliceRandom;
use rand::{Rng, SeedableRng};

use katana_db::abstraction::{Database, DbCursor, DbTx};
use katana_db::codecs::Compress;
use katana_db::{tables, Db};

type StdRng = rand::rngs::StdRng;

#[derive(Parser)]
#[command(name = "train-dict")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Train dictionaries from random samples and write to disk.
    Train {
        /// Path to the Katana database directory.
        #[arg(long)]
        path: PathBuf,

        /// Output directory for trained dictionaries.
        #[arg(long, default_value = "./dictionaries")]
        output_dir: PathBuf,

        /// Target dictionary size in bytes.
        #[arg(long, default_value_t = 65536)]
        dict_size: usize,

        /// Total number of samples to collect per table (split into train + test).
        #[arg(long, default_value_t = 100_000)]
        max_samples: usize,

        /// Fraction of samples reserved for the test set (0.0–1.0).
        #[arg(long, default_value_t = 0.2)]
        test_ratio: f64,

        /// RNG seed for reproducible sampling.
        #[arg(long, default_value_t = 42)]
        seed: u64,
    },

    /// Explore the pareto frontier: vary training range, dict size, and sample count.
    Pareto {
        /// Path to the Katana database directory.
        #[arg(long)]
        path: PathBuf,

        /// Number of test samples (held-out, random across full range).
        #[arg(long, default_value_t = 20_000)]
        test_samples: usize,

        /// RNG seed for reproducible sampling.
        #[arg(long, default_value_t = 42)]
        seed: u64,

        /// Output directory to save the best dictionary for each table.
        #[arg(long, default_value = "./dictionaries")]
        output_dir: PathBuf,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Train { path, output_dir, dict_size, max_samples, test_ratio, seed } => {
            cmd_train(&path, &output_dir, dict_size, max_samples, test_ratio, seed)
        }
        Command::Pareto { path, test_samples, seed, output_dir } => {
            cmd_pareto(&path, test_samples, seed, &output_dir)
        }
    }
}

// ── train subcommand ────────────────────────────────────────────────────────

fn cmd_train(
    path: &PathBuf,
    output_dir: &PathBuf,
    dict_size: usize,
    max_samples: usize,
    test_ratio: f64,
    seed: u64,
) -> anyhow::Result<()> {
    std::fs::create_dir_all(output_dir)?;
    println!("Opening database at {} (read-only)...", path.display());
    let db = Db::open_ro(path)?;
    let mut rng = StdRng::seed_from_u64(seed);

    for (name, filename, samples) in [
        (
            "Receipts",
            "receipts_v1.dict",
            collect_random_samples::<tables::Receipts>(&db, max_samples, &mut rng)?,
        ),
        (
            "Transactions",
            "transactions_v1.dict",
            collect_random_samples::<tables::Transactions>(&db, max_samples, &mut rng)?,
        ),
    ] {
        println!("\n=== {name} ===");
        if samples.is_empty() {
            println!("No samples found, skipping.");
            continue;
        }

        let split = ((1.0 - test_ratio) * samples.len() as f64) as usize;
        let (train, test) = samples.split_at(split);
        println!(
            "Collected {} total samples ({} train, {} test)",
            samples.len(),
            train.len(),
            test.len()
        );

        let train_refs: Vec<&[u8]> = train.iter().map(|s| s.as_slice()).collect();
        println!("Training dictionary (size={dict_size})...");
        let dict = zstd::dict::from_samples(&train_refs, dict_size)?;

        let out_path = output_dir.join(filename);
        std::fs::write(&out_path, &dict)?;
        println!("Wrote dictionary to {} ({} bytes)", out_path.display(), dict.len());

        println!("\n  -- Test set ({} samples) --", test.len());
        print_stats(test, &dict);
        println!("\n  -- Train set ({} samples, for reference) --", train.len());
        print_stats(train, &dict);
    }

    println!("\nDone.");
    Ok(())
}

// ── pareto subcommand ───────────────────────────────────────────────────────

/// Describes a range within the key space to draw training samples from.
#[derive(Clone)]
struct TrainRange {
    label: &'static str,
    /// Start fraction of the key range (0.0 = first key).
    start_frac: f64,
    /// End fraction of the key range (1.0 = last key).
    end_frac: f64,
}

fn cmd_pareto(
    path: &PathBuf,
    test_samples: usize,
    seed: u64,
    output_dir: &PathBuf,
) -> anyhow::Result<()> {
    std::fs::create_dir_all(output_dir)?;
    println!("Opening database at {} (read-only)...", path.display());
    let db = Db::open_ro(path)?;

    // ── Axis definitions ────────────────────────────────────────────────
    let ranges = vec![
        TrainRange { label: "full", start_frac: 0.0, end_frac: 1.0 },
        TrainRange { label: "recent-25%", start_frac: 0.75, end_frac: 1.0 },
        TrainRange { label: "recent-10%", start_frac: 0.90, end_frac: 1.0 },
        TrainRange { label: "recent-5%", start_frac: 0.95, end_frac: 1.0 },
        TrainRange { label: "mid-50%", start_frac: 0.25, end_frac: 0.75 },
        TrainRange { label: "oldest-25%", start_frac: 0.0, end_frac: 0.25 },
    ];

    let dict_sizes: Vec<usize> = vec![8_192, 16_384, 32_768, 65_536, 131_072];
    let train_counts: Vec<usize> = vec![10_000, 50_000, 100_000];

    // ── Run for each table ──────────────────────────────────────────────
    pareto_for_table::<tables::Receipts>(
        &db,
        "Receipts",
        "receipts_v1.dict",
        &ranges,
        &dict_sizes,
        &train_counts,
        test_samples,
        seed,
        output_dir,
    )?;

    pareto_for_table::<tables::Transactions>(
        &db,
        "Transactions",
        "transactions_v1.dict",
        &ranges,
        &dict_sizes,
        &train_counts,
        test_samples,
        seed + 1, // different seed so test set differs from receipts
        output_dir,
    )?;

    Ok(())
}

fn pareto_for_table<T>(
    db: &Db,
    table_name: &str,
    best_dict_filename: &str,
    ranges: &[TrainRange],
    dict_sizes: &[usize],
    train_counts: &[usize],
    test_samples: usize,
    seed: u64,
    output_dir: &PathBuf,
) -> anyhow::Result<()>
where
    T: katana_db::tables::Table<Key = u64>,
    T::Value: Compress,
{
    println!("\n{}", "=".repeat(60));
    println!("  PARETO EXPLORATION: {table_name}");
    println!("{}", "=".repeat(60));

    let mut rng = StdRng::seed_from_u64(seed);

    // Determine key range
    let tx = db.tx()?;
    let total = tx.entries::<T>()?;
    let mut cursor = tx.cursor::<T>()?;
    let (min_key, _) = cursor.first()?.expect("table is non-empty");
    let (max_key, _) = cursor.last()?.expect("table is non-empty");
    drop(cursor);
    drop(tx);

    println!("Table: {total} entries, key range {min_key}..={max_key}");

    // ── Collect fixed test set (random across full range) ───────────────
    println!("Collecting {test_samples} random test samples across full range...");
    let test_set = collect_range_samples::<T>(db, min_key, max_key, test_samples, &mut rng)?;
    // Note: we don't explicitly dedup train vs test keys — the random shuffle gives sufficient
    // separation, and seeking random keys across 281M entries has negligible collision probability.
    println!("Test set: {} samples", test_set.len());

    // Baseline stats
    let total_raw: usize = test_set.iter().map(|s| s.len()).sum();
    let total_identity: usize = test_set.iter().map(|s| s.len() + 8).sum();
    let total_zstd: usize = test_set
        .iter()
        .map(|s| zstd::encode_all(s.as_slice(), 0).map(|c| c.len()).unwrap_or(s.len()))
        .sum();

    println!(
        "Baselines — raw: {} B, identity(+8B hdr): {} B, zstd(no dict): {} B",
        total_raw, total_identity, total_zstd
    );
    println!();

    // ── Print table header ──────────────────────────────────────────────
    println!(
        "{:<14} {:>10} {:>10} {:>12} {:>12} {:>8} {:>8}",
        "range", "dict_size", "train_n", "test_bytes", "vs_ident%", "vs_zstd%", "ratio"
    );
    println!("{}", "-".repeat(82));

    let mut best_score = usize::MAX;
    let mut best_dict: Option<Vec<u8>> = None;
    let mut best_label = String::new();

    // ── Sweep all combinations ──────────────────────────────────────────
    for range in ranges {
        let range_start = min_key + ((max_key - min_key) as f64 * range.start_frac) as u64;
        let range_end = min_key + ((max_key - min_key) as f64 * range.end_frac) as u64;

        for &train_n in train_counts {
            // Collect training samples from this range
            let mut train_rng = StdRng::seed_from_u64(seed.wrapping_add(
                (range.label.len() as u64) * 1000 + train_n as u64,
            ));
            let train_set =
                collect_range_samples::<T>(db, range_start, range_end, train_n, &mut train_rng)?;

            if train_set.len() < 100 {
                continue; // too few samples to train
            }

            for &dict_size in dict_sizes {
                let train_refs: Vec<&[u8]> = train_set.iter().map(|s| s.as_slice()).collect();
                let dict = match zstd::dict::from_samples(&train_refs, dict_size) {
                    Ok(d) => d,
                    Err(_) => continue,
                };

                // Evaluate on the fixed test set
                let total_dict = compress_all_with_dict(&test_set, &dict);

                let gain_vs_identity =
                    (1.0 - total_dict as f64 / total_identity as f64) * 100.0;
                let gain_vs_zstd =
                    (1.0 - total_dict as f64 / total_zstd as f64) * 100.0;
                let ratio = total_raw as f64 / total_dict as f64;

                println!(
                    "{:<14} {:>8}KB {:>10} {:>12} {:>11.1}% {:>7.1}% {:>8.3}",
                    range.label,
                    dict_size / 1024,
                    train_set.len(),
                    total_dict,
                    gain_vs_identity,
                    gain_vs_zstd,
                    ratio
                );

                if total_dict < best_score {
                    best_score = total_dict;
                    best_dict = Some(dict);
                    best_label = format!(
                        "{}  dict={}KB  train_n={}",
                        range.label,
                        dict_size / 1024,
                        train_set.len()
                    );
                }
            }
        }
    }

    println!("{}", "-".repeat(82));
    println!("Best: {best_label}  ({best_score} bytes on test set)");

    // Save the best dictionary
    if let Some(dict) = best_dict {
        let out_path = output_dir.join(best_dict_filename);
        std::fs::write(&out_path, &dict)?;
        println!("Saved best dictionary to {}", out_path.display());

        println!("\nDetailed stats for best dictionary:");
        print_stats(&test_set, &dict);
    }

    Ok(())
}

// ── Shared helpers ──────────────────────────────────────────────────────────

/// Collect samples by seeking to random keys within [range_start, range_end].
fn collect_range_samples<T>(
    db: &Db,
    range_start: u64,
    range_end: u64,
    max_samples: usize,
    rng: &mut StdRng,
) -> anyhow::Result<Vec<Vec<u8>>>
where
    T: katana_db::tables::Table<Key = u64>,
    T::Value: Compress,
{
    let tx = db.tx()?;
    let mut cursor = tx.cursor::<T>()?;
    let mut samples = Vec::with_capacity(max_samples);
    let mut seen_keys = HashSet::new();

    // Oversample to account for dedup
    let oversample = (max_samples as f64 * 1.3) as usize;
    let mut target_keys: Vec<u64> =
        (0..oversample).map(|_| rng.gen_range(range_start..=range_end)).collect();
    target_keys.sort_unstable();
    target_keys.dedup();

    for target in target_keys {
        if samples.len() >= max_samples {
            break;
        }
        if let Some((actual_key, value)) = cursor.seek(target)? {
            if actual_key > range_end {
                continue; // seek overshot past our range
            }
            if seen_keys.insert(actual_key) {
                if let Ok(bytes) = value.compress() {
                    samples.push(bytes.into());
                }
            }
        }
    }

    samples.shuffle(rng);
    Ok(samples)
}

/// Legacy: collect random samples across the full key range (used by `train` subcommand).
fn collect_random_samples<T>(
    db: &Db,
    max_samples: usize,
    rng: &mut StdRng,
) -> anyhow::Result<Vec<Vec<u8>>>
where
    T: katana_db::tables::Table<Key = u64>,
    T::Value: Compress,
{
    let tx = db.tx()?;
    let total = tx.entries::<T>()?;
    println!("  Table has {total} entries");

    if total == 0 {
        return Ok(Vec::new());
    }

    let mut cursor = tx.cursor::<T>()?;
    let (min_key, _) = cursor.first()?.expect("table is non-empty");
    let (max_key, _) = cursor.last()?.expect("table is non-empty");
    println!("  Key range: {min_key}..={max_key}");
    drop(cursor);
    drop(tx);

    collect_range_samples::<T>(db, min_key, max_key, max_samples, rng)
}

/// Compress every sample with a prepared dictionary, returning total compressed size.
fn compress_all_with_dict(samples: &[Vec<u8>], dict: &[u8]) -> usize {
    let encoder = zstd::dict::EncoderDictionary::copy(dict, 0);
    samples
        .iter()
        .map(|s| {
            let mut output = Vec::new();
            let mut enc =
                zstd::stream::Encoder::with_prepared_dictionary(&mut output, &encoder).unwrap();
            std::io::copy(&mut std::io::Cursor::new(s), &mut enc).unwrap();
            enc.finish().unwrap();
            output.len()
        })
        .sum()
}

fn print_stats(samples: &[Vec<u8>], dict: &[u8]) {
    let total_raw: usize = samples.iter().map(|s| s.len()).sum();
    let avg_raw = total_raw as f64 / samples.len() as f64;
    let total_identity: usize = samples.iter().map(|s| s.len() + 8).sum();

    let zstd_sizes: Vec<usize> = samples
        .iter()
        .map(|s| zstd::encode_all(s.as_slice(), 0).map(|c| c.len()).unwrap_or(s.len()))
        .collect();
    let total_zstd: usize = zstd_sizes.iter().sum();

    let dict_sizes: Vec<usize> = {
        let encoder = zstd::dict::EncoderDictionary::copy(dict, 0);
        samples
            .iter()
            .map(|s| {
                let mut output = Vec::new();
                let mut enc =
                    zstd::stream::Encoder::with_prepared_dictionary(&mut output, &encoder).unwrap();
                std::io::copy(&mut std::io::Cursor::new(s), &mut enc).unwrap();
                enc.finish().unwrap();
                output.len()
            })
            .collect()
    };
    let total_dict: usize = dict_sizes.iter().sum();

    let expanded_zstd =
        zstd_sizes.iter().zip(samples.iter()).filter(|(c, s)| **c >= s.len()).count();
    let expanded_dict =
        dict_sizes.iter().zip(samples.iter()).filter(|(c, s)| **c >= s.len()).count();

    let ratio_zstd = total_raw as f64 / total_zstd as f64;
    let ratio_dict = total_raw as f64 / total_dict as f64;

    let mut sorted_raw: Vec<usize> = samples.iter().map(|s| s.len()).collect();
    sorted_raw.sort_unstable();
    let mut sorted_dict = dict_sizes.clone();
    sorted_dict.sort_unstable();

    println!("  Payload sizes (raw postcard):");
    println!(
        "    avg: {avg_raw:.0} B | p50: {} B | p95: {} B | p99: {} B | min: {} B | max: {} B",
        percentile(&sorted_raw, 50),
        percentile(&sorted_raw, 95),
        percentile(&sorted_raw, 99),
        sorted_raw.first().unwrap_or(&0),
        sorted_raw.last().unwrap_or(&0),
    );

    println!();
    println!("  Aggregate compression:");
    println!("    Identity (raw+hdr): {total_identity:>12} bytes");
    println!(
        "    Zstd (no dict):     {total_zstd:>12} bytes  (ratio: {ratio_zstd:.3}x, {expanded_zstd}/{} expanded)",
        samples.len()
    );
    println!(
        "    Zstd (w/ dict):     {total_dict:>12} bytes  (ratio: {ratio_dict:.3}x, {expanded_dict}/{} expanded)",
        samples.len()
    );
    println!(
        "    Dictionary gain vs zstd:     {:.1}%",
        (1.0 - (total_dict as f64 / total_zstd as f64)) * 100.0
    );
    println!(
        "    Dictionary gain vs identity: {:.1}%",
        (1.0 - (total_dict as f64 / total_identity as f64)) * 100.0
    );

    println!();
    println!("  Per-record compressed sizes (w/ dict):");
    println!(
        "    avg: {:.0} B | p50: {} B | p95: {} B | p99: {} B | min: {} B | max: {} B",
        total_dict as f64 / samples.len() as f64,
        percentile(&sorted_dict, 50),
        percentile(&sorted_dict, 95),
        percentile(&sorted_dict, 99),
        sorted_dict.first().unwrap_or(&0),
        sorted_dict.last().unwrap_or(&0),
    );
}

fn percentile(sorted: &[usize], p: usize) -> usize {
    if sorted.is_empty() {
        return 0;
    }
    let idx = (p as f64 / 100.0 * (sorted.len() - 1) as f64).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}
