use std::time::Duration;

use blockifier::state::cached_state::CachedState;
use criterion::measurement::WallTime;
use criterion::{criterion_group, criterion_main, BatchSize, BenchmarkGroup, Criterion};
use katana_chain_spec::ChainSpec;
use katana_executor::implementation::blockifier::state::StateProviderDb;
use katana_executor::ExecutionFlags;
use katana_primitives::env::BlockEnv;
use katana_primitives::transaction::ExecutableTxWithHash;
use katana_provider::api::state::StateFactoryProvider;
use katana_provider::{test_utils, ProviderFactory};
use pprof::criterion::{Output, PProfProfiler};

use crate::utils::{envs, tx};

mod utils;

fn executor_transact(c: &mut Criterion) {
    let mut group = c.benchmark_group("Invoke.ERC20.transfer");
    group.warm_up_time(Duration::from_millis(200));

    let provider_factory = test_utils::test_provider();
    let provider = provider_factory.provider();
    let flags = ExecutionFlags::new();

    let tx = tx();
    let envs = envs();
    let cs = ChainSpec::dev();

    blockifier(&mut group, &provider, &flags, &envs, &cs, tx);
}

fn blockifier(
    group: &mut BenchmarkGroup<'_, WallTime>,
    provider: impl StateFactoryProvider,
    execution_flags: &ExecutionFlags,
    block_envs: &BlockEnv,
    chain_spec: &ChainSpec,
    tx: ExecutableTxWithHash,
) {
    use katana_executor::implementation::blockifier::utils::{block_context_from_envs, transact};

    // convert to blockifier block context
    let block_context = block_context_from_envs(chain_spec, block_envs, None);

    group.bench_function("Blockifier.Cold", |b| {
        // we need to set up the cached state for each iteration as it's not cloneable
        b.iter_batched(
            || {
                // setup state
                let state = provider.latest().expect("failed to get latest state");
                let state = CachedState::new(StateProviderDb::new(state));

                (state, &block_context, execution_flags, tx.clone())
            },
            |(mut state, block_context, flags, tx)| {
                transact(&mut state, block_context, flags, tx, None)
            },
            BatchSize::SmallInput,
        )
    });
}

criterion_group! {
    name = benches;
    config = Criterion::default().with_profiler(PProfProfiler::new(100, Output::Flamegraph(None)));
    targets = executor_transact
}

criterion_main!(benches);
