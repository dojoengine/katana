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

    // pub async fn get_storage_proof(
    //     &self,
    //     block_id: BlockIdOrTag,
    //     class_hashes: Option<Vec<ClassHash>>,
    //     contract_addresses: Option<Vec<ContractAddress>>,
    //     contracts_storage_keys: Option<Vec<ContractStorageKeys>>,
    // ) -> Result<GetStorageProofResponse, Error> {
    //     // Validate block is within forked range
    //     match block_id {
    //         BlockIdOrTag::Number(_) => {
    //             unimplemented!() //its not working on rpc - need to find out why
    //         }
    //         BlockIdOrTag::Hash(_) => {
    //             unimplemented!() //its not working on rpc - need to find out why
    //         }
    //         BlockIdOrTag::Tag(tag) => {
    //             if tag != BlockTag::Latest {
    //                 return Err(Error::BlockTagNotAllowed);
    //             }
    //         }
    //     }

    //     // Use the stored URL to create an HttpClient with StarknetApiClient trait
    //     if let Some(url) = &self.url {
    //         // jsonrpsee HttpClientBuilder requires explicit port
    //         let url_with_port = if url.port().is_none() {
    //             let default_port = match url.scheme() {
    //                 "https" => ":443",
    //                 "http" => ":80",
    //                 _ => return Err(Error::JsonRpc(JsonRpcseError::Transport(
    //                     anyhow::anyhow!("Unsupported URL scheme: {}", url.scheme()).into()
    //                 ))),
    //             };
    //             // Manually construct URL with port
    //             format!("{}://{}{}{}",
    //                 url.scheme(),
    //                 url.host_str().unwrap_or(""),
    //                 default_port,
    //                 url.path()
    //             )
    //         } else {
    //             url.to_string()
    //         };

    //         let client = HttpClientBuilder::default().build(&url_with_port)?;
    //         match client.get_storage_proof(
    //             block_id,
    //             class_hashes,
    //             contract_addresses,
    //             contracts_storage_keys,
    //         ).await {
    //             Ok(response) => Ok(response),
    //             Err(e) => {
    //                 tracing::warn!(
    //                     "Storage proof request failed for endpoint {}: {}",
    //                     url, e
    //                 );
    //                 Err(Error::JsonRpc(e))
    //             }
    //         }
    //     } else {
    //         // Fallback for when URL is not available (generic provider)
    //         Err(Error::KatanaProvider(ProviderError::Other(
    //             "Storage proof not supported for this provider type".to_string()
    //         )))
    //     }
    // }

    // /// Get storage proof for state updates - designed to work with trie operations
    // pub async fn get_storage_proof_for_state_update(
    //     &self,
    //     block_id: BlockIdOrTag,
    //     state_updates: &katana_primitives::state::StateUpdates,
    // ) -> Result<GetStorageProofResponse, Error> {
    //     use katana_primitives::state::StateUpdates;

    //     // Collect all the data we need from state updates
    //     let mut class_hashes = Vec::new();
    //     let mut contract_addresses = Vec::new();
    //     let mut contracts_storage_keys = Vec::new();

    //     // Collect class hashes from declared classes
    //     for class_hash in state_updates.declared_classes.keys() {
    //         class_hashes.push(*class_hash);
    //     }

    //     // Collect contract addresses from various updates
    //     for address in state_updates.deployed_contracts.keys() {
    //         contract_addresses.push(*address);
    //     }
    //     for address in state_updates.replaced_classes.keys() {
    //         contract_addresses.push(*address);
    //     }
    //     for address in state_updates.nonce_updates.keys() {
    //         contract_addresses.push(*address);
    //     }

    //     // Collect storage updates
    //     for (address, storage_map) in &state_updates.storage_updates {
    //         contract_addresses.push(*address);
    //         contracts_storage_keys.push(ContractStorageKeys {
    //             address: *address,
    //             keys: storage_map.keys().cloned().collect(),
    //         });
    //     }

    //     // Remove duplicates
    //     contract_addresses.sort();
    //     contract_addresses.dedup();

    //     // Make the actual request
    //     self.get_storage_proof(
    //         block_id,
    //         if class_hashes.is_empty() { None } else { Some(class_hashes) },
    //         if contract_addresses.is_empty() { None } else { Some(contract_addresses) },
    //         if contracts_storage_keys.is_empty() { None } else { Some(contracts_storage_keys) },
    //     ).await
    // }
}

