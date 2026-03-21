//! Benchmarks for the static file storage layer.
//!
//! Measures write (insert_block_data) and read performance using **file-backed** databases
//! with real disk I/O. Run with:
//!
//! ```sh
//! cargo bench -p katana-provider --bench static_files
//! ```

use std::hint::black_box;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use katana_primitives::block::{Block, FinalityStatus, Header, SealedBlockWithStatus};
use katana_primitives::execution::TypedTransactionExecutionInfo;
use katana_primitives::fee::FeeInfo;
use katana_primitives::receipt::{InvokeTxReceipt, Receipt};
use katana_primitives::state::StateUpdatesWithClasses;
use katana_primitives::transaction::{InvokeTx, Tx, TxWithHash};
use katana_primitives::Felt;
use katana_provider::{DbProviderFactory, MutableProvider, ProviderFactory};
use katana_provider_api::block::{
    BlockHashProvider, BlockNumberProvider, BlockProvider, BlockWriter, HeaderProvider,
};
use katana_provider_api::transaction::{
    ReceiptProvider, TransactionProvider, TransactionTraceProvider, TransactionsProviderExt,
};

// ---------------------------------------------------------------------------
// Data generation
// ---------------------------------------------------------------------------

fn generate_block(
    block_number: u64,
    parent_hash: Felt,
    tx_count: usize,
) -> (SealedBlockWithStatus, Vec<Receipt>, Vec<TypedTransactionExecutionInfo>) {
    let mut txs = Vec::with_capacity(tx_count);
    let mut receipts = Vec::with_capacity(tx_count);
    let mut executions = Vec::with_capacity(tx_count);

    for _ in 0..tx_count {
        txs.push(TxWithHash {
            hash: Felt::from(rand::random::<u128>()),
            transaction: Tx::Invoke(InvokeTx::V1(Default::default())),
        });
        receipts.push(Receipt::Invoke(InvokeTxReceipt {
            revert_error: None,
            events: Vec::new(),
            messages_sent: Vec::new(),
            fee: FeeInfo::default(),
            execution_resources: Default::default(),
        }));
        executions.push(TypedTransactionExecutionInfo::default());
    }

    let header = Header { parent_hash, number: block_number, ..Default::default() };
    let block = Block { header, body: txs }.seal_with_hash(Felt::from(rand::random::<u128>()));

    (SealedBlockWithStatus { block, status: FinalityStatus::AcceptedOnL2 }, receipts, executions)
}

fn generate_blocks(
    count: u64,
    txs_per_block: usize,
) -> Vec<(SealedBlockWithStatus, Vec<Receipt>, Vec<TypedTransactionExecutionInfo>)> {
    let mut blocks = Vec::with_capacity(count as usize);
    let mut parent_hash = Felt::ZERO;

    for i in 0..count {
        let (block, receipts, execs) = generate_block(i, parent_hash, txs_per_block);
        parent_hash = block.block.hash;
        blocks.push((block, receipts, execs));
    }

    blocks
}

/// Create a file-backed DbProviderFactory in a temporary directory.
fn create_file_backed_factory() -> (DbProviderFactory, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("failed to create tempdir");
    let db = katana_db::Db::new(dir.path()).expect("failed to create db");
    (DbProviderFactory::new(db), dir)
}

// ---------------------------------------------------------------------------
// Write benchmark
// ---------------------------------------------------------------------------

