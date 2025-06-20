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
        // #![proptest_config(ProptestConfig {
        //     cases: 5,
        //     .. ProptestConfig::default()
        // })]
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

                let mut producer = IntervalBlockProducer::new(backend.clone(), None);

                let initial_state = &state_updates_vec[0];
                provider.compute_state_root(block_number, initial_state).unwrap();
                producer.force_mine();
                block_number = provider.latest_number().unwrap();

                let db = katana_db::init_ephemeral_db().unwrap();
                let forked_provider = ForkedProvider::new(
                    db.clone(),
                    katana_primitives::block::BlockHashOrNumber::Num(block_number),
                    starknet_provider,
                    url.clone(),
                );

                for i in 0..num_iters {
                    let fork_minimal_updates = &fork_minimal_updates_vec[i % fork_minimal_updates_vec.len()];

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
        
        // First state update from state_updates_vec[0]
        // let state_updates = StateUpdates {
        //     nonce_updates: [].into_iter().collect(),
        //     storage_updates: [(
        //         ContractAddress::from(Felt::from_hex("0x4c3417b29b568b0ef3f6c1e4ab6aa844a26f7b6539f3853cae3c486e55f4774").unwrap()),
        //         BTreeMap::new(),
        //     )].into_iter().collect(),
        //     deployed_contracts: [].into_iter().collect(),
        //     declared_classes: [].into_iter().collect(),
        //     deprecated_declared_classes: BTreeSet::new(),
        //     replaced_classes: BTreeMap::new(),
        // };

        // First update from fork_minimal_updates_vec[0]
        let fork_minimal_updates = StateUpdates {
            nonce_updates: [].into_iter().collect(),
            storage_updates: [
                (ContractAddress::from(Felt::from_hex("0x246258999ea81791cf6e6873e9cffb15c27c4e96b99558d78ff7e3c177d73c8").unwrap()),
                 BTreeMap::new()),
                (ContractAddress::from(Felt::from_hex("0x140d99b5f8493f04b1f1eb09734048e2860352cc76cd57f9b2e2a4deafbc9c0").unwrap()),
                 BTreeMap::new(),
                ),
                (ContractAddress::from(Felt::from_hex("0x3d2d7cf3e9a59d09ed30e4812ab0d0cbd8cda5bdaa14a1bf5abe3ce6536ea7c").unwrap()),
                 BTreeMap::new()),
            ].into_iter().collect(),
            deployed_contracts: [].into_iter().collect(),
            declared_classes: [].into_iter().collect(),
            deprecated_declared_classes: BTreeSet::new(),
            replaced_classes: BTreeMap::new(),
        };

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let sequencer = TestNode::new().await;
            let backend = sequencer.backend();
            let starknet_provider = sequencer.starknet_provider();
            let provider = backend.blockchain.provider();

            let url = format!("http://{}", sequencer.rpc_addr());
            let url = Url::parse(&url).unwrap();
            let mut block_number = provider.latest_number().unwrap();

            let mut producer = IntervalBlockProducer::new(backend.clone(), None);
            // producer.force_mine();
            // provider.compute_state_root(block_number, &state_updates).unwrap();
            // producer.force_mine();
            // block_number = provider.latest_number().unwrap();

            let db = katana_db::init_ephemeral_db().unwrap();
            let forked_provider = ForkedProvider::new(
                db.clone(),
                katana_primitives::block::BlockHashOrNumber::Num(block_number),
                starknet_provider,
                url.clone(),
            );

            let fork_root = forked_provider.compute_state_root(block_number, &fork_minimal_updates).unwrap();
            let mainnet_root = provider.compute_state_root(block_number, &fork_minimal_updates).unwrap();

            assert_eq!(fork_root, mainnet_root, "State roots do not match");
        });
    }

    #[test]
    fn debug_passing_case() {
        use katana_primitives::{Felt, ContractAddress};
        use katana_primitives::state::StateUpdates;
        use std::collections::{BTreeMap, BTreeSet};
        
        // First state update from state_updates_vec[0]
        let state_updates = StateUpdates {
            nonce_updates: [].into_iter().collect(),
            storage_updates: [(
                ContractAddress::from(Felt::from_hex("0x4c3417b29b568b0ef3f6c1e4ab6aa844a26f7b6539f3853cae3c486e55f4774").unwrap()),
                [].into_iter().collect()
            )].into_iter().collect(),
            deployed_contracts: [].into_iter().collect(),
            declared_classes: [].into_iter().collect(),
            deprecated_declared_classes: BTreeSet::new(),
            replaced_classes: BTreeMap::new(),
        };

        // First update from fork_minimal_updates_vec[0]
        let fork_minimal_updates = StateUpdates {
            nonce_updates: [].into_iter().collect(),
            storage_updates: [
                (ContractAddress::from(Felt::from_hex("0x140d99b5f8493f04b1f1eb09734048e2860352cc76cd57f9b2e2a4deafbc9c0").unwrap()),
                 [].into_iter().collect()
                ),
                (ContractAddress::from(Felt::from_hex("0x246258999ea81791cf6e6873e9cffb15c27c4e96b99558d78ff7e3c177d73c8").unwrap()),
                 BTreeMap::new()),
                // (ContractAddress::from(Felt::from_hex("0x3d2d7cf3e9a59d09ed30e4812ab0d0cbd8cda5bdaa14a1bf5abe3ce6536ea7c").unwrap()),
                //  BTreeMap::new()),
            ].into_iter().collect(),
            deployed_contracts: [].into_iter().collect(),
            declared_classes: [].into_iter().collect(),
            deprecated_declared_classes: BTreeSet::new(),
            replaced_classes: BTreeMap::new(),
        };

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let sequencer = TestNode::new().await;
            let backend = sequencer.backend();
            let starknet_provider = sequencer.starknet_provider();
            let provider = backend.blockchain.provider();

            let url = format!("http://{}", sequencer.rpc_addr());
            let url = Url::parse(&url).unwrap();
            let mut block_number = provider.latest_number().unwrap();

            let mut producer = IntervalBlockProducer::new(backend.clone(), None);
            // producer.force_mine();
            provider.compute_state_root(block_number, &state_updates).unwrap();
            // producer.force_mine();
            // block_number = provider.latest_number().unwrap();

            let db = katana_db::init_ephemeral_db().unwrap();
            let forked_provider = ForkedProvider::new(
                db.clone(),
                katana_primitives::block::BlockHashOrNumber::Num(block_number),
                starknet_provider,
                url.clone(),
            );

            let fork_root = forked_provider.compute_state_root(block_number, &fork_minimal_updates).unwrap();
            let mainnet_root = provider.compute_state_root(block_number, &fork_minimal_updates).unwrap();

            assert_eq!(fork_root, mainnet_root, "State roots do not match");
        });
    }
}
