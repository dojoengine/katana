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

    #[tokio::test]
    async fn test_commit_new_state_root() {
        use katana_primitives::state::StateUpdates;
        use katana_provider::providers::fork::ForkedProvider;
        use katana_provider::traits::trie::TrieWriter;
        use starknet::providers::jsonrpc::HttpTransport;
        use starknet::providers::JsonRpcClient;
        use std::collections::BTreeMap;
        use katana_chain_spec::ChainSpec;
        use katana_executor::implementation::noop::NoopExecutorFactory;
        use katana_core::backend::storage::Blockchain;
        use katana_core::backend::gas_oracle::GasOracle;
        use katana_core::backend::Backend;
        use katana_runner::KatanaRunner;
        use katana_runner::KatanaRunnerConfig;
        
        let runner = KatanaRunner::new().expect("Failed to start local Katana");
        let provider = runner.starknet_provider();
        let rpc_url = runner.instance.rpc_addr();
        let url = Url::parse(&format!("http://{}", rpc_url)).expect("invalid url");

        let url = Url::parse("http://localhost:5051").unwrap();

        let external_client =
            JsonRpcClient::new(HttpTransport::new(url.clone()));

        let block_number = external_client.block_number().await.unwrap();
        println!("Block number: {:?}", block_number);

        // let url = Url::parse(SEPOLIA_URL).unwrap();
        
        // let client = ForkedClient::new_http(url.clone(), block_number);

        // let external_state_update =
        //     external_client.get_state_update(BlockId::Tag(BlockTag::Latest)).await.unwrap();

        // let (external_state_root, external_state_update) = match external_state_update {
        //     StarknetRsMaybePendingStateUpdate::Update(state_update) => {
        //         println!("External new_root: {:#x}", state_update.new_root);
        //         println!("External old_root: {:#x}", state_update.old_root);
        //         println!("External block_hash: {:#x}", state_update.block_hash);
        //         (state_update.new_root, state_update)
        //     }
        //     StarknetRsMaybePendingStateUpdate::PendingUpdate(pending) => {
        //         println!("Pending state update - no state root available");
        //         return;
        //     }
        // };

        let mut state_updates = StateUpdates::default();

        // Contract 1: Original contract with multiple storage updates
        let contract_address_1 =
            felt!("0x06a4d4e8c1cc9785e125195a2f8bd4e5b0c7510b19f3e2dd63533524f5687e41").into();
        let class_hash_1 =
            felt!("0x03d5de568b28042464214dfbe2ea0d7e22d162986bcdb9f56d691d22955a4c23");
        
        // Multiple storage entries for contract 1
        let mut contract_1_storage = BTreeMap::new();
        contract_1_storage.insert(felt!("0x1").into(), felt!("0x123456").into());
        contract_1_storage.insert(felt!("0x2").into(), felt!("0x789abc").into());
        contract_1_storage.insert(felt!("0x5f2e74e1dce39a8d0d7e6b8f9c3a7e4d1c0b2a98").into(), felt!("0xdeadbeef").into());
        
        state_updates.deployed_contracts.insert(contract_address_1, class_hash_1);
        state_updates.storage_updates.insert(contract_address_1, contract_1_storage);
        state_updates.declared_classes.insert(class_hash_1, felt!("0x123").into());
        state_updates.nonce_updates.insert(contract_address_1, felt!("0x5").into());

        // Contract 2: ERC20-like contract
        let contract_address_2 =
            felt!("0x049d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7").into();
        let class_hash_2 =
            felt!("0x02760f25d5a4fb2bdde5f561fd0b44a3dee78c28903577d37d669939d97036a0");
            
        let mut contract_2_storage = BTreeMap::new();
        // Balance mapping entries
        contract_2_storage.insert(felt!("0x916907772491729262376534102982").into(), felt!("0x1000000000000000000").into()); // 1 ETH
        contract_2_storage.insert(felt!("0x916907772491729262376534102983").into(), felt!("0x2000000000000000000").into()); // 2 ETH
        contract_2_storage.insert(felt!("0x5").into(), felt!("0x1bc16d674ec80000").into()); // Total supply
        
        state_updates.deployed_contracts.insert(contract_address_2, class_hash_2);
        state_updates.storage_updates.insert(contract_address_2, contract_2_storage);
        state_updates.declared_classes.insert(class_hash_2, felt!("0x456").into());
        state_updates.nonce_updates.insert(contract_address_2, felt!("0x12").into());

        // Contract 3: Account contract
        let contract_address_3 =
            felt!("0x0127fd5f1fe78a71f8bcd1fec63e3fe2f0486b6ecd5c86a0466c3a21fa5cfcec").into();
        let class_hash_3 =
            felt!("0x033434ad846cdd5f23eb73ff09fe6fddd568284a0fb7d1be20ee482f044dabe2");
            
        let mut contract_3_storage = BTreeMap::new();
        contract_3_storage.insert(felt!("0x0").into(), felt!("0x2dd76e7ad84dbed81c314ffe5e7a7cacfb8f4836f01af4e913f275f89a3de1a").into()); // Public key
        contract_3_storage.insert(felt!("0x1").into(), felt!("0x1").into()); // Some flag
        
        state_updates.deployed_contracts.insert(contract_address_3, class_hash_3);
        state_updates.storage_updates.insert(contract_address_3, contract_3_storage);
        state_updates.declared_classes.insert(class_hash_3, felt!("0x789").into());
        state_updates.nonce_updates.insert(contract_address_3, felt!("0x7").into());

        // Contract 4: Upgraded contract (replaced class)
        let contract_address_4 =
            felt!("0x04194c376fcddd757b190476f840f2d211d44c68ba79a6b627fa47e157cd4f97").into();
        let old_class_hash_4 =
            felt!("0x025ec026985a3bf9d0cc1fe17326b245dfdc3ff89b8fde106542a3ea56c5a918");
        let new_class_hash_4 =
            felt!("0x033434ad846cdd5f23eb73ff09fe6fddd568284a0fb7d1be20ee482f044dabe2");
            
        let mut contract_4_storage = BTreeMap::new();
        contract_4_storage.insert(felt!("0xa").into(), felt!("0xcafebabe").into());
        contract_4_storage.insert(felt!("0xb").into(), felt!("0x42424242").into());
        
        state_updates.storage_updates.insert(contract_address_4, contract_4_storage);
        state_updates.replaced_classes.insert(contract_address_4, new_class_hash_4);
        state_updates.declared_classes.insert(new_class_hash_4, felt!("0xabc").into());
        state_updates.nonce_updates.insert(contract_address_4, felt!("0x3").into());

        // Contract 5: Simple storage contract
        let contract_address_5 =
            felt!("0x01234567890abcdef1234567890abcdef1234567890abcdef1234567890abcde").into();
        let class_hash_5 =
            felt!("0x0543211234567890abcdef0123456789abcdef0123456789abcdef0123456789");
            
        let mut contract_5_storage = BTreeMap::new();
        contract_5_storage.insert(felt!("0x100").into(), felt!("0x1111111111111111").into());
        contract_5_storage.insert(felt!("0x200").into(), felt!("0x2222222222222222").into());
        contract_5_storage.insert(felt!("0x300").into(), felt!("0x3333333333333333").into());
        
        state_updates.deployed_contracts.insert(contract_address_5, class_hash_5);
        state_updates.storage_updates.insert(contract_address_5, contract_5_storage);
        state_updates.declared_classes.insert(class_hash_5, felt!("0xdef").into());
        state_updates.nonce_updates.insert(contract_address_5, felt!("0x1").into());

        // Additional declared classes (not deployed yet)
        let additional_class_hash_1 = felt!("0x0987654321098765432109876543210987654321098765432109876543210987");
        let additional_class_hash_2 = felt!("0x0111222333444555666777888999aaabbbcccdddeeefffaaabbbcccdddeee");
        let additional_class_hash_3 = felt!("0x0fedcba9876543210fedcba9876543210fedcba9876543210fedcba987654321");
        
        state_updates.declared_classes.insert(additional_class_hash_1, felt!("0x111").into());
        state_updates.declared_classes.insert(additional_class_hash_2, felt!("0x222").into());
        state_updates.declared_classes.insert(additional_class_hash_3, felt!("0x333").into());

        // Deprecated declared classes
        let deprecated_class_hash_1 = felt!("0x0abcdef123456789abcdef123456789abcdef123456789abcdef123456789abc");
        let deprecated_class_hash_2 = felt!("0x0555666777888999aaabbbcccdddeeefffaaabbbcccdddeeefffaaabbbcccddd");
        
        state_updates.deprecated_declared_classes.insert(deprecated_class_hash_1);
        state_updates.deprecated_declared_classes.insert(deprecated_class_hash_2);

        println!("📊 Enhanced state updates with {} contracts, {} storage entries, {} nonces, {} declared classes, {} deprecated classes, {} replaced classes", 
            state_updates.deployed_contracts.len(),
            state_updates.storage_updates.values().map(|s| s.len()).sum::<usize>(),
            state_updates.nonce_updates.len(),
            state_updates.declared_classes.len(),
            state_updates.deprecated_declared_classes.len(),
            state_updates.replaced_classes.len()
        );

        let db = katana_db::init_ephemeral_db().unwrap();
        // Create the provider
        let rpc_provider = JsonRpcClient::new(HttpTransport::new(url.clone()));
        let forked_provider = ForkedProvider::new(
            db.clone(),
            katana_primitives::block::BlockHashOrNumber::Num(block_number),
            rpc_provider,
            url.clone(),
        );
        let state_root = forked_provider.compute_state_root(block_number, &state_updates).unwrap();

        let chain_spec = Arc::new(ChainSpec::dev());
        let executor_factory = NoopExecutorFactory::new();
        let blockchain = Blockchain::new(DbProvider::new_ephemeral());
        let gas_oracle = GasOracle::fixed(Default::default(), Default::default());
        let backend = Arc::new(Backend::new(chain_spec, blockchain.clone(), gas_oracle, executor_factory));
        // backend.init_genesis(false).expect("failed to initialize genesis");

        let mainnet_provider = blockchain.provider();
        let mainnet_state_root_after_insertion = mainnet_provider.compute_state_root(block_number, &state_updates).unwrap();
        // println!("State root: {:?}", state_root);
        // println!("External state root: {:?}", external_state_root);
        println!("Mainnet state root after insertion: {:?}", mainnet_state_root_after_insertion);

    }

    #[tokio::test]
    async fn compare_storage_roots() {
        let katana =
            JsonRpcClient::new(HttpTransport::new(Url::parse("http://localhost:5050").unwrap()));
        let forked_katana =
            JsonRpcClient::new(HttpTransport::new(Url::parse("http://localhost:5051").unwrap()));

        let block_number = katana.block_number().await.unwrap();
        println!("Block number: {:?}", block_number);

        // Compare state roots for blocks 0-4
        for block_num in 0..=8 {
            println!("\n==================================================");
            println!("Comparing block {}", block_num);
            println!("==================================================");

            let katana_state_update = match katana.get_state_update(BlockId::Number(block_num)).await {
                Ok(update) => update,
                Err(e) => {
                    println!("❌ Failed to get katana state update for block {}: {:?}", block_num, e);
                    continue;
                }
            };

            let forked_katana_state_update = match forked_katana.get_state_update(BlockId::Number(block_num)).await {
                Ok(update) => update,
                Err(e) => {
                    println!("❌ Failed to get forked katana state update for block {}: {:?}", block_num, e);
                    continue;
                }
            };

            let katana_state_root = match katana_state_update {
                StarknetRsMaybePendingStateUpdate::Update(state_update) => {
                    println!("Katana block {} new_root: {:#x}", block_num, state_update.new_root);
                    println!("Katana block {} old_root: {:#x}", block_num, state_update.old_root);
                    println!("Katana block {} block_hash: {:#x}", block_num, state_update.block_hash);
                    println!("Katana block {} state_diff: {:?}", block_num, state_update.state_diff);
                    state_update.new_root
                }
                StarknetRsMaybePendingStateUpdate::PendingUpdate(pending) => {
                    println!("Pending state update for block {} - no state root available", block_num);
                    continue;
                }
            };

            let forked_katana_state_root = match forked_katana_state_update {
                StarknetRsMaybePendingStateUpdate::Update(state_update) => {
                    println!("Forked katana block {} new_root: {:#x}", block_num, state_update.new_root);
                    println!("Forked katana block {} old_root: {:#x}", block_num, state_update.old_root);
                    println!("Forked katana block {} block_hash: {:#x}", block_num, state_update.block_hash);
                    println!("Forked katana block {} state_diff: {:?}", block_num, state_update.state_diff);
                    state_update.new_root
                }
                StarknetRsMaybePendingStateUpdate::PendingUpdate(pending) => {
                    println!("Pending state update for forked block {} - no state root available", block_num);
                    continue;
                }
            };

            println!("\nComparing block {} state roots:", block_num);
            println!("Katana block {} state_root: {:#x}\n", block_num, katana_state_root);
            println!("Forked katana block {} state_root: {:#x}", block_num, forked_katana_state_root);
            
            if katana_state_root == forked_katana_state_root {
                println!("✅ Block {} state roots match!", block_num);
            } else {
                println!("❌ Block {} state roots do NOT match!", block_num);
            }
        }

        // Check account nonce as additional verification
        let account_address: ContractAddress =
            felt!("0x127fd5f1fe78a71f8bcd1fec63e3fe2f0486b6ecd5c86a0466c3a21fa5cfcec").into();
        let account_nonce =
            katana.get_nonce(BlockId::Tag(BlockTag::Latest), account_address).await.unwrap();
        println!("\nAccount nonce (latest): {:?}", account_nonce);

        let forked_account_nonce =
            forked_katana.get_nonce(BlockId::Tag(BlockTag::Latest), account_address).await.unwrap();
        println!("Forked account nonce (latest): {:?}", forked_account_nonce);

        // let contract_address: ContractAddress =
        //     felt!("0x4194c376fcddd757b190476f840f2d211d44c68ba79a6b627fa47e157cd4f97").into();
        // let contract_nonce =
        //     katana.get_nonce(BlockId::Tag(BlockTag::Latest), contract_address).await.unwrap();
        // println!("Contract nonce: {:?}", contract_nonce);

        // let forked_contract_nonce = forked_katana
        //     .get_nonce(BlockId::Tag(BlockTag::Latest), contract_address)
        //     .await
        //     .unwrap();
        // println!("Forked contract nonce: {:?}", forked_contract_nonce);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_commit_new_state_root_two_katana_instances() {
        use katana_primitives::state::StateUpdates;
        use katana_provider::providers::fork::ForkedProvider;
        use katana_provider::traits::trie::TrieWriter;
        use starknet::providers::jsonrpc::HttpTransport;
        use starknet::providers::JsonRpcClient;
        use std::collections::BTreeMap;
        use katana_chain_spec::dev::ChainSpec;
        use katana_executor::implementation::noop::NoopExecutorFactory;
        use katana_core::backend::storage::Blockchain;
        use katana_core::backend::gas_oracle::GasOracle;
        use katana_core::backend::Backend;
        use jsonrpsee::http_client::HttpClientBuilder;
        use katana_provider::traits::block::{BlockNumberProvider, BlockProvider};
        use katana_provider::traits::env::BlockEnvProvider;
        use katana_rpc_api::dev::DevApiClient;
        use katana_utils::TestNode;
        use katana_utils::node::test_config_forking;

        let sequencer = TestNode::new().await;
        let backend = sequencer.backend();
        let blockchain = sequencer.blockchain();
        let starknet_provider = sequencer.starknet_provider();
        let provider = backend.blockchain.provider();
    
        // Create a jsonrpsee client for the DevApi
        let url = format!("http://{}", sequencer.rpc_addr());
        println!("URL: {:?}", url);
        
        let url = Url::parse(&url).unwrap();
        println!("parsed URL: {:?}", url);
        

        let block_number = provider.latest_number().unwrap();
        println!("Block number from provider: {:?}", block_number);

        let mut state_updates = StateUpdates::default();

        // Contract 1: Original contract with multiple storage updates
        let contract_address_1 =
            felt!("0x06a4d4e8c1cc9785e125195a2f8bd4e5b0c7510b19f3e2dd63533524f5687e41").into();
        let class_hash_1 =
            felt!("0x03d5de568b28042464214dfbe2ea0d7e22d162986bcdb9f56d691d22955a4c23");
        
        // Multiple storage entries for contract 1
        let mut contract_1_storage = BTreeMap::new();
        contract_1_storage.insert(felt!("0x1").into(), felt!("0x123456").into());
        contract_1_storage.insert(felt!("0x2").into(), felt!("0x789abc").into());
        contract_1_storage.insert(felt!("0x5f2e74e1dce39a8d0d7e6b8f9c3a7e4d1c0b2a98").into(), felt!("0xdeadbeef").into());
        
        state_updates.deployed_contracts.insert(contract_address_1, class_hash_1);
        state_updates.storage_updates.insert(contract_address_1, contract_1_storage);
        state_updates.declared_classes.insert(class_hash_1, felt!("0x123").into());
        state_updates.nonce_updates.insert(contract_address_1, felt!("0x5").into());

        // Contract 2: ERC20-like contract
        let contract_address_2 =
            felt!("0x049d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7").into();
        let class_hash_2 =
            felt!("0x02760f25d5a4fb2bdde5f561fd0b44a3dee78c28903577d37d669939d97036a0");
            
        let mut contract_2_storage = BTreeMap::new();
        // Balance mapping entries
        contract_2_storage.insert(felt!("0x916907772491729262376534102982").into(), felt!("0x1000000000000000000").into()); // 1 ETH
        contract_2_storage.insert(felt!("0x916907772491729262376534102983").into(), felt!("0x2000000000000000000").into()); // 2 ETH
        contract_2_storage.insert(felt!("0x5").into(), felt!("0x1bc16d674ec80000").into()); // Total supply
        
        state_updates.deployed_contracts.insert(contract_address_2, class_hash_2);
        state_updates.storage_updates.insert(contract_address_2, contract_2_storage);
        state_updates.declared_classes.insert(class_hash_2, felt!("0x456").into());
        state_updates.nonce_updates.insert(contract_address_2, felt!("0x12").into());

        // Contract 3: Account contract
        let contract_address_3 =
            felt!("0x0127fd5f1fe78a71f8bcd1fec63e3fe2f0486b6ecd5c86a0466c3a21fa5cfcec").into();
        let class_hash_3 =
            felt!("0x033434ad846cdd5f23eb73ff09fe6fddd568284a0fb7d1be20ee482f044dabe2");
            
        let mut contract_3_storage = BTreeMap::new();
        contract_3_storage.insert(felt!("0x0").into(), felt!("0x2dd76e7ad84dbed81c314ffe5e7a7cacfb8f4836f01af4e913f275f89a3de1a").into()); // Public key
        contract_3_storage.insert(felt!("0x1").into(), felt!("0x1").into()); // Some flag
        
        state_updates.deployed_contracts.insert(contract_address_3, class_hash_3);
        state_updates.storage_updates.insert(contract_address_3, contract_3_storage);
        state_updates.declared_classes.insert(class_hash_3, felt!("0x789").into());
        state_updates.nonce_updates.insert(contract_address_3, felt!("0x7").into());

        // Contract 4: Upgraded contract (replaced class)
        let contract_address_4 =
            felt!("0x04194c376fcddd757b190476f840f2d211d44c68ba79a6b627fa47e157cd4f97").into();
        let old_class_hash_4 =
            felt!("0x025ec026985a3bf9d0cc1fe17326b245dfdc3ff89b8fde106542a3ea56c5a918");
        let new_class_hash_4 =
            felt!("0x033434ad846cdd5f23eb73ff09fe6fddd568284a0fb7d1be20ee482f044dabe2");
            
        let mut contract_4_storage = BTreeMap::new();
        contract_4_storage.insert(felt!("0xa").into(), felt!("0xcafebabe").into());
        contract_4_storage.insert(felt!("0xb").into(), felt!("0x42424242").into());
        
        state_updates.storage_updates.insert(contract_address_4, contract_4_storage);
        state_updates.replaced_classes.insert(contract_address_4, new_class_hash_4);
        state_updates.declared_classes.insert(new_class_hash_4, felt!("0xabc").into());
        state_updates.nonce_updates.insert(contract_address_4, felt!("0x3").into());

        // Contract 5: Simple storage contract
        let contract_address_5 =
            felt!("0x01234567890abcdef1234567890abcdef1234567890abcdef1234567890abcde").into();
        let class_hash_5 =
            felt!("0x0543211234567890abcdef0123456789abcdef0123456789abcdef0123456789");
            
        let mut contract_5_storage = BTreeMap::new();
        contract_5_storage.insert(felt!("0x100").into(), felt!("0x1111111111111111").into());
        contract_5_storage.insert(felt!("0x200").into(), felt!("0x2222222222222222").into());
        contract_5_storage.insert(felt!("0x300").into(), felt!("0x3333333333333333").into());
        
        state_updates.deployed_contracts.insert(contract_address_5, class_hash_5);
        state_updates.storage_updates.insert(contract_address_5, contract_5_storage);
        state_updates.declared_classes.insert(class_hash_5, felt!("0xdef").into());
        state_updates.nonce_updates.insert(contract_address_5, felt!("0x1").into());

        // Additional declared classes (not deployed yet)
        let additional_class_hash_1 = felt!("0x0987654321098765432109876543210987654321098765432109876543210987");
        let additional_class_hash_2 = felt!("0x0111222333444555666777888999aaabbbcccdddeeefffaaabbbcccdddeee");
        let additional_class_hash_3 = felt!("0x0fedcba9876543210fedcba9876543210fedcba9876543210fedcba987654321");
        
        state_updates.declared_classes.insert(additional_class_hash_1, felt!("0x111").into());
        state_updates.declared_classes.insert(additional_class_hash_2, felt!("0x222").into());
        state_updates.declared_classes.insert(additional_class_hash_3, felt!("0x333").into());

        // Deprecated declared classes
        let deprecated_class_hash_1 = felt!("0x0abcdef123456789abcdef123456789abcdef123456789abcdef123456789abc");
        let deprecated_class_hash_2 = felt!("0x0555666777888999aaabbbcccdddeeefffaaabbbcccdddeeefffaaabbbcccddd");
        
        state_updates.deprecated_declared_classes.insert(deprecated_class_hash_1);
        state_updates.deprecated_declared_classes.insert(deprecated_class_hash_2);

        println!("📊 Enhanced state updates with {} contracts, {} storage entries, {} nonces, {} declared classes, {} deprecated classes, {} replaced classes", 
            state_updates.deployed_contracts.len(),
            state_updates.storage_updates.values().map(|s| s.len()).sum::<usize>(),
            state_updates.nonce_updates.len(),
            state_updates.declared_classes.len(),
            state_updates.deprecated_declared_classes.len(),
            state_updates.replaced_classes.len()
        );
        let mainnet_provider = provider;
        let mainnet_state_root = mainnet_provider.compute_state_root(block_number, &state_updates).unwrap();
        
        println!("Mainnet state root in genesis: {:?}", mainnet_state_root);
        
        // Create minimal fork updates with one example from each category
        let mut fork_minimal_updates = StateUpdates::default();
        
        // // 1. Deployed contract
        let minimal_contract_address = felt!("0x06a4d4e8c1cc9785e125195a2f8bd4e5b0c7510b19f3e2dd63533524f5687e41").into();
        let minimal_class_hash = felt!("0x03d5de568b28042464214dfbe2ea0d7e22d162986bcdb9f56d691d22955a4c23");
        fork_minimal_updates.deployed_contracts.insert(minimal_contract_address, minimal_class_hash);
        
        // 2. Storage update
        let mut minimal_storage = BTreeMap::new();
        minimal_storage.insert(felt!("0x1").into(), felt!("0x123456").into());
        fork_minimal_updates.storage_updates.insert(minimal_contract_address, minimal_storage);
        
        // 3. Nonce update
        fork_minimal_updates.nonce_updates.insert(minimal_contract_address, felt!("0x5").into());
        
        // 4. Declared class
        let minimal_declared_class = felt!("0x0987654321098765432109876543210987654321098765432109876543210987");
        fork_minimal_updates.declared_classes.insert(minimal_declared_class, felt!("0x111").into());
        
        // 5. Deprecated class
        let minimal_deprecated_class = felt!("0x0abcdef123456789abcdef123456789abcdef123456789abcdef123456789abc");
        fork_minimal_updates.deprecated_declared_classes.insert(minimal_deprecated_class);
        
        // 6. Replaced class
        let minimal_replaced_contract = felt!("0x04194c376fcddd757b190476f840f2d211d44c68ba79a6b627fa47e157cd4f97").into();
        let minimal_new_class = felt!("0x033434ad846cdd5f23eb73ff09fe6fddd568284a0fb7d1be20ee482f044dabe2");
        fork_minimal_updates.replaced_classes.insert(minimal_replaced_contract, minimal_new_class);

        println!("📊 Minimal fork updates with {} contracts, {} storage entries, {} nonces, {} declared classes, {} deprecated classes, {} replaced classes", 
            fork_minimal_updates.deployed_contracts.len(),
            fork_minimal_updates.storage_updates.values().map(|s| s.len()).sum::<usize>(),
            fork_minimal_updates.nonce_updates.len(),
            fork_minimal_updates.declared_classes.len(),
            fork_minimal_updates.deprecated_declared_classes.len(),
            fork_minimal_updates.replaced_classes.len()
        );


        let fork_url = url.clone();
        let fork_block = katana_primitives::block::BlockHashOrNumber::Num(block_number);
        // let test_node_config = test_config_forking(fork_url, fork_block);
        // let sequencer = TestNode::new_with_config(test_node_config).await;
        // let backend = sequencer.backend();
        // let blockchain = sequencer.blockchain();
        // let starknet_forked_provider = sequencer.starknet_provider();
        // let provider = backend.blockchain.provider();

        let db = katana_db::init_ephemeral_db().unwrap();
      
        // let mut chain_spec = ChainSpec{
        //     id: ChainId::from_str(s"SN_SEPOLIA").unwrap(),
        //     genesis: GenesisConfig::default(),
        //     fee_contracts: FeeContractsConfig::default(),
        //     settlement: SettlementConfig::default(),
        // };

        // let (forked_blockchain, _) = Blockchain::new_from_forked(db, fork_url, Some(fork_block), &mut chain_spec).await.unwrap();
        // let forked_provider = forked_blockchain.provider();
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
    }
}
