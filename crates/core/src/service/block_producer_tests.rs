use std::task::Poll;
use std::time::Duration;

use arbitrary::{Arbitrary, Unstructured};
use futures::future::poll_fn;
use katana_chain_spec::ChainSpec;
use katana_executor::noop::NoopExecutorFactory;
use katana_gas_price_oracle::GasPriceOracle;
use katana_primitives::transaction::{ExecutableTx, InvokeTx};
use katana_primitives::Felt;
use katana_provider::DbProviderFactory;

use super::*;

fn test_backend() -> Arc<Backend<DbProviderFactory>> {
    let chain_spec = Arc::new(ChainSpec::dev());
    let executor_factory: Arc<dyn katana_executor::ExecutorFactory> =
        Arc::new(NoopExecutorFactory::new());
    let storage = DbProviderFactory::new_in_memory();
    let gas_oracle = GasPriceOracle::create_for_testing();
    let backend = Arc::new(Backend::new(chain_spec, storage, gas_oracle, executor_factory));
    backend.init_genesis(false).expect("failed to initialize genesis");
    backend
}

async fn wait_for_mined_block(producer: &BlockProducer<DbProviderFactory>) -> MinedBlockOutcome {
    tokio::time::timeout(
        Duration::from_secs(2),
        poll_fn(|cx| match producer.poll_next(cx) {
            Poll::Ready(Some(res)) => Poll::Ready(res),
            Poll::Ready(None) | Poll::Pending => Poll::Pending,
        }),
    )
    .await
    .expect("timeout waiting for mined block")
    .expect("block production should succeed")
}

#[tokio::test]
async fn pending_executor_exists_for_all_modes() {
    let backend = test_backend();

    let instant = BlockProducer::instant(backend.clone());
    let interval = BlockProducer::interval(backend.clone(), 1_000);
    let on_demand = BlockProducer::on_demand(backend);

    assert!(instant.pending_executor().is_some());
    assert!(interval.pending_executor().is_some());
    assert!(on_demand.pending_executor().is_some());
}

#[tokio::test]
async fn on_demand_force_mine_without_transactions() {
    let backend = test_backend();
    let producer = BlockProducer::on_demand(backend.clone());

    producer.force_mine();

    let outcome = wait_for_mined_block(&producer).await;
    assert_eq!(outcome.block_number, 1);
    assert_eq!(backend.storage.provider().latest_number().unwrap(), 1);
}

#[tokio::test]
async fn interval_mines_after_timer() {
    let backend = test_backend();
    let producer = BlockProducer::interval(backend.clone(), 20);

    producer.queue(vec![dummy_transaction()]);

    let outcome = wait_for_mined_block(&producer).await;
    assert_eq!(outcome.block_number, 1);
    assert_eq!(backend.storage.provider().latest_number().unwrap(), 1);
}

// Helper functions to create test transactions
fn dummy_transaction() -> ExecutableTxWithHash {
    fn tx() -> ExecutableTx {
        let data = (0..InvokeTx::size_hint(0).0).map(|_| rand::random::<u8>()).collect::<Vec<u8>>();
        let mut unstructured = Unstructured::new(&data);
        ExecutableTx::Invoke(InvokeTx::arbitrary(&mut unstructured).unwrap())
    }

    ExecutableTxWithHash { hash: Felt::ONE, transaction: tx() }
}
