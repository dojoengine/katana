use katana_pool::ordering::FiFo;
use katana_pool::pool::Pool;
use katana_pool::validation::NoopValidator;
use katana_pool::{PoolTransaction, TransactionPool};
use katana_primitives::contract::{ContractAddress, Nonce};
use katana_primitives::transaction::TxHash;
use katana_primitives::Felt;
use katana_rpc_api::txpool::TxPoolApiServer;
use katana_rpc_server::txpool::TxPoolApi;

// -- Mock transaction type ---------------------------------------------------

#[derive(Clone, Debug)]
struct MockTx {
    hash: TxHash,
    nonce: Nonce,
    sender: ContractAddress,
    max_fee: u128,
    tip: u64,
}

impl MockTx {
    fn new(sender: ContractAddress, nonce: u64) -> Self {
        Self {
            hash: TxHash::from(Felt::from(rand::random::<u128>())),
            nonce: Nonce::from(nonce),
            sender,
            max_fee: 1000,
            tip: 10,
        }
    }
}

impl PoolTransaction for MockTx {
    fn hash(&self) -> TxHash {
        self.hash
    }
    fn nonce(&self) -> Nonce {
        self.nonce
    }
    fn sender(&self) -> ContractAddress {
        self.sender
    }
    fn max_fee(&self) -> u128 {
        self.max_fee
    }
    fn tip(&self) -> u64 {
        self.tip
    }
}

// -- Helpers -----------------------------------------------------------------

type TestPool = Pool<MockTx, NoopValidator<MockTx>, FiFo<MockTx>>;

fn test_pool() -> TestPool {
    Pool::new(NoopValidator::new(), FiFo::new())
}

fn sender_a() -> ContractAddress {
    ContractAddress::from(Felt::from(0xA))
}

fn sender_b() -> ContractAddress {
    ContractAddress::from(Felt::from(0xB))
}

// -- Tests -------------------------------------------------------------------

#[tokio::test]
async fn status_empty_pool() {
    let pool = test_pool();
    let api = TxPoolApi::new(pool);

    let status = api.txpool_status().await.unwrap();
    assert_eq!(status.pending, 0);
    assert_eq!(status.queued, 0);
}

#[tokio::test]
async fn status_after_add() {
    let pool = test_pool();
    pool.add_transaction(MockTx::new(sender_a(), 0)).await.unwrap();

    let api = TxPoolApi::new(pool);
    let status = api.txpool_status().await.unwrap();
    assert_eq!(status.pending, 1);
    assert_eq!(status.queued, 0);
}

#[tokio::test]
async fn content_populated() {
    let pool = test_pool();
    let tx = MockTx::new(sender_a(), 0);
    let expected_hash = tx.hash;
    pool.add_transaction(tx).await.unwrap();

    let api = TxPoolApi::new(pool);
    let content = api.txpool_content().await.unwrap();

    assert_eq!(content.pending.len(), 1);
    assert!(content.queued.is_empty());

    let sender_txs = content.pending.get(&sender_a()).expect("sender should be present");
    assert_eq!(sender_txs.len(), 1);

    let tx_entry = sender_txs.values().next().unwrap();
    assert_eq!(tx_entry.hash, expected_hash);
    assert_eq!(tx_entry.sender, sender_a());
    assert_eq!(tx_entry.nonce, Nonce::from(0u64));
    assert_eq!(tx_entry.max_fee, 1000);
    assert_eq!(tx_entry.tip, 10);
}

#[tokio::test]
async fn content_from_filters_by_address() {
    let pool = test_pool();
    pool.add_transaction(MockTx::new(sender_a(), 0)).await.unwrap();
    pool.add_transaction(MockTx::new(sender_b(), 0)).await.unwrap();

    let api = TxPoolApi::new(pool);

    // Filter by sender_a — should only see sender_a's transaction
    let content = api.txpool_content_from(sender_a()).await.unwrap();
    assert_eq!(content.pending.len(), 1);
    assert!(content.pending.contains_key(&sender_a()));
    assert!(!content.pending.contains_key(&sender_b()));

    // Filter by an unrelated address — should be empty
    let other = ContractAddress::from(Felt::from(0xDEAD));
    let content = api.txpool_content_from(other).await.unwrap();
    assert!(content.pending.is_empty());
}

#[tokio::test]
async fn inspect_format() {
    let pool = test_pool();
    pool.add_transaction(MockTx::new(sender_a(), 0)).await.unwrap();

    let api = TxPoolApi::new(pool);
    let inspect = api.txpool_inspect().await.unwrap();

    assert_eq!(inspect.pending.len(), 1);
    assert!(inspect.queued.is_empty());

    let summaries = inspect.pending.get(&sender_a()).expect("sender should be present");
    let summary = summaries.values().next().unwrap();

    assert!(summary.contains("hash="), "summary should contain hash: {summary}");
    assert!(summary.contains("nonce="), "summary should contain nonce: {summary}");
    assert!(summary.contains("max_fee="), "summary should contain max_fee: {summary}");
    assert!(summary.contains("tip="), "summary should contain tip: {summary}");
}

#[tokio::test]
async fn multiple_transactions_same_sender() {
    let pool = test_pool();
    for nonce in 0..3 {
        pool.add_transaction(MockTx::new(sender_a(), nonce)).await.unwrap();
    }

    let api = TxPoolApi::new(pool);

    let status = api.txpool_status().await.unwrap();
    assert_eq!(status.pending, 3);

    let content = api.txpool_content().await.unwrap();
    let sender_txs = content.pending.get(&sender_a()).expect("sender should be present");
    assert_eq!(sender_txs.len(), 3);
}

#[tokio::test]
async fn multiple_senders() {
    let pool = test_pool();
    pool.add_transaction(MockTx::new(sender_a(), 0)).await.unwrap();
    pool.add_transaction(MockTx::new(sender_a(), 1)).await.unwrap();
    pool.add_transaction(MockTx::new(sender_b(), 0)).await.unwrap();

    let api = TxPoolApi::new(pool);

    let status = api.txpool_status().await.unwrap();
    assert_eq!(status.pending, 3);

    let content = api.txpool_content().await.unwrap();
    assert_eq!(content.pending.len(), 2);
    assert_eq!(content.pending.get(&sender_a()).unwrap().len(), 2);
    assert_eq!(content.pending.get(&sender_b()).unwrap().len(), 1);
}

#[tokio::test]
async fn pool_drained_after_remove() {
    let pool = test_pool();
    let tx = MockTx::new(sender_a(), 0);
    let hash = tx.hash;
    pool.add_transaction(tx).await.unwrap();

    let api = TxPoolApi::new(pool.clone());

    let status = api.txpool_status().await.unwrap();
    assert_eq!(status.pending, 1);

    pool.remove_transactions(&[hash]);

    let status = api.txpool_status().await.unwrap();
    assert_eq!(status.pending, 0);

    let content = api.txpool_content().await.unwrap();
    assert!(content.pending.is_empty());
}
