use jsonrpsee::http_client::HttpClientBuilder;
use katana_primitives::block::{BlockHash, BlockIdOrTag, BlockNumber};
use katana_primitives::class::ClassHash;
use katana_primitives::contract::ContractAddress;
use katana_primitives::transaction::TxHash;
use katana_primitives::Felt;
use katana_provider::error::ProviderError;
use katana_rpc_api::error::starknet::StarknetApiError;
use katana_rpc_client::starknet::Client;
use katana_rpc_types::block::{
    GetBlockWithReceiptsResponse, GetBlockWithTxHashesResponse, MaybePreConfirmedBlock,
};
use katana_rpc_types::event::{EventFilter, GetEventsResponse};
use katana_rpc_types::receipt::{ReceiptBlockInfo, TxReceiptWithBlockInfo};
use katana_rpc_types::state_update::StateUpdate;
use katana_rpc_types::transaction::RpcTxWithHash;
use katana_rpc_types::TxStatus;
use jsonrpsee::http_client::HttpClientBuilder;
use katana_rpc_api::starknet::StarknetApiClient;
use jsonrpsee::core::Error as JsonRpcseError;
use crate::starknet::BlockTag;
use katana_rpc_types::trie::{ContractStorageKeys, GetStorageProofResponse};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("rpc client error: {0}")]
    RpcClient(#[from] katana_rpc_client::starknet::Error),

    #[error("block out of range")]
    BlockOutOfRange,

    #[error("not allowed to use block tag as a block identifier")]
    BlockTagNotAllowed,

    #[error("unexpected pending data")]
    UnexpectedPendingData,

    /// Error from jsonrpsee client
    #[error("JsonRPC client error: {0}")]
    JsonRpc(#[from] JsonRpcseError),

    /// Error from katana provider
    #[error("Katana provider error: {0}")]
    KatanaProvider(#[from] ProviderError),
}

#[derive(Debug, Clone)]
pub struct ForkedClient {
    /// The block number where the node is forked from.
    block: BlockNumber,
    /// The Starknet JSON-RPC client for doing the request to the forked network.
    client: Client,
    /// The URL of the forked network (only set when using JsonRpcClient<HttpTransport>).
    url: Option<Url>,
}

impl ForkedClient {
    /// Creates a new forked client from the given [`Client`] and block number.
    pub fn new(client: Client, block: BlockNumber) -> Self {
        Self { block, client, url: None }
    }

    /// Returns the block number of the forked client.
    pub fn block(&self) -> &BlockNumber {
        &self.block
    }
}

impl ForkedClient {
       /// Creates a new forked client from the given HTTP URL and block number.
       pub fn new_http(url: Url, block: BlockNumber) -> Self {
        Self { 
            client: Client::new(HttpClientBuilder::new().build(url.clone()).unwrap()), 
            block,
            url: Some(url),
        }
    }

    pub async fn get_block_number_by_hash(&self, hash: BlockHash) -> Result<BlockNumber, Error> {
        let number = match self.client.get_block_with_tx_hashes(BlockIdOrTag::Hash(hash)).await? {
            GetBlockWithTxHashesResponse::Block(block) => block.block_number,
            GetBlockWithTxHashesResponse::PreConfirmed(block) => block.block_number,
        };

        if number > self.block {
            Err(Error::BlockOutOfRange)
        } else {
            Ok(number)
        }
    }

    pub async fn get_transaction_by_hash(&self, hash: TxHash) -> Result<RpcTxWithHash, Error> {
        Ok(self.client.get_transaction_by_hash(hash).await?)
    }

    pub async fn get_transaction_receipt(
        &self,
        hash: TxHash,
    ) -> Result<TxReceiptWithBlockInfo, Error> {
        let receipt = self.client.get_transaction_receipt(hash).await?;

        if let ReceiptBlockInfo::Block { block_number, .. } = receipt.block {
            if block_number > self.block {
                return Err(Error::BlockOutOfRange);
            }
        }

        Ok(receipt)
    }

    pub async fn get_transaction_status(&self, hash: TxHash) -> Result<TxStatus, Error> {
        let (receipt, status) = tokio::join!(
            self.get_transaction_receipt(hash),
            self.client.get_transaction_status(hash)
        );

        // We get the receipt first to check if the block number is within the forked range.
        let _ = receipt?;

        Ok(status?)
    }

    pub async fn get_transaction_by_block_id_and_index(
        &self,
        block_id: BlockIdOrTag,
        idx: u64,
    ) -> Result<RpcTxWithHash, Error> {
        match block_id {
            BlockIdOrTag::Number(num) => {
                if num > self.block {
                    return Err(Error::BlockOutOfRange);
                }

                Ok(self.client.get_transaction_by_block_id_and_index(block_id, idx).await?)
            }

            BlockIdOrTag::Hash(hash) => {
                let (block, tx) = tokio::join!(
                    self.client.get_block_with_tx_hashes(BlockIdOrTag::Hash(hash)),
                    self.client.get_transaction_by_block_id_and_index(block_id, idx)
                );

                let number = match block? {
                    GetBlockWithTxHashesResponse::Block(block) => block.block_number,
                    GetBlockWithTxHashesResponse::PreConfirmed(_) => {
                        return Err(Error::UnexpectedPendingData);
                    }
                };

                if number > self.block {
                    return Err(Error::BlockOutOfRange);
                }

                Ok(tx?)
            }

            BlockIdOrTag::L1Accepted | BlockIdOrTag::Latest | BlockIdOrTag::PreConfirmed => {
                Err(Error::BlockTagNotAllowed)
            }
        }
    }

    pub async fn get_block_with_txs(
        &self,
        block_id: BlockIdOrTag,
    ) -> Result<MaybePreConfirmedBlock, Error> {
        let block = self.client.get_block_with_txs(block_id).await?;

        match block {
            MaybePreConfirmedBlock::PreConfirmed(_) => Err(Error::UnexpectedPendingData),
            MaybePreConfirmedBlock::Confirmed(ref b) => {
                if b.block_number > self.block {
                    Err(Error::BlockOutOfRange)
                } else {
                    Ok(block)
                }
            }
        }
    }

    pub async fn get_block_with_receipts(
        &self,
        block_id: BlockIdOrTag,
    ) -> Result<GetBlockWithReceiptsResponse, Error> {
        let block = self.client.get_block_with_receipts(block_id).await?;

        match block {
            GetBlockWithReceiptsResponse::Block(ref b) => {
                if b.block_number > self.block {
                    return Err(Error::BlockOutOfRange);
                }
            }
            GetBlockWithReceiptsResponse::PreConfirmed(_) => {
                return Err(Error::UnexpectedPendingData);
            }
        }

        Ok(block)
    }

    pub async fn get_block_with_tx_hashes(
        &self,
        block_id: BlockIdOrTag,
    ) -> Result<GetBlockWithTxHashesResponse, Error> {
        let block = self.client.get_block_with_tx_hashes(block_id).await?;

        match block {
            GetBlockWithTxHashesResponse::Block(ref b) => {
                if b.block_number > self.block {
                    return Err(Error::BlockOutOfRange);
                }
            }
            GetBlockWithTxHashesResponse::PreConfirmed(_) => {
                return Err(Error::UnexpectedPendingData);
            }
        }

        Ok(block)
    }

    pub async fn get_block_transaction_count(&self, block_id: BlockIdOrTag) -> Result<u64, Error> {
        match block_id {
            BlockIdOrTag::Number(num) if num > self.block => {
                return Err(Error::BlockOutOfRange);
            }
            BlockIdOrTag::Hash(hash) => {
                let block = self.client.get_block_with_tx_hashes(BlockIdOrTag::Hash(hash)).await?;
                if let GetBlockWithTxHashesResponse::Block(b) = block {
                    if b.block_number > self.block {
                        return Err(Error::BlockOutOfRange);
                    }
                }
            }
            BlockIdOrTag::L1Accepted | BlockIdOrTag::Latest | BlockIdOrTag::PreConfirmed => {
                return Err(Error::BlockTagNotAllowed);
            }
            BlockIdOrTag::Number(_) => {}
        }

        Ok(self.client.get_block_transaction_count(block_id).await?)
    }

    pub async fn get_state_update(&self, block_id: BlockIdOrTag) -> Result<StateUpdate, Error> {
        match block_id {
            BlockIdOrTag::Number(num) if num > self.block => {
                return Err(Error::BlockOutOfRange);
            }
            BlockIdOrTag::Hash(hash) => {
                let block = self.client.get_block_with_tx_hashes(BlockIdOrTag::Hash(hash)).await?;
                if let GetBlockWithTxHashesResponse::Block(b) = block {
                    if b.block_number > self.block {
                        return Err(Error::BlockOutOfRange);
                    }
                }
            }
            BlockIdOrTag::L1Accepted | BlockIdOrTag::Latest | BlockIdOrTag::PreConfirmed => {
                return Err(Error::BlockTagNotAllowed);
            }
            BlockIdOrTag::Number(_) => {}
        }

        Ok(self.client.get_state_update(block_id).await?)
    }

    // NOTE(kariy): The reason why I don't just use EventFilter as a param, bcs i wanna make sure
    // the from/to blocks are not None. maybe should do the same for other methods that accept a
    // BlockId in some way?
    pub async fn get_events(
        &self,
        from: BlockNumber,
        to: BlockNumber,
        address: Option<ContractAddress>,
        keys: Option<Vec<Vec<Felt>>>,
        continuation_token: Option<String>,
        chunk_size: u64,
    ) -> Result<GetEventsResponse, Error> {
        if from > self.block || to > self.block {
            return Err(Error::BlockOutOfRange);
        }

        let from_block = Some(BlockIdOrTag::Number(from));
        let to_block = Some(BlockIdOrTag::Number(to));

        let event_filter = EventFilter { address, from_block, to_block, keys };

        Ok(self.client.get_events(event_filter, continuation_token, chunk_size).await?)
    }
}

impl From<Error> for StarknetApiError {
    fn from(value: Error) -> Self {
        match value {
            Error::RpcClient(katana_rpc_client::starknet::Error::Starknet(err)) => err,
            Error::RpcClient(err) => StarknetApiError::unexpected(err),
            Error::BlockOutOfRange => StarknetApiError::BlockNotFound,
            Error::BlockTagNotAllowed | Error::UnexpectedPendingData => {
                StarknetApiError::unexpected(value)
            }
            Error::JsonRpc(json_rpc_error) => {
                StarknetApiError::UnexpectedError { reason: json_rpc_error.to_string() }
            }
            Error::KatanaProvider(provider_error) => provider_error.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use jsonrpsee::http_client::HttpClientBuilder;
    use katana_primitives::felt;
    use katana_rpc_types::trie::ContractStorageKeys;
    use katana_provider::providers::db::DbProvider;
    use url::Url;
    use proptest::proptest;
    use proptest::prelude::ProptestConfig;
    use katana_core::backend::storage::Blockchain;
    use katana_provider::traits::trie::TrieWriter;
    use katana_provider::traits::block::BlockNumberProvider;
    use katana_utils::TestNode;
    use katana_utils::node::test_config_forking;
    use proptest::prop_assert_eq;
    use proptest::prelude::Strategy;
    use proptest::prelude::Just;
    use proptest::arbitrary::any;
    use std::collections::BTreeSet;
    use super::*;

    // const SEPOLIA_URL: &str = "https://api.cartridge.gg/x/starknet/sepolia";
    const SEPOLIA_URL: &str = "https://rpc.starknet-testnet.lava.build:443";

    const FORK_BLOCK_NUMBER: BlockNumber = 268_471;

    #[tokio::test]
    async fn get_block_hash() {
        let http_client = HttpClientBuilder::new().build(SEPOLIA_URL).unwrap();
        let rpc_client = Client::new(http_client);
        let client = ForkedClient::new(rpc_client, FORK_BLOCK_NUMBER);

        // -----------------------------------------------------------------------
        // Block before the forked block

        // https://sepolia.voyager.online/block/0x4dfd88ba652622450c7758b49ac4a2f23b1fa8e6676297333ea9c97d0756c7a
        let hash = felt!("0x4dfd88ba652622450c7758b49ac4a2f23b1fa8e6676297333ea9c97d0756c7a");
        let number =
            client.get_block_number_by_hash(hash).await.expect("failed to get block number");
        assert_eq!(number, 268469);

        // -----------------------------------------------------------------------
        // Block after the forked block (exists only in the forked chain)

        // https://sepolia.voyager.online/block/0x335a605f2c91873f8f830a6e5285e704caec18503ca28c18485ea6f682eb65e
        let hash = felt!("0x335a605f2c91873f8f830a6e5285e704caec18503ca28c18485ea6f682eb65e");
        let err = client.get_block_number_by_hash(hash).await.expect_err("should return an error");
        assert!(matches!(err, Error::BlockOutOfRange));
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_commit_new_state_root_mainnet_blockchain_and_forked_provider() {
        use katana_primitives::state::StateUpdates;
        use katana_provider::providers::fork::ForkedProvider;
        use katana_provider::traits::trie::TrieWriter;
        use std::collections::BTreeMap;
        use katana_provider::traits::block::BlockNumberProvider;
        use katana_utils::TestNode;

        let sequencer = TestNode::new().await;
        let backend = sequencer.backend();
        let blockchain = sequencer.blockchain();
        let starknet_provider = sequencer.starknet_provider();
        let provider = backend.blockchain.provider();
    
        let url = format!("http://{}", sequencer.rpc_addr());
        let url = Url::parse(&url).unwrap();
        
        let block_number = provider.latest_number().unwrap();
        println!("Block number from provider: {:?}", block_number);

        // Generate random state updates
        let state_updates = setup_mainnet_updates_randomized(5);
        
        println!("📊 Enhanced state updates with {} contracts, {} storage entries, {} nonces, {} declared classes, {} deprecated classes, {} replaced classes", 
            state_updates.deployed_contracts.len(),
            state_updates.storage_updates.values().map(|s| s.len()).sum::<usize>(),
            state_updates.nonce_updates.len(),
            state_updates.declared_classes.len(),
            state_updates.deprecated_declared_classes.len(),
            state_updates.replaced_classes.len()
        );

        let mainnet_provider = provider;
        //init first state for mainnet
        mainnet_provider.compute_state_root(block_number, &state_updates).unwrap();
        
        // Create minimal fork updates with one example from each category
        let mut fork_minimal_updates = StateUpdates::default();
        
        let fork_minimal_updates = setup_mainnet_updates_randomized(5);

        println!("📊 Minimal fork updates with {} contracts, {} storage entries, {} nonces, {} declared classes, {} deprecated classes, {} replaced classes", 
            fork_minimal_updates.deployed_contracts.len(),
            fork_minimal_updates.storage_updates.values().map(|s| s.len()).sum::<usize>(),
            fork_minimal_updates.nonce_updates.len(),
            fork_minimal_updates.declared_classes.len(),
            fork_minimal_updates.deprecated_declared_classes.len(),
            fork_minimal_updates.replaced_classes.len()
        );

        let db = katana_db::init_ephemeral_db().unwrap();
        let forked_provider = ForkedProvider::new(
            db.clone(),
            katana_primitives::block::BlockHashOrNumber::Num(block_number),
            starknet_provider,
            url.clone(),
        );

        let state_root = forked_provider.compute_state_root(block_number, &fork_minimal_updates).unwrap();
        println!("Forked state root: {:?}", state_root);

        let mainnet_state_root_same_updates = mainnet_provider.compute_state_root(block_number, &fork_minimal_updates).unwrap();
        println!("Mainnet state root same updates to compare: {:?}", mainnet_state_root_same_updates);

        if state_root == mainnet_state_root_same_updates {
            println!("✅ State roots match!");
        } else {
            println!("❌ State roots do NOT match!");
        }
        assert!(state_root == mainnet_state_root_same_updates, "State roots do not match on first run");

        // Second iteration with new random updates
        let state_updates = setup_mainnet_updates_randomized(5);
        //IT's important here to compute state root for forked network first, then for mainnet
        //otherwise it will be different roots because it's like double computation of same changes
        let fork_state_root = forked_provider.compute_state_root(block_number, &state_updates).unwrap();
        let mainnet_state_root = mainnet_provider.compute_state_root(block_number, &state_updates).unwrap();

        println!("Mainnet state root: {:?}", mainnet_state_root);
        println!("Fork state root: {:?}", fork_state_root);

        if mainnet_state_root == fork_state_root {
            println!("✅ State roots match!");
        } else {
            println!("❌ State roots do NOT match!");
        }
        assert!(mainnet_state_root == fork_state_root, "State roots do not match on second run");
    }

    fn setup_mainnet_updates_randomized(num_contracts: usize) -> StateUpdates {
        let mut state_updates = StateUpdates::default();

        for _ in 0..num_contracts {
            let (address, class_hash, storage, nonce) = random_contract();
            state_updates.deployed_contracts.insert(address, class_hash);
            state_updates.storage_updates.insert(address, storage);
            state_updates.declared_classes.insert(class_hash, random_felt());
            state_updates.nonce_updates.insert(address, nonce);
            if thread_rng().gen_bool(0.2) {
                let new_class_hash = random_class_hash();
                state_updates.replaced_classes.insert(address, new_class_hash);
                state_updates.declared_classes.insert(new_class_hash, random_felt());
            }
            if thread_rng().gen_bool(0.2) {
                state_updates.deprecated_declared_classes.insert(random_class_hash());
            }
        }

        state_updates
    }

    fn random_felt() -> Felt {
        let mut bytes = [0u8; 32];
        rand::thread_rng().fill(&mut bytes);
        Felt::from_bytes_be(&bytes)
    }

    fn random_class_hash() -> ClassHash {
        ClassHash::from(random_felt())
    }

    fn random_contract_address() -> ContractAddress {
        ContractAddress::from(random_felt())
    }

    fn random_contract() -> (ContractAddress, ClassHash, BTreeMap<Felt, Felt>, Felt) {
        let address = random_contract_address();
        let class_hash = random_class_hash();
        let mut storage = BTreeMap::new();
        for _ in 0..thread_rng().gen_range(1..=3) {
            storage.insert(random_felt(), random_felt());
        }
        let nonce = random_felt();
        (address, class_hash, storage, nonce)
    }


    /// To run this test you need to comment out global cache part in Node::buil()
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_commit_new_state_root_two_katana_instances() {
        let mainnet_handle = tokio::spawn(async {
            let sequencer = TestNode::new().await;
            let provider = sequencer.backend().blockchain.provider().clone();
            let url = format!("http://{}", sequencer.rpc_addr());
            let block_number = provider.latest_number().unwrap();

            tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
            
            (provider, url, sequencer, block_number)
        });

        let (provider, url, sequencer, block_number) = mainnet_handle.await.unwrap();
        let block_number = Arc::new(block_number);

        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

        let fork_handle = tokio::spawn({
            let block_number = block_number.clone();
            async move {
                let fork_url = Url::parse(&url).unwrap();
                let fork_block = katana_primitives::block::BlockHashOrNumber::Num(*block_number);
                let fork_config = test_config_forking(fork_url, fork_block);
                let sequencer = TestNode::new_with_config(fork_config).await;
                let provider = sequencer.backend().blockchain.provider().clone();
                (provider, sequencer)
            }
        });

        let (fork_provider, fork_sequencer) = fork_handle.await.unwrap();

        let block_number = provider.latest_number().unwrap();
        println!("Mainnet block number: {:?}", block_number);
        let fork_block_number = fork_provider.latest_number().unwrap();
        println!("Fork block number: {:?}", fork_block_number);

        let state_updates = setup_mainnet_updates_randomized(5);
        //Initialize genesis
        provider.compute_state_root(block_number, &state_updates).unwrap();

        let mut producer = IntervalBlockProducer::new(sequencer.backend().clone(), None);
        let mut fork_producer = IntervalBlockProducer::new(fork_sequencer.backend().clone(), None);

        producer.force_mine();
        fork_producer.force_mine();
        
        let block_number = provider.latest_number().unwrap();
        println!("Mainnet block number after genesis: {:?}", block_number);
        let fork_block_number = fork_provider.latest_number().unwrap();
        println!("Fork block number after genesis: {:?}", fork_block_number);

        let fork_minimal_updates = setup_mainnet_updates_randomized(5);
        let state_root = fork_provider.compute_state_root(block_number, &fork_minimal_updates).unwrap();
        let mainnet_state_root_same_updates = provider.compute_state_root(block_number, &fork_minimal_updates).unwrap();

        producer.force_mine();
        fork_producer.force_mine();

        let block_number = provider.latest_number().unwrap();
        println!("Mainnet block number after first run: {:?}", block_number);
        let fork_block_number = fork_provider.latest_number().unwrap();
        println!("Fork block number after first run: {:?}", fork_block_number);

        println!("Forked state root first run: {:?}", state_root);
        println!("Mainnet state root first run: {:?}", mainnet_state_root_same_updates);
        assert!(state_root == mainnet_state_root_same_updates, "State roots do not match on first run");

        let state_updates = setup_mainnet_updates_randomized(5);
        let fork_state_root = fork_provider.compute_state_root(fork_block_number, &state_updates).unwrap();
        let mainnet_state_root = provider.compute_state_root(block_number, &state_updates).unwrap();

        producer.force_mine();
        fork_producer.force_mine();

        let block_number = provider.latest_number().unwrap();
        println!("Mainnet block number after second run: {:?}", block_number);
        let fork_block_number = fork_provider.latest_number().unwrap();
        println!("Fork block number after second run: {:?}", fork_block_number);

        println!("Forked state root second run: {:?}", fork_state_root);
        println!("Mainnet state root second run: {:?}", mainnet_state_root);
        assert!(fork_state_root == mainnet_state_root, "State roots do not match on second run");

        let block_number = provider.latest_number().unwrap();
        println!("Mainnet block number after third run: {:?}", block_number);
        let fork_block_number = fork_provider.latest_number().unwrap();
        println!("Fork block number after third run: {:?}", fork_block_number);

        let state_updates = setup_mainnet_updates_randomized(5);
        let fork_state_root = fork_provider.compute_state_root(fork_block_number, &state_updates).unwrap();
        let mainnet_state_root = provider.compute_state_root(block_number, &state_updates).unwrap();

        println!("Forked state root third run: {:?}", fork_state_root);
        println!("Mainnet state root third run: {:?}", mainnet_state_root);
        assert!(fork_state_root == mainnet_state_root, "State roots do not match on third run");

        producer.force_mine();
        fork_producer.force_mine();

        let cleanup = tokio::join!(
            tokio::spawn(async move { sequencer.handle().stop().await }),
            tokio::spawn(async move { fork_sequencer.handle().stop().await })
        );
        
        cleanup.0.unwrap().unwrap();
        cleanup.1.unwrap().unwrap();
    }

    fn arb_felt() -> impl Strategy<Value = Felt> {
        any::<[u8; 32]>().prop_map(|bytes| Felt::from_bytes_be(&bytes))
    }
    
    fn arb_class_hash() -> impl Strategy<Value = ClassHash> {
        arb_felt().prop_map(ClassHash::from)
    }
    
    fn arb_contract_address() -> impl Strategy<Value = ContractAddress> {
        arb_felt().prop_map(ContractAddress::from)
    }
    
    fn arb_storage() -> impl Strategy<Value = BTreeMap<Felt, Felt>> {
        proptest::collection::btree_map(arb_felt(), arb_felt(), 0..3)
    }
    
    fn arb_state_updates() -> impl Strategy<Value = StateUpdates> {
        proptest::collection::btree_map(arb_contract_address(), (arb_class_hash(), arb_storage(), arb_felt()), 1..6)
            .prop_flat_map(|contracts| {
                // Rozbij na odpowiednie pola
                let mut deployed_contracts = BTreeMap::new();
                let mut storage_updates = BTreeMap::new();
                let mut nonce_updates = BTreeMap::new();
                let mut declared_classes = BTreeMap::new();
                let mut replaced_classes = BTreeMap::new();
                let mut deprecated_declared_classes = BTreeSet::new();
    
                for (address, (class_hash, storage, nonce)) in &contracts {
                    deployed_contracts.insert(*address, *class_hash);
                    storage_updates.insert(*address, storage.clone());
                    nonce_updates.insert(*address, *nonce);
                    declared_classes.insert(*class_hash, Felt::from(1u8)); // losowa wartość
                    // losowo dodaj replaced_classes i deprecated_declared_classes
                }
    
                Just(StateUpdates {
                    deployed_contracts,
                    storage_updates,
                    nonce_updates,
                    declared_classes,
                    replaced_classes,
                    deprecated_declared_classes,
                    ..Default::default()
                })
            })
    }

    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 10,
            .. ProptestConfig::default()
        })]
        #[test]
        fn prop_state_roots_match_for_mainnet_and_forked(
            num_iters in 1usize..=5,
            state_updates_vec in proptest::collection::vec(arb_state_updates(), 1..=5),
            fork_minimal_updates_vec in proptest::collection::vec(arb_state_updates(), 1..=5)
        ) {
            let rt = tokio::runtime::Runtime::new().unwrap();
            let _ = rt.block_on(async {
                let sequencer = TestNode::new().await;
                let backend = sequencer.backend();
                let starknet_provider = sequencer.starknet_provider();
                let provider = backend.blockchain.provider();

                let url = format!("http://{}", sequencer.rpc_addr());
                let url = Url::parse(&url).unwrap();
                let mut block_number = provider.latest_number().unwrap();

                let db = katana_db::init_ephemeral_db().unwrap();
                let forked_provider = ForkedProvider::new(
                    db.clone(),
                    katana_primitives::block::BlockHashOrNumber::Num(block_number),
                    starknet_provider,
                    url.clone(),
                );

                let mut producer = IntervalBlockProducer::new(backend.clone(), None);

                for i in 0..num_iters {
                    let state_updates = &state_updates_vec[i % state_updates_vec.len()];
                    let fork_minimal_updates = &fork_minimal_updates_vec[i % fork_minimal_updates_vec.len()];

                    provider.compute_state_root(block_number, state_updates).unwrap();

                    let fork_root = forked_provider.compute_state_root(block_number, fork_minimal_updates).unwrap();
                    let mainnet_root = provider.compute_state_root(block_number, fork_minimal_updates).unwrap();

                    prop_assert_eq!(fork_root, mainnet_root, "State roots do not match at iteration {}", i);
                    
                    producer.force_mine();
                    block_number = provider.latest_number().unwrap();
                }
                Ok(())
            });
        }
    }

    #[test]
    fn debug_failing_case() {
        use katana_primitives::{Felt, ContractAddress};
        use katana_primitives::state::StateUpdates;
        use std::collections::{BTreeMap, BTreeSet};
        let mut btree_map = BTreeMap::new();
        btree_map.insert(Felt::ZERO, Felt::ZERO);

        // Skopiowane dane z pliku proptest-regressions/starknet/forking.txt
        let state_updates = StateUpdates {
            nonce_updates: [
                (ContractAddress::from(Felt::from_hex("0x10fb243e82c06238d838f4b3c582b7ad17837ba4117d640ecadc143fd43db5b").unwrap()),
                 Felt::from_hex("0x374c6b4ecc7464bbc2f1b2d1e56cc796d5ad2df9f3736dac32d078dda5e8b49").unwrap())
            ].into_iter().collect(),
            storage_updates: [
                (ContractAddress::from(Felt::from_hex("0x10fb243e82c06238d838f4b3c582b7ad17837ba4117d640ecadc143fd43db5b").unwrap()),
                 btree_map)
            ].into_iter().collect(),
            deployed_contracts: [
                (ContractAddress::from(Felt::from_hex("0x10fb243e82c06238d838f4b3c582b7ad17837ba4117d640ecadc143fd43db5b").unwrap()),
                 ClassHash::from(Felt::from_hex("0x6f6ffb37b185784c4298866f829b0eb736799027bf167b9a7df342520334b4b").unwrap()))
            ].into_iter().collect(),
            declared_classes: [
                (ClassHash::from(Felt::from_hex("0x6f6ffb37b185784c4298866f829b0eb736799027bf167b9a7df342520334b4b").unwrap()),
                 Felt::from_hex("0x1").unwrap())
            ].into_iter().collect(),
            deprecated_declared_classes: BTreeSet::new(),
            replaced_classes: BTreeMap::new(),
            ..Default::default()
        };

        let fork_minimal_updates = StateUpdates {
            nonce_updates: [
                (ContractAddress::from(Felt::from_hex("0x5991364353694863494443e75a19bddf51a6a67df7da4d196f6e3987b6874d").unwrap()),
                 Felt::from_hex("0x686a912006908670917a9695ed0021e5462bb7495aa52fc29ff73f0ee9fe5d9").unwrap()),
                (ContractAddress::from(Felt::from_hex("0x16ea94eafbe872cd98f9f6e070fa083382a5876a18f34dbae1858601757a5f1").unwrap()),
                 Felt::from_hex("0x77d5d0ef0d14fa2d36f7f943b0a81921d799d8c0da7d1c3ac09d9c9b48c5379").unwrap()),
                (ContractAddress::from(Felt::from_hex("0x40807a654b32e4aa778a9aa76702aaa25935fce2d79a4a3c9af568eb0839874").unwrap()),
                 Felt::from_hex("0x27926baafc1aae314b0366ddd961c0d0455e4b69939bee4b43ec41482db4c16").unwrap()),
                (ContractAddress::from(Felt::from_hex("0x5c6fe4e6bf95e03d9478e3fe3e2d6eab767a72ff868caa1a8a964491373ba7d").unwrap()),
                 Felt::from_hex("0x3c294d24e1f8f499865adebdd230647a088a86373230177cb48e397bc38788c").unwrap()),
                (ContractAddress::from(Felt::from_hex("0x77d97a9faa26dce726faf8a30f308abd81fd375b1b55f7b6c8aae1dec3503c4").unwrap()),
                 Felt::from_hex("0x69727236c816d0e41aa76f71656d2c7c1b63ecf03a6e16fe1b928a2dfaf7463").unwrap()),
            ].into_iter().collect(),
            storage_updates: [
                (ContractAddress::from(Felt::from_hex("0x5991364353694863494443e75a19bddf51a6a67df7da4d196f6e3987b6874d").unwrap()),
                 [
                    (Felt::from_hex("0xc8b4591e6ac3f1e0f3e0d1e7dcbb0624656a2e559add1b86086774892b8fd4").unwrap(), Felt::from_hex("0x68edc8122ff1009d0e56463bf6b26a5ae6a91ca617247db773512cc01612c44").unwrap()),
                    (Felt::from_hex("0x69f5683aae47a69bfe12823bf555bd25bbfe8afc2bdbf21d8cea06d4d0657c6").unwrap(), Felt::from_hex("0x5ad43ca135b55ef291824b48bab917e3070d96437f17b1950d00c196eeda3e0").unwrap()),
                 ].into_iter().collect()),
                (ContractAddress::from(Felt::from_hex("0x16ea94eafbe872cd98f9f6e070fa083382a5876a18f34dbae1858601757a5f1").unwrap()),
                 [
                    (Felt::from_hex("0xd528c0c0a99dc646a419bd72211be0dbce7862973e77cc9a8c68f2af4bc59f").unwrap(), Felt::from_hex("0x58905833ca7d08e659c9cbed4c38389edd272f0d97ab49a5625f59e6db76a25").unwrap()),
                 ].into_iter().collect()),
                (ContractAddress::from(Felt::from_hex("0x40807a654b32e4aa778a9aa76702aaa25935fce2d79a4a3c9af568eb0839874").unwrap()),
                 [
                    (Felt::from_hex("0x1866df9ae650d18ebaa717b55d550f2f69ed8a853bd7db4cdd4b3b41d51cc4f").unwrap(), Felt::from_hex("0x3cec9df661101c30fc01333dc5d801e8fc04b071f63caa0f34a5ab97fa943a1").unwrap()),
                    (Felt::from_hex("0x321cb66a9256c0c081ca52dbaa3f577b5dda466cf6246cf5e5e390f092fc5c3").unwrap(), Felt::from_hex("0x3da951024bfcdce26b7fe125a1cfebcd5b3773e444a39e3e6d454a45c3fb843").unwrap()),
                 ].into_iter().collect()),
                (ContractAddress::from(Felt::from_hex("0x5c6fe4e6bf95e03d9478e3fe3e2d6eab767a72ff868caa1a8a964491373ba7d").unwrap()),
                 [
                    (Felt::from_hex("0x44ca6e0abdd6baf26e7092e757e6362f78fc763b9dbe015264826edc40fc682").unwrap(), Felt::from_hex("0x51d40d70918a1f84c2347aa6ab476ed7d6932e32e5d608911596f90aea8af46").unwrap()),
                 ].into_iter().collect()),
                (ContractAddress::from(Felt::from_hex("0x77d97a9faa26dce726faf8a30f308abd81fd375b1b55f7b6c8aae1dec3503c4").unwrap()),
                 BTreeMap::new()),
            ].into_iter().collect(),
            deployed_contracts: [
                (ContractAddress::from(Felt::from_hex("0x5991364353694863494443e75a19bddf51a6a67df7da4d196f6e3987b6874d").unwrap()), ClassHash::from(Felt::from_hex("0x43157a62d2e288a46a0ff0e106e5d871d676b658648d79b6913f880343cb820").unwrap())),
                (ContractAddress::from(Felt::from_hex("0x16ea94eafbe872cd98f9f6e070fa083382a5876a18f34dbae1858601757a5f1").unwrap()), ClassHash::from(Felt::from_hex("0x1378b9ebe07cab5d9f68eff2f5d6e404256a6ac07b48394eedf49448b54929").unwrap())),
                (ContractAddress::from(Felt::from_hex("0x40807a654b32e4aa778a9aa76702aaa25935fce2d79a4a3c9af568eb0839874").unwrap()), ClassHash::from(Felt::from_hex("0x451a4390ad61731406919a96d605bef7fe27d428cdcd36c10f9fc42f1de50de").unwrap())),
                (ContractAddress::from(Felt::from_hex("0x5c6fe4e6bf95e03d9478e3fe3e2d6eab767a72ff868caa1a8a964491373ba7d").unwrap()), ClassHash::from(Felt::from_hex("0xa6b89431e8bd401a9e723f74b91fd48ff87d29fbf1f81fddf752aeca40c3df").unwrap())),
                (ContractAddress::from(Felt::from_hex("0x77d97a9faa26dce726faf8a30f308abd81fd375b1b55f7b6c8aae1dec3503c4").unwrap()), ClassHash::from(Felt::from_hex("0xd7299f46238eeb37ceb34f3552d7f618e86a8a889b4b3bb8eda330ce196ecd").unwrap())),
            ].into_iter().collect(),
            declared_classes: [
                (ClassHash::from(Felt::from_hex("0x1378b9ebe07cab5d9f68eff2f5d6e404256a6ac07b48394eedf49448b54929").unwrap()), Felt::from_hex("0x1").unwrap()),
                (ClassHash::from(Felt::from_hex("0xa6b89431e8bd401a9e723f74b91fd48ff87d29fbf1f81fddf752aeca40c3df").unwrap()), Felt::from_hex("0x1").unwrap()),
                (ClassHash::from(Felt::from_hex("0xd7299f46238eeb37ceb34f3552d7f618e86a8a889b4b3bb8eda330ce196ecd").unwrap()), Felt::from_hex("0x1").unwrap()),
                (ClassHash::from(Felt::from_hex("0x43157a62d2e288a46a0ff0e106e5d871d676b658648d79b6913f880343cb820").unwrap()), Felt::from_hex("0x1").unwrap()),
                (ClassHash::from(Felt::from_hex("0x451a4390ad61731406919a96d605bef7fe27d428cdcd36c10f9fc42f1de50de").unwrap()), Felt::from_hex("0x1").unwrap()),
            ].into_iter().collect(),
            deprecated_declared_classes: BTreeSet::new(),
            replaced_classes: BTreeMap::new(),
            ..Default::default()
        };

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let sequencer = TestNode::new().await;
            let backend = sequencer.backend();
            let starknet_provider = sequencer.starknet_provider();
            let provider = backend.blockchain.provider();

            let url = format!("http://{}", sequencer.rpc_addr());
            let url = Url::parse(&url).unwrap();
            let block_number = provider.latest_number().unwrap();

            provider.compute_state_root(block_number, &state_updates).unwrap();

            let db = katana_db::init_ephemeral_db().unwrap();
            let forked_provider = ForkedProvider::new(
                db.clone(),
                katana_primitives::block::BlockHashOrNumber::Num(block_number),
                starknet_provider,
                url.clone(),
            );

            let fork_root = forked_provider.compute_state_root(block_number, &fork_minimal_updates).unwrap();
            let mainnet_root = provider.compute_state_root(block_number, &fork_minimal_updates).unwrap();

            assert_eq!(fork_root, mainnet_root);
        });
    }
}
