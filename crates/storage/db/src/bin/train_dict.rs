//! Standalone binary to train and benchmark zstd compression dictionaries from an existing Katana
//! database.
//!
//! Samples are collected by strided random sampling across the full key range, then split into
//! disjoint train/test sets so that compression benchmarks measure generalisation — not
//! memorisation.
//!
//! Usage:
//!   cargo run --bin train-dict --features cli -- --path /data/katana-mainnet-data2/

use std::path::PathBuf;

use clap::Parser;
use rand::seq::SliceRandom;
use rand::SeedableRng;

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

    /// Total number of samples to collect per table (split into train + test).
    #[arg(long, default_value_t = 100_000)]
    max_samples: usize,

    /// Fraction of samples reserved for the test set (0.0–1.0).
    #[arg(long, default_value_t = 0.2)]
    test_ratio: f64,

    /// RNG seed for reproducible sampling.
    #[arg(long, default_value_t = 42)]
    seed: u64,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    std::fs::create_dir_all(&args.output_dir)?;

    println!("Opening database at {} (read-only)...", args.path.display());
    let db = Db::open_ro(&args.path)?;

    let mut rng = rand::rngs::StdRng::seed_from_u64(args.seed);

    for (name, filename, samples) in [
        (
            "Receipts",
            "receipts_v1.dict",
            collect_random_samples::<tables::Receipts>(&db, args.max_samples, &mut rng)?,
        ),
        (
            "Transactions",
            "transactions_v1.dict",
            collect_random_samples::<tables::Transactions>(&db, args.max_samples, &mut rng)?,
        ),
    ] {
        println!("\n=== {name} ===");

        if samples.is_empty() {
            println!("No samples found, skipping.");
            continue;
        }

        // Split into train / test
        let split = ((1.0 - args.test_ratio) * samples.len() as f64) as usize;
        let (train, test) = samples.split_at(split);

        println!(
            "Collected {} total samples ({} train, {} test)",
            samples.len(),
            train.len(),
            test.len()
        );

        // Train dictionary on the train set only
        let train_refs: Vec<&[u8]> = train.iter().map(|s| s.as_slice()).collect();
        println!("Training dictionary (size={})...", args.dict_size);
        let dict = zstd::dict::from_samples(&train_refs, args.dict_size)?;

        let out_path = args.output_dir.join(filename);
        std::fs::write(&out_path, &dict)?;
        println!("Wrote dictionary to {} ({} bytes)", out_path.display(), dict.len());

        // Benchmark on test set (unseen data)
        println!("\n  -- Test set results ({} samples) --", test.len());
        print_stats(test, &dict);

        // Also show train set for comparison (to detect overfitting)
        println!("\n  -- Train set results ({} samples, for reference) --", train.len());
        print_stats(train, &dict);
    }

    println!("\nDone.");
    Ok(())
}

/// Collect samples by seeking to random keys across the full key range.
///
/// 1. Read the first and last key to determine the key range.
/// 2. Generate `max_samples` random keys uniformly distributed in that range.
/// 3. Seek to each key (lands on the nearest existing entry).
/// 4. Deduplicate and shuffle so the train/test split is random.
fn collect_random_samples<T>(
    db: &Db,
    max_samples: usize,
    rng: &mut rand::rngs::StdRng,
) -> anyhow::Result<Vec<Vec<u8>>>
where
    T: katana_db::tables::Table<Key = u64>,
    T::Value: Compress,
{
    use rand::Rng;
    use std::collections::HashSet;

    let tx = db.tx()?;
    let total = tx.entries::<T>()?;
    println!("  Table has {total} entries");

    if total == 0 {
        return Ok(Vec::new());
    }

    // Determine key range
    let mut cursor = tx.cursor::<T>()?;
    let (min_key, _) = cursor.first()?.expect("table is non-empty");
    let (max_key, _) = cursor.last()?.expect("table is non-empty");
    println!("  Key range: {min_key}..={max_key}");

    // Generate random target keys (with some oversampling to account for gaps/dedup)
    let oversample = (max_samples as f64 * 1.2) as usize;
    let mut target_keys: Vec<u64> = (0..oversample).map(|_| rng.gen_range(min_key..=max_key)).collect();
    target_keys.sort_unstable();
    target_keys.dedup();

    // Seek to each key and collect the sample
    let mut samples = Vec::with_capacity(max_samples);
    let mut seen_keys = HashSet::new();

    for target in target_keys {
        if samples.len() >= max_samples {
            break;
        }
        if let Some((actual_key, value)) = cursor.seek(target)? {
            // Avoid collecting the same entry twice (seek snaps to nearest)
            if seen_keys.insert(actual_key) {
                if let Ok(bytes) = value.compress() {
                    samples.push(bytes.into());
                }
            }
        }
    }

    // Shuffle so that train/test split is random w.r.t. key order
    samples.shuffle(rng);

    Ok(samples)
}

fn print_stats(samples: &[Vec<u8>], dict: &[u8]) {
    let total_raw: usize = samples.iter().map(|s| s.len()).sum();
    let avg_raw = total_raw as f64 / samples.len() as f64;

    // Identity (no compression, with 8-byte envelope header overhead)
    let total_identity: usize = samples.iter().map(|s| s.len() + 8).sum();

    // Compress without dictionary
    let zstd_sizes: Vec<usize> = samples
        .iter()
        .map(|s| zstd::encode_all(s.as_slice(), 0).map(|c| c.len()).unwrap_or(s.len()))
        .collect();
    let total_zstd: usize = zstd_sizes.iter().sum();

    // Compress with dictionary
    let encoder = zstd::dict::EncoderDictionary::copy(dict, 0);
    let dict_sizes: Vec<usize> = samples
        .iter()
        .map(|s| {
            let mut output = Vec::new();
            let mut enc =
                zstd::stream::Encoder::with_prepared_dictionary(&mut output, &encoder).unwrap();
            std::io::copy(&mut std::io::Cursor::new(s), &mut enc).unwrap();
            enc.finish().unwrap();
            output.len()
        })
        .collect();
    let total_dict: usize = dict_sizes.iter().sum();

    // Count how many records zstd actually expands (ratio < 1.0)
    let expanded_zstd = zstd_sizes.iter().zip(samples.iter()).filter(|(c, s)| **c >= s.len()).count();
    let expanded_dict = dict_sizes.iter().zip(samples.iter()).filter(|(c, s)| **c >= s.len()).count();

    let ratio_zstd = total_raw as f64 / total_zstd as f64;
    let ratio_dict = total_raw as f64 / total_dict as f64;

    // Per-record percentiles
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
        "    Dictionary gain vs zstd:  {:.1}%",
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
