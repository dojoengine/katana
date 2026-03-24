use std::collections::BTreeMap;
use std::time::Duration;

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use katana_primitives::contract::ContractAddress;
use katana_primitives::state::{compute_state_diff_hash, StateUpdates};
use katana_primitives::Felt;

/// Generates a realistic `StateUpdates` with the given number of contracts and storage entries per
/// contract. This simulates a moderate-to-heavy block on Starknet.
fn generate_state_updates(
    num_contracts: usize,
    storage_entries_per_contract: usize,
) -> StateUpdates {
    let mut state_updates = StateUpdates::default();

    for i in 0..num_contracts {
        let addr = ContractAddress::from(Felt::from(i as u64 + 1));

        // nonce updates
        state_updates.nonce_updates.insert(addr, Felt::from(i as u64 + 100));

        // storage updates
        let mut storage = BTreeMap::new();
        for j in 0..storage_entries_per_contract {
            storage.insert(Felt::from(j as u64), Felt::from(j as u64 + 1000));
        }
        state_updates.storage_updates.insert(addr, storage);

        // deployed contracts (first half)
        if i < num_contracts / 2 {
            state_updates.deployed_contracts.insert(addr, Felt::from(i as u64 + 500));
        }

        // declared classes (some)
        if i % 5 == 0 {
            state_updates
                .declared_classes
                .insert(Felt::from(i as u64 + 2000), Felt::from(i as u64 + 3000));
        }
    }

    state_updates
}

fn bench_compute_state_diff_hash(c: &mut Criterion) {
    let mut group = c.benchmark_group("state_diff_hash");

    // Small state update (~10 contracts, 5 storage entries each)
    let small = generate_state_updates(10, 5);
    group.bench_function("small", |b| b.iter(|| black_box(compute_state_diff_hash(&small))));

    // Medium state update (~50 contracts, 20 storage entries each)
    let medium = generate_state_updates(50, 20);
    group.bench_function("medium", |b| b.iter(|| black_box(compute_state_diff_hash(&medium))));

    // Large state update (~200 contracts, 50 storage entries each)
    let large = generate_state_updates(200, 50);
    group.bench_function("large", |b| b.iter(|| black_box(compute_state_diff_hash(&large))));

    group.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default().warm_up_time(Duration::from_millis(500)).measurement_time(Duration::from_secs(3));
    targets = bench_compute_state_diff_hash
}

criterion_main!(benches);