// impl ForkedClient {
//     /// Helper method for making storage proof requests when using JsonRpcClient
//     async fn make_storage_proof_request(
//         &self,
//         block_id: BlockIdOrTag,
//         class_hashes: Option<Vec<ClassHash>>,
//         contract_addresses: Option<Vec<ContractAddress>>,
//         contracts_storage_keys: Option<Vec<ContractStorageKeys>>,
//     ) -> Result<GetStorageProofResponse, Error> {
//         // This method is no longer needed since we handle it directly in get_storage_proof
//         // But we'll keep it for compatibility
//         self.get_storage_proof(block_id, class_hashes, contract_addresses, contracts_storage_keys).await
//     }
// }

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

    // #[tokio::test]
    // async fn get_storage_proof_success() {
    //     let url = Url::parse(SEPOLIA_URL).unwrap();
    //     let client = ForkedClient::new_http(url.clone(), FORK_BLOCK_NUMBER);

    //     // Create a simple StateUpdates object for testing
    //     use katana_primitives::state::StateUpdates;
    //     use std::collections::BTreeMap;

    //     let contract_address = felt!("0x06a4d4e8c1cc9785e125195a2f8bd4e5b0c7510b19f3e2dd63533524f5687e41").into();
    //     let class_hash = felt!("0x03d5de568b28042464214dfbe2ea0d7e22d162986bcdb9f56d691d22955a4c23");
    //     let storage_key = felt!("0x1").into();
    //     let storage_value = felt!("0x42").into();

    //     let mut state_updates = StateUpdates::default();

    //     // Add some sample data
    //     state_updates.deployed_contracts.insert(contract_address, class_hash);
    //     state_updates.storage_updates.insert(
    //         contract_address,
    //         BTreeMap::from([(storage_key, storage_value)])
    //     );
    //     state_updates.declared_classes.insert(class_hash, felt!("0x123").into());

    //     // Step 1: Get proof using ForkedClient
    //     let result = client.get_storage_proof_for_state_update(
    //         BlockIdOrTag::Tag(BlockTag::Latest),
    //         &state_updates,
    //     ).await;

    //     println!("DEBUG: Result: {:?}", result);

    //     match result {
    //         Ok(proof_response) => {
    //             println!("Successfully got storage proof: classes_proof nodes: {}, contracts_proof nodes: {}",
    //                     proof_response.classes_proof.nodes.len(),
    //                     proof_response.contracts_proof.nodes.len());

    //             // Step 2: Convert to MultiProof format
    //             use katana_trie::MultiProof;
    //             let classes_proof = MultiProof::from(proof_response.classes_proof.nodes.clone());
    //             let contracts_proof = MultiProof::from(proof_response.contracts_proof.nodes.clone());

    //             // Extract roots
    //             let classes_tree_root = proof_response.global_roots.classes_tree_root;
    //             let contracts_tree_root = proof_response.global_roots.contracts_tree_root;

    //             println!("Classes proof has {} nodes", classes_proof.0.len());
    //             println!("Contracts proof has {} nodes", contracts_proof.0.len());
    //             println!("Classes tree root: {:#x}", classes_tree_root);
    //             println!("Contracts tree root: {:#x}", contracts_tree_root);

    //             // Step 3: Create ForkedProvider and test trie operations with proof
    //             use katana_provider::providers::fork::ForkedProvider;
    //             use katana_provider::traits::trie::TrieWriter;
    //             use katana_db::mdbx::DbEnv;
    //             use starknet::providers::{JsonRpcClient, Provider};
    //             use starknet::providers::jsonrpc::HttpTransport;
    //             use std::sync::Arc;

    //             // Create the provider
    //             let rpc_provider = Arc::new(JsonRpcClient::new(HttpTransport::new(url.clone())));
    //             let forked_provider = ForkedProvider::new_ephemeral(
    //                 katana_primitives::block::BlockHashOrNumber::Num(FORK_BLOCK_NUMBER),
    //                 rpc_provider
    //             );

    //             // Test declared classes with proof
    //             if !state_updates.declared_classes.is_empty() {
    //                 match forked_provider.trie_insert_declared_classes_with_proof(
    //                     FORK_BLOCK_NUMBER + 1,
    //                     &state_updates.declared_classes,
    //                     classes_proof,
    //                     classes_tree_root,
    //                 ) {
    //                     Ok(computed_root) => {
    //                         println!("✅ Classes trie with proof successful! Computed root: {:#x}", computed_root);
    //                     }
    //                     Err(e) => {
    //                         println!("❌ Classes trie with proof failed: {}", e);
    //                     }
    //                 }
    //             }

    //             // Test contract updates with proof
    //             if !state_updates.deployed_contracts.is_empty()
    //                 || !state_updates.storage_updates.is_empty()
    //             {
    //                 match forked_provider.trie_insert_contract_updates_with_proof(
    //                     FORK_BLOCK_NUMBER + 1,
    //                     &state_updates,
    //                     contracts_proof,
    //                     contracts_tree_root,
    //                 ) {
    //                     Ok(computed_root) => {
    //                         println!("✅ Contracts trie with proof successful! Computed root: {:#x}", computed_root);
    //                     }
    //                     Err(e) => {
    //                         println!("❌ Contracts trie with proof failed: {}", e);
    //                     }
    //                 }
    //             }

    //             // Step 4: Compare with regular trie operations (without proof)
    //             let regular_classes_root = forked_provider.trie_insert_declared_classes(
    //                 FORK_BLOCK_NUMBER + 2,
    //                 &state_updates.declared_classes
    //             ).expect("Regular classes trie should work");

    //             let regular_contracts_root = forked_provider.trie_insert_contract_updates(
    //                 FORK_BLOCK_NUMBER + 2,
    //                 &state_updates
    //             ).expect("Regular contracts trie should work");

    //             println!("📊 Comparison:");
    //             println!("   Regular classes root:  {:#x}", regular_classes_root);
    //             println!("   Regular contracts root: {:#x}", regular_contracts_root);

    //             assert!(!proof_response.contracts_proof.nodes.is_empty(), "Should have contract proofs");
    //         }
    //         Err(e) => {
    //             println!("Storage proof request failed (expected in test env): {}", e);
    //         }
    //     }
    // }

    #[tokio::test]
    async fn test_get_storage_proof() {
        let external_client = JsonRpcClient::new(HttpTransport::new(
            Url::parse("https://api.cartridge.gg/x/starknet/sepolia").unwrap(),
        ));
        let block_number = external_client.block_number().await.unwrap();
        println!("Block number: {:?}", block_number);

        let url = Url::parse(SEPOLIA_URL).unwrap();
        let client = ForkedClient::new_http(url.clone(), block_number);

        // Create a simple StateUpdates object for testing
        use katana_primitives::state::StateUpdates;
        use katana_provider::providers::fork::ForkedProvider;
        use katana_provider::traits::trie::TrieWriter;
        use starknet::providers::jsonrpc::HttpTransport;
        use starknet::providers::JsonRpcClient;
        use std::collections::BTreeMap;

        let contract_address =
            felt!("0x06a4d4e8c1cc9785e125195a2f8bd4e5b0c7510b19f3e2dd63533524f5687e41").into();
        let class_hash =
            felt!("0x03d5de568b28042464214dfbe2ea0d7e22d162986bcdb9f56d691d22955a4c23");
        let storage_key = felt!("0x1").into();
        let storage_value = felt!("0x1").into();

        let mut state_updates = StateUpdates::default();

        // Add some sample data
        state_updates.deployed_contracts.insert(contract_address, class_hash);
        state_updates
            .storage_updates
            .insert(contract_address, BTreeMap::from([(storage_key, storage_value)]));
        state_updates.declared_classes.insert(class_hash, felt!("0x123").into());

        // Create the provider
        let rpc_provider = Arc::new(JsonRpcClient::new(HttpTransport::new(url.clone())));
        let forked_provider = ForkedProvider::new_ephemeral(
            katana_primitives::block::BlockHashOrNumber::Num(block_number),
            rpc_provider,
            url.clone(),
        );

        let state_root = forked_provider.compute_state_root(block_number, &state_updates).unwrap();
        println!("State root: {:?}", state_root);

        let external_state_update =
            external_client.get_state_update(BlockId::Tag(BlockTag::Latest)).await.unwrap();

        let external_state_root = match external_state_update {
            StarknetRsMaybePendingStateUpdate::Update(state_update) => {
                println!("External new_root: {:#x}", state_update.new_root);
                println!("External old_root: {:#x}", state_update.old_root);
                println!("External block_hash: {:#x}", state_update.block_hash);
                state_update.new_root
            }
            StarknetRsMaybePendingStateUpdate::PendingUpdate(pending) => {
                println!("Pending state update - no state root available");
                return;
            }
        };
    }
}
