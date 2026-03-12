use arbitrary::{Arbitrary, Unstructured};
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use katana_db::codecs::{Compress, Decompress};
use katana_db::models::block::StoredBlockBodyIndices;
use katana_db::models::class::MigratedCompiledClassHash;
use katana_db::models::contract::{
    ContractClassChange, ContractInfoChangeList, ContractNonceChange,
};
use katana_db::models::list::BlockList;
use katana_db::models::receipt::ReceiptEnvelope;
use katana_db::models::stage::{ExecutionCheckpoint, PruningCheckpoint};
use katana_db::models::storage::{ContractStorageEntry, StorageEntry};
use katana_db::models::trie::{
    TrieDatabaseKey, TrieDatabaseKeyType, TrieDatabaseValue, TrieHistoryEntry,
};
use katana_db::models::versioned::block::VersionedHeader;
use katana_db::models::versioned::class::VersionedContractClass;
use katana_db::models::versioned::transaction::VersionedTx;
use katana_primitives::block::{BlockHash, BlockNumber, FinalityStatus};
use katana_primitives::class::{ClassHash, CompiledClassHash};
use katana_primitives::contract::GenericContractInfo;
use katana_primitives::execution::TypedTransactionExecutionInfo;
use katana_primitives::receipt::{InvokeTxReceipt, Receipt};
use katana_primitives::transaction::{TxHash, TxNumber};
use katana_primitives::utils::class::parse_compiled_class;
use katana_primitives::Felt;
use katana_trie::bonsai::ByteVec;
use rand::Rng;

const SAMPLE_COUNT: usize = 100;

/// Generate a random byte buffer.
fn random_bytes(rng: &mut impl Rng, len: usize) -> Vec<u8> {
    let mut buf = vec![0u8; len];
    rng.fill(buf.as_mut_slice());
    buf
}

/// Generate a single Arbitrary instance from random bytes.
fn arb<'a, T: Arbitrary<'a>>(rng: &mut impl Rng) -> T {
    let buf = random_bytes(rng, 4096);
    let buf: &'static [u8] = Vec::leak(buf);
    let mut u = Unstructured::new(buf);
    T::arbitrary(&mut u).expect("failed to generate arbitrary value")
}

/// Benchmark compress and decompress for a type.
/// `$make` is an expression producing a value of type `$ty`.
/// It may use variables from the enclosing scope (e.g., `rng`).
macro_rules! bench_type {
    ($c:expr, $name:expr, $ty:ty, $make:expr) => {{
        // Pre-generate compressed bytes for decompress bench
        let compressed: Vec<Vec<u8>> = {
            let mut v = Vec::with_capacity(SAMPLE_COUNT);
            for _ in 0..SAMPLE_COUNT {
                let val: $ty = $make;
                let comp: <$ty as Compress>::Compressed = val.compress().expect("compress failed");
                v.push(AsRef::<[u8]>::as_ref(&comp).to_vec());
            }
            v
        };

        $c.bench_function(&format!("{}/compress", $name), |b| {
            b.iter(|| {
                let val: $ty = $make;
                black_box(val.compress().unwrap());
            })
        });

        $c.bench_function(&format!("{}/decompress", $name), |b| {
            let mut i = 0usize;
            b.iter(|| {
                black_box(
                    <$ty as Decompress>::decompress(compressed[i % SAMPLE_COUNT].as_slice())
                        .unwrap(),
                );
                i += 1;
            })
        });
    }};
}

// --- Existing CompiledClass benchmark (JSON fixture) ---

fn compress_compiled_class(c: &mut Criterion) {
    let json = serde_json::from_str(include_str!("./fixtures/dojo_world_240.json")).unwrap();
    let class = parse_compiled_class(json).unwrap();

    c.bench_function("CompiledClass(fixture)/compress", |b| {
        b.iter_with_large_drop(|| {
            Compress::compress(black_box(class.clone())).expect("compress failed")
        })
    });
}

fn decompress_compiled_class(c: &mut Criterion) {
    let json = serde_json::from_str(include_str!("./fixtures/dojo_world_240.json")).unwrap();
    let class = parse_compiled_class(json).unwrap();
    let compressed: Vec<u8> = Compress::compress(class).expect("compress failed");

    c.bench_function("CompiledClass(fixture)/decompress", |b| {
        b.iter_with_large_drop(|| {
            <katana_primitives::class::CompiledClass as Decompress>::decompress(black_box(
                &compressed,
            ))
            .unwrap()
        })
    });
}

// --- All value type benchmarks ---