fn bench_write(c: &mut Criterion) {
    let mut group = c.benchmark_group("write/insert_block_data");

    for &(block_count, txs_per_block) in &[(100, 1), (100, 10), (100, 50), (100, 100), (500, 100)] {
        let label = format!("{block_count}blocks_{txs_per_block}txs");
        let total_txs = block_count as u64 * txs_per_block as u64;
        group.throughput(Throughput::Elements(total_txs));

        group.bench_function(BenchmarkId::new("file_backed", &label), |b| {
            b.iter_with_setup(
                || {
                    let blocks = generate_blocks(block_count, txs_per_block);
                    let (factory, dir) = create_file_backed_factory();
                    (factory, dir, blocks)
                },
                |(factory, _dir, blocks)| {
                    for (block, receipts, executions) in blocks {
                        let p = factory.provider_mut();
                        p.insert_block_with_states_and_receipts(
                            black_box(block),
                            StateUpdatesWithClasses::default(),
                            black_box(receipts),
                            black_box(executions),
                        )
                        .unwrap();
                        p.commit().unwrap();
                    }
                },
            );
        });
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// Read benchmark
// ---------------------------------------------------------------------------

fn setup_file_backed_db(
    block_count: u64,
    txs_per_block: usize,
) -> (DbProviderFactory, tempfile::TempDir) {
    let (factory, dir) = create_file_backed_factory();
    let blocks = generate_blocks(block_count, txs_per_block);

    for (block, receipts, executions) in blocks {
        let p = factory.provider_mut();
        p.insert_block_with_states_and_receipts(
            block,
            StateUpdatesWithClasses::default(),
            receipts,
            executions,
        )
        .unwrap();
        p.commit().unwrap();
    }

    (factory, dir)
}

fn bench_read(c: &mut Criterion) {
    let block_count = 500u64;
    let txs_per_block = 100usize;
    let (factory, _dir) = setup_file_backed_db(block_count, txs_per_block);

    let mut group = c.benchmark_group("read");

    group.bench_function("latest_number", |b| {
        b.iter(|| {
            let p = factory.provider();
            black_box(p.latest_number().unwrap());
        });
    });

    group.bench_function("latest_hash", |b| {
        b.iter(|| {
            let p = factory.provider();
            black_box(p.latest_hash().unwrap());
        });
    });

    group.bench_function("header_by_number", |b| {
        b.iter(|| {
            let p = factory.provider();
            let num = rand::random::<u64>() % block_count;
            black_box(p.header(num.into()).unwrap());
        });
    });

    group.bench_function("block_body_indices", |b| {
        b.iter(|| {
            let p = factory.provider();
            let num = rand::random::<u64>() % block_count;
            black_box(p.block_body_indices(num.into()).unwrap());
        });
    });

    {
        let p = factory.provider();
        let hashes = p.transaction_hashes_in_range(0..50).unwrap();

        group.bench_function("transaction_by_hash", |b| {
            b.iter(|| {
                let p = factory.provider();
                let h = hashes[rand::random::<usize>() % hashes.len()];
                black_box(p.transaction_by_hash(h).unwrap());
            });
        });
    }

    {
        let p = factory.provider();
        let hashes = p.transaction_hashes_in_range(0..50).unwrap();

        group.bench_function("receipt_by_hash", |b| {
            b.iter(|| {
                let p = factory.provider();
                let h = hashes[rand::random::<usize>() % hashes.len()];
                black_box(p.receipt_by_hash(h).unwrap());
            });
        });
    }

    {
        let p = factory.provider();
        let hashes = p.transaction_hashes_in_range(0..50).unwrap();

        group.bench_function("transaction_execution", |b| {
            b.iter(|| {
                let p = factory.provider();
                let h = hashes[rand::random::<usize>() % hashes.len()];
                black_box(p.transaction_execution(h).unwrap());
            });
        });
    }

    group.bench_function("total_transactions", |b| {
        b.iter(|| {
            let p = factory.provider();
            black_box(p.total_transactions().unwrap());
        });
    });

    group.bench_function("full_block", |b| {
        b.iter(|| {
            let p = factory.provider();
            let num = rand::random::<u64>() % block_count;
            black_box(p.block(num.into()).unwrap());
        });
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// Entry
// ---------------------------------------------------------------------------

criterion_group!(
    name = benches;
    config = Criterion::default().sample_size(20);
    targets = bench_write, bench_read
);
criterion_main!(benches);
