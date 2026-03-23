//! Standalone binary to train and benchmark zstd compression dictionaries from an existing Katana
//! database.
//!
//! **Subcommands:**
//!
//! - `train` — Train dictionaries from random samples and write them to disk.
//! - `pareto` — Systematic exploration: vary training range, dict size, and sample count, then
//!   evaluate every combination against a single held-out random test set. Outputs SVG charts.
//!
//! Usage:
//!   cargo run --release --bin train-dict --features cli -- train  --path
//! /data/katana-mainnet-data2/   cargo run --release --bin train-dict --features cli -- pareto
//! --path /data/katana-mainnet-data2/

use std::collections::HashSet;
use std::fmt::Write as FmtWrite;
use std::path::PathBuf;

use clap::{Parser, Subcommand};
use katana_db::abstraction::{Database, DbCursor, DbTx};
use katana_db::codecs::Compress;
use katana_db::{tables, Db};
use rand::seq::SliceRandom;
use rand::{Rng, SeedableRng};

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

        /// Output directory to save the best dictionary and charts for each table.
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

/// Print database summary: latest block number and table entry counts.
fn print_db_info(db: &Db) {
    let tx = db.tx().expect("failed to open read transaction");
    let mut cursor = tx.cursor::<tables::Headers>().expect("failed to open Headers cursor");
    if let Ok(Some((last_block, _))) = cursor.last() {
        println!("Latest block: {last_block}");
    }
    if let Ok(n) = tx.entries::<tables::Receipts>() {
        println!("Receipts entries: {n}");
    }
    if let Ok(n) = tx.entries::<tables::Transactions>() {
        println!("Transactions entries: {n}");
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
    print_db_info(&db);
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

#[derive(Clone)]
struct TrainRange {
    label: &'static str,
    start_frac: f64,
    end_frac: f64,
}

#[derive(Clone)]
struct ParetoRow {
    range: String,
    dict_size_kb: usize,
    train_n: usize,
    gain_vs_identity: f64,
}

struct ParetoResults {
    table_name: String,
    rows: Vec<ParetoRow>,
    best_dict: Option<Vec<u8>>,
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
    print_db_info(&db);

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

    let mut all_results: Vec<ParetoResults> = Vec::new();

    for (table_name, dict_filename, seed_offset) in
        [("Receipts", "receipts_v1.dict", 0u64), ("Transactions", "transactions_v1.dict", 1)]
    {
        let results = match table_name {
            "Receipts" => pareto_for_table::<tables::Receipts>(
                &db,
                table_name,
                &ranges,
                &dict_sizes,
                &train_counts,
                test_samples,
                seed + seed_offset,
            )?,
            _ => pareto_for_table::<tables::Transactions>(
                &db,
                table_name,
                &ranges,
                &dict_sizes,
                &train_counts,
                test_samples,
                seed + seed_offset,
            )?,
        };

        if let Some(ref dict) = results.best_dict {
            let out_path = output_dir.join(dict_filename);
            std::fs::write(&out_path, dict)?;
            println!("Saved best dictionary to {}", out_path.display());
        }

        all_results.push(results);
    }

    // Generate charts
    generate_dict_size_chart(&all_results, &dict_sizes, output_dir)?;
    generate_range_chart(&all_results, &ranges, output_dir)?;

    println!("\nDone.");
    Ok(())
}

fn pareto_for_table<T>(
    db: &Db,
    table_name: &str,
    ranges: &[TrainRange],
    dict_sizes: &[usize],
    train_counts: &[usize],
    test_samples: usize,
    seed: u64,
) -> anyhow::Result<ParetoResults>
where
    T: katana_db::tables::Table<Key = u64>,
    T::Value: Compress,
{
    println!("\n{}", "=".repeat(60));
    println!("  PARETO EXPLORATION: {table_name}");
    println!("{}", "=".repeat(60));

    let mut rng = StdRng::seed_from_u64(seed);

    let tx = db.tx()?;
    let total = tx.entries::<T>()?;
    let mut cursor = tx.cursor::<T>()?;
    let (min_key, _) = cursor.first()?.expect("table is non-empty");
    let (max_key, _) = cursor.last()?.expect("table is non-empty");
    drop(cursor);
    drop(tx);

    println!("Table: {total} entries, key range {min_key}..={max_key}");

    println!("Collecting {test_samples} random test samples across full range...");
    let test_set = collect_range_samples::<T>(db, min_key, max_key, test_samples, &mut rng)?;
    println!("Test set: {} samples", test_set.len());

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

    println!(
        "{:<14} {:>10} {:>10} {:>12} {:>12} {:>8} {:>8}",
        "range", "dict_size", "train_n", "test_bytes", "vs_ident%", "vs_zstd%", "ratio"
    );
    println!("{}", "-".repeat(82));

    let mut best_score = usize::MAX;
    let mut best_dict: Option<Vec<u8>> = None;
    let mut best_label = String::new();
    let mut rows = Vec::new();

    for range in ranges {
        let range_start = min_key + ((max_key - min_key) as f64 * range.start_frac) as u64;
        let range_end = min_key + ((max_key - min_key) as f64 * range.end_frac) as u64;

        for &train_n in train_counts {
            let mut train_rng = StdRng::seed_from_u64(
                seed.wrapping_add((range.label.len() as u64) * 1000 + train_n as u64),
            );
            let train_set =
                collect_range_samples::<T>(db, range_start, range_end, train_n, &mut train_rng)?;

            if train_set.len() < 100 {
                continue;
            }

            for &dict_size in dict_sizes {
                let train_refs: Vec<&[u8]> = train_set.iter().map(|s| s.as_slice()).collect();
                let dict = match zstd::dict::from_samples(&train_refs, dict_size) {
                    Ok(d) => d,
                    Err(_) => continue,
                };

                let total_dict = compress_all_with_dict(&test_set, &dict);

                let gain_vs_identity = (1.0 - total_dict as f64 / total_identity as f64) * 100.0;
                let gain_vs_zstd = (1.0 - total_dict as f64 / total_zstd as f64) * 100.0;
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

                rows.push(ParetoRow {
                    range: range.label.to_string(),
                    dict_size_kb: dict_size / 1024,
                    train_n: train_set.len(),
                    gain_vs_identity,
                });

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

    if let Some(ref dict) = best_dict {
        println!("\nDetailed stats for best dictionary:");
        print_stats(&test_set, dict);
    }

    Ok(ParetoResults { table_name: table_name.to_string(), rows, best_dict })
}

// ── SVG chart generation ────────────────────────────────────────────────────

/// Line chart: dict size (x) vs savings % (y), one line per table.
fn generate_dict_size_chart(
    all_results: &[ParetoResults],
    dict_sizes: &[usize],
    output_dir: &PathBuf,
) -> anyhow::Result<()> {
    let colors = ["#2196F3", "#FF9800"];
    let w = 700.0f64;
    let h = 420.0;
    let ml = 70.0;
    let mr = 90.0;
    let mt = 50.0;
    let mb = 60.0;
    let pw = w - ml - mr;
    let ph = h - mt - mb;
    let y_min = 20.0f64;
    let y_max = 55.0;

    let labels: Vec<String> = dict_sizes.iter().map(|s| format!("{}", s / 1024)).collect();
    let n = labels.len();

    let mut s = String::new();
    writeln!(
        s,
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {w} {h}" font-family="system-ui,sans-serif">"##
    )?;
    writeln!(s, r##"<rect width="{w}" height="{h}" fill="#fafafa" rx="6"/>"##)?;
    writeln!(
        s,
        r##"<text x="{}" y="30" text-anchor="middle" font-size="15" font-weight="bold" fill="#1a1a1a">Pareto Frontier: Dictionary Size vs Compression Savings</text>"##,
        w / 2.0
    )?;

    // Y grid + labels
    for i in 0..=7 {
        let v = y_min + (y_max - y_min) * i as f64 / 7.0;
        let y = mt + ph - (v - y_min) / (y_max - y_min) * ph;
        writeln!(s, r##"<line x1="{ml}" y1="{y}" x2="{}" y2="{y}" stroke="#e0e0e0"/>"##, ml + pw)?;
        writeln!(
            s,
            r##"<text x="{}" y="{}" text-anchor="end" font-size="11" fill="#666">{v:.0}%</text>"##,
            ml - 8.0,
            y + 4.0
        )?;
    }
    // X labels
    for (i, l) in labels.iter().enumerate() {
        let x = ml + (i as f64 / (n - 1) as f64) * pw;
        writeln!(
            s,
            r##"<text x="{x}" y="{}" text-anchor="middle" font-size="11" fill="#666">{l} KB</text>"##,
            mt + ph + 20.0
        )?;
    }
    // Axis titles
    writeln!(
        s,
        r##"<text x="{}" y="{}" text-anchor="middle" font-size="12" fill="#444">Dictionary Size</text>"##,
        ml + pw / 2.0,
        h - 10.0
    )?;
    writeln!(
        s,
        r##"<text x="18" y="{}" text-anchor="middle" font-size="12" fill="#444" transform="rotate(-90 18 {})">Savings vs Identity (%)</text>"##,
        mt + ph / 2.0,
        mt + ph / 2.0
    )?;

    for (idx, res) in all_results.iter().enumerate() {
        let c = colors[idx % 2];
        // Best row per dict size (full range, max train_n)
        let vals: Vec<f64> = dict_sizes
            .iter()
            .map(|ds| {
                let kb = ds / 1024;
                res.rows
                    .iter()
                    .filter(|r| r.dict_size_kb == kb && r.range == "full")
                    .max_by(|a, b| a.train_n.cmp(&b.train_n))
                    .map(|r| r.gain_vs_identity)
                    .unwrap_or(0.0)
            })
            .collect();

        let pts: Vec<(f64, f64)> = vals
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let x = ml + (i as f64 / (n - 1) as f64) * pw;
                let y = mt + ph - (v - y_min) / (y_max - y_min) * ph;
                (x, y)
            })
            .collect();

        // Fill
        let mut d = format!("M{:.1},{:.1}", pts[0].0, mt + ph);
        for (x, y) in &pts {
            write!(d, " L{x:.1},{y:.1}")?;
        }
        write!(d, " L{:.1},{:.1}Z", pts.last().unwrap().0, mt + ph)?;
        writeln!(s, r##"<path d="{d}" fill="{c}" opacity="0.1"/>"##)?;

        // Line
        let mut d = format!("M{:.1},{:.1}", pts[0].0, pts[0].1);
        for (x, y) in &pts[1..] {
            write!(d, " L{x:.1},{y:.1}")?;
        }
        writeln!(s, r##"<path d="{d}" fill="none" stroke="{c}" stroke-width="2.5"/>"##)?;

        // Points + end label
        for (i, ((x, y), v)) in pts.iter().zip(vals.iter()).enumerate() {
            writeln!(s, r##"<circle cx="{x}" cy="{y}" r="4" fill="{c}"/>"##)?;
            if i == pts.len() - 1 {
                writeln!(
                    s,
                    r##"<text x="{}" y="{}" font-size="12" font-weight="bold" fill="{c}">{v:.1}%</text>"##,
                    x + 8.0,
                    y + 4.0
                )?;
            }
        }

        // Legend
        let ly = mt + 15.0 + idx as f64 * 22.0;
        let lx = ml + 10.0;
        writeln!(
            s,
            r##"<line x1="{lx}" y1="{ly}" x2="{}" y2="{ly}" stroke="{c}" stroke-width="2.5"/>"##,
            lx + 20.0
        )?;
        writeln!(s, r##"<circle cx="{}" cy="{ly}" r="3" fill="{c}"/>"##, lx + 10.0)?;
        writeln!(
            s,
            r##"<text x="{}" y="{}" font-size="11" fill="#333">{}</text>"##,
            lx + 26.0,
            ly + 4.0,
            res.table_name
        )?;
    }

    writeln!(s, "</svg>")?;
    let path = output_dir.join("pareto_dict_size.svg");
    std::fs::write(&path, &s)?;
    println!("Chart saved to {}", path.display());
    Ok(())
}

/// Grouped bar chart: training range (x) vs savings % (y), one color per table.
fn generate_range_chart(
    all_results: &[ParetoResults],
    ranges: &[TrainRange],
    output_dir: &PathBuf,
) -> anyhow::Result<()> {
    let colors = ["#2196F3", "#FF9800"];
    let ng = ranges.len();
    let nb = all_results.len();
    let w = 750.0f64;
    let h = 420.0;
    let ml = 70.0;
    let mr = 30.0;
    let mt = 50.0;
    let mb = 70.0;
    let pw = w - ml - mr;
    let ph = h - mt - mb;
    let y_max = 55.0f64;
    let gw = pw / ng as f64;
    let bw = gw * 0.35;
    let gap = gw * 0.08;

    let mut s = String::new();
    writeln!(
        s,
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {w} {h}" font-family="system-ui,sans-serif">"##
    )?;
    writeln!(s, r##"<rect width="{w}" height="{h}" fill="#fafafa" rx="6"/>"##)?;
    writeln!(
        s,
        r##"<text x="{}" y="30" text-anchor="middle" font-size="15" font-weight="bold" fill="#1a1a1a">Training Range Impact (128 KB dict, 100k samples)</text>"##,
        w / 2.0
    )?;

    // Y grid
    for i in 0..=5 {
        let v = y_max * i as f64 / 5.0;
        let y = mt + ph - v / y_max * ph;
        writeln!(s, r##"<line x1="{ml}" y1="{y}" x2="{}" y2="{y}" stroke="#e0e0e0"/>"##, ml + pw)?;
        writeln!(
            s,
            r##"<text x="{}" y="{}" text-anchor="end" font-size="11" fill="#666">{v:.0}%</text>"##,
            ml - 8.0,
            y + 4.0
        )?;
    }
    writeln!(
        s,
        r##"<text x="{}" y="{}" text-anchor="middle" font-size="12" fill="#444">Training Range</text>"##,
        ml + pw / 2.0,
        h - 8.0
    )?;
    writeln!(
        s,
        r##"<text x="18" y="{}" text-anchor="middle" font-size="12" fill="#444" transform="rotate(-90 18 {})">Savings vs Identity (%)</text>"##,
        mt + ph / 2.0,
        mt + ph / 2.0
    )?;

    for (gi, range) in ranges.iter().enumerate() {
        let gx = ml + gi as f64 * gw + gw / 2.0;
        writeln!(
            s,
            r##"<text x="{gx}" y="{}" text-anchor="middle" font-size="10" fill="#666">{}</text>"##,
            mt + ph + 18.0,
            range.label
        )?;

        for (bi, res) in all_results.iter().enumerate() {
            let c = colors[bi % 2];
            let v = res
                .rows
                .iter()
                .filter(|r| r.range == range.label && r.dict_size_kb == 128)
                .max_by(|a, b| a.train_n.cmp(&b.train_n))
                .map(|r| r.gain_vs_identity)
                .unwrap_or(0.0);

            let tbw = nb as f64 * bw + (nb - 1) as f64 * gap;
            let bx = gx - tbw / 2.0 + bi as f64 * (bw + gap);
            let bh = v / y_max * ph;
            let by = mt + ph - bh;

            writeln!(
                s,
                r##"<rect x="{bx}" y="{by}" width="{bw}" height="{bh}" fill="{c}" rx="2"/>"##
            )?;
            writeln!(
                s,
                r##"<text x="{}" y="{}" text-anchor="middle" font-size="9" font-weight="bold" fill="{c}">{v:.1}%</text>"##,
                bx + bw / 2.0,
                by - 5.0
            )?;
        }
    }

    // Legend
    for (i, res) in all_results.iter().enumerate() {
        let c = colors[i % 2];
        let lx = ml + pw - 130.0;
        let ly = mt + 15.0 + i as f64 * 22.0;
        writeln!(
            s,
            r##"<rect x="{lx}" y="{}" width="14" height="14" fill="{c}" rx="2"/>"##,
            ly - 10.0
        )?;
        writeln!(
            s,
            r##"<text x="{}" y="{}" font-size="11" fill="#333">{}</text>"##,
            lx + 20.0,
            ly + 1.0,
            res.table_name
        )?;
    }

    writeln!(s, "</svg>")?;
    let path = output_dir.join("pareto_training_range.svg");
    std::fs::write(&path, &s)?;
    println!("Chart saved to {}", path.display());
    Ok(())
}

// ── Shared helpers ──────────────────────────────────────────────────────────

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
                continue;
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
        "    Zstd (no dict):     {total_zstd:>12} bytes  (ratio: {ratio_zstd:.3}x, \
         {expanded_zstd}/{} expanded)",
        samples.len()
    );
    println!(
        "    Zstd (w/ dict):     {total_dict:>12} bytes  (ratio: {ratio_dict:.3}x, \
         {expanded_dict}/{} expanded)",
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
