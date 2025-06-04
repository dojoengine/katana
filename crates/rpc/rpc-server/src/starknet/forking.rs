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