fn bench_all_value_types(c: &mut Criterion) {
    let mut rng = rand::thread_rng();

    // Types with Arbitrary derives
    bench_type!(c, "ExecutionCheckpoint", ExecutionCheckpoint, arb(&mut rng));
    bench_type!(c, "PruningCheckpoint", PruningCheckpoint, arb(&mut rng));
    bench_type!(c, "VersionedHeader", VersionedHeader, arb(&mut rng));
    bench_type!(c, "StoredBlockBodyIndices", StoredBlockBodyIndices, arb(&mut rng));
    bench_type!(c, "VersionedTx", VersionedTx, arb(&mut rng));
    bench_type!(c, "StorageEntry", StorageEntry, arb(&mut rng));
    bench_type!(c, "ContractNonceChange", ContractNonceChange, arb(&mut rng));
    bench_type!(c, "ContractClassChange", ContractClassChange, arb(&mut rng));
    bench_type!(c, "ContractStorageEntry", ContractStorageEntry, arb(&mut rng));
    bench_type!(c, "GenericContractInfo", GenericContractInfo, arb(&mut rng));

    // Felt-based types
    bench_type!(c, "Felt", Felt, arb(&mut rng));
    bench_type!(c, "BlockHash", BlockHash, arb::<Felt>(&mut rng));
    bench_type!(c, "TxHash", TxHash, arb::<Felt>(&mut rng));
    bench_type!(c, "ClassHash", ClassHash, arb::<Felt>(&mut rng));
    bench_type!(c, "CompiledClassHash", CompiledClassHash, arb::<Felt>(&mut rng));

    // u64 types
    bench_type!(c, "BlockNumber", BlockNumber, rng.gen::<u64>());
    bench_type!(c, "TxNumber", TxNumber, rng.gen::<u64>());

    // FinalityStatus
    bench_type!(c, "FinalityStatus", FinalityStatus, {
        if rng.gen::<bool>() {
            FinalityStatus::AcceptedOnL1
        } else {
            FinalityStatus::AcceptedOnL2
        }
    });

    // TypedTransactionExecutionInfo — blockifier type, no Arbitrary
    bench_type!(
        c,
        "TypedTransactionExecutionInfo",
        TypedTransactionExecutionInfo,
        TypedTransactionExecutionInfo::default()
    );

    // VersionedContractClass — serde_json codec
    bench_type!(
        c,
        "VersionedContractClass",
        VersionedContractClass,
        VersionedContractClass::default()
    );

    // MigratedCompiledClassHash
    bench_type!(c, "MigratedCompiledClassHash", MigratedCompiledClassHash, {
        MigratedCompiledClassHash {
            class_hash: arb::<Felt>(&mut rng),
            compiled_class_hash: arb::<Felt>(&mut rng),
        }
    });

    // ContractInfoChangeList
    bench_type!(c, "ContractInfoChangeList", ContractInfoChangeList, {
        let mut class_list = BlockList::default();
        let mut nonce_list = BlockList::default();
        for j in 0..10u64 {
            class_list.insert(rng.gen::<u64>().wrapping_add(j));
            nonce_list.insert(rng.gen::<u64>().wrapping_add(j));
        }
        ContractInfoChangeList { class_change_list: class_list, nonce_change_list: nonce_list }
    });

    // BlockList
    bench_type!(c, "BlockList", BlockList, {
        let vals: [u64; 8] = std::array::from_fn(|_| rng.gen::<u64>());
        BlockList::from(vals)
    });

    // ReceiptEnvelope
    bench_type!(c, "ReceiptEnvelope", ReceiptEnvelope, {
        ReceiptEnvelope::from(Receipt::Invoke(InvokeTxReceipt {
            revert_error: None,
            events: Vec::new(),
            fee: Default::default(),
            messages_sent: Vec::new(),
            execution_resources: Default::default(),
        }))
    });

    // TrieDatabaseValue
    bench_type!(c, "TrieDatabaseValue", TrieDatabaseValue, {
        ByteVec::from(random_bytes(&mut rng, 32))
    });

    // TrieHistoryEntry
    bench_type!(c, "TrieHistoryEntry", TrieHistoryEntry, {
        TrieHistoryEntry {
            key: TrieDatabaseKey {
                r#type: TrieDatabaseKeyType::Flat,
                key: random_bytes(&mut rng, 32),
            },
            value: ByteVec::from(random_bytes(&mut rng, 32)),
        }
    });
}

criterion_group!(compiled_class, compress_compiled_class, decompress_compiled_class);
criterion_group!(all_value_types, bench_all_value_types);
criterion_main!(compiled_class, all_value_types);
