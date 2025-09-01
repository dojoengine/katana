use jsonrpsee::core::ClientError;
use jsonrpsee::http_client::HttpClient;
use katana_primitives::block::{BlockHash, BlockIdOrTag, BlockNumber};
use katana_primitives::contract::ContractAddress;
use katana_primitives::transaction::TxHash;
use katana_primitives::Felt;
use katana_rpc_api::error::starknet::StarknetApiError;
use katana_rpc_api::starknet::StarknetApiClient;
use katana_rpc_types::block::{
    MaybePreConfirmedBlockWithReceipts, MaybePreConfirmedBlockWithTxHashes,
    MaybePreConfirmedBlockWithTxs,
};
use katana_rpc_types::event::{
    EventFilter, EventFilterWithPage, GetEventsResponse, ResultPageRequest,
};
use katana_rpc_types::receipt::{ReceiptBlock, TxReceiptWithBlockInfo};
use katana_rpc_types::state_update::MaybePreConfirmedStateUpdate;
use katana_rpc_types::transaction::RpcTxWithHash;
use starknet::core::types::TransactionStatus;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("client error: {0}")]
    ClientError(ClientError),

    /// Error that occurs in the JSON-RPC call itself.
    #[error("call error: {0}")]
    CallError(jsonrpsee::types::ErrorObjectOwned),

    #[error("block out of range")]
    BlockOutOfRange,

    #[error("not allowed to use block tag as a block identifier")]
    BlockTagNotAllowed,

    #[error("unexpected pending data")]
    UnexpectedPendingData,
}

#[derive(Debug, Clone)]
pub struct ForkedClient {
    /// The block number where the node is forked from.
    block: BlockNumber,
    /// The Starknet JSON-RPC client for doing the request to the forked network.
    client: HttpClient,
}

impl ForkedClient {
    /// Creates a new forked client from the given [`Provider`] and block number.
    pub fn new(client: HttpClient, block: BlockNumber) -> Self {
        Self { block, client }
    }

    /// Returns the block number of the forked client.
    pub fn block(&self) -> &BlockNumber {
        &self.block
    }
}

impl ForkedClient {
    pub async fn get_block_number_by_hash(&self, hash: BlockHash) -> Result<BlockNumber, Error> {
        let block = self.client.get_block_with_tx_hashes(BlockIdOrTag::Hash(hash)).await?;
        // Pending block doesn't have a hash yet, so if we get a pending block, we return an error.
        let MaybePreConfirmedBlockWithTxHashes::Block(block) = block else {
            return Err(Error::UnexpectedPendingData);
        };

        if block.block_number > self.block {
            Err(Error::BlockOutOfRange)
        } else {
            Ok(block.block_number)
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

        if let ReceiptBlock::Block { block_number, .. } = receipt.block {
            if block_number > self.block {
                return Err(Error::BlockOutOfRange);
            }
        }

        Ok(receipt)
    }

    pub async fn get_transaction_status(&self, hash: TxHash) -> Result<TransactionStatus, Error> {
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
                    MaybePreConfirmedBlockWithTxHashes::Block(block) => block.block_number,
                    MaybePreConfirmedBlockWithTxHashes::PreConfirmed(_) => {
                        return Err(Error::UnexpectedPendingData);
                    }
                };

                if number > self.block {
                    return Err(Error::BlockOutOfRange);
                }

                Ok(tx?)
            }

            BlockIdOrTag::Tag(_) => Err(Error::BlockTagNotAllowed),
        }
    }

    pub async fn get_block_with_txs(
        &self,
        block_id: BlockIdOrTag,
    ) -> Result<MaybePreConfirmedBlockWithTxs, Error> {
        let block = self.client.get_block_with_txs(block_id).await?;

        match block {
            MaybePreConfirmedBlockWithTxs::PreConfirmed(_) => Err(Error::UnexpectedPendingData),
            MaybePreConfirmedBlockWithTxs::Block(ref b) => {
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
    ) -> Result<MaybePreConfirmedBlockWithReceipts, Error> {
        let block = self.client.get_block_with_receipts(block_id).await?;

        match block {
            MaybePreConfirmedBlockWithReceipts::Block(ref b) => {
                if b.block_number > self.block {
                    return Err(Error::BlockOutOfRange);
                }
            }
            MaybePreConfirmedBlockWithReceipts::PreConfirmed(_) => {
                return Err(Error::UnexpectedPendingData);
            }
        }

        Ok(block)
    }

    pub async fn get_block_with_tx_hashes(
        &self,
        block_id: BlockIdOrTag,
    ) -> Result<MaybePreConfirmedBlockWithTxHashes, Error> {
        let block = self.client.get_block_with_tx_hashes(block_id).await?;

        match block {
            MaybePreConfirmedBlockWithTxHashes::Block(ref b) => {
                if b.block_number > self.block {
                    return Err(Error::BlockOutOfRange);
                }
            }
            MaybePreConfirmedBlockWithTxHashes::PreConfirmed(_) => {
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
                if let MaybePreConfirmedBlockWithTxHashes::Block(b) = block {
                    if b.block_number > self.block {
                        return Err(Error::BlockOutOfRange);
                    }
                }
            }
            BlockIdOrTag::Tag(_) => {
                return Err(Error::BlockTagNotAllowed);
            }
            _ => {}
        }

        Ok(self.client.get_block_transaction_count(block_id).await?)
    }

    pub async fn get_state_update(
        &self,
        block_id: BlockIdOrTag,
    ) -> Result<MaybePreConfirmedStateUpdate, Error> {
        match block_id {
            BlockIdOrTag::Number(num) if num > self.block => {
                return Err(Error::BlockOutOfRange);
            }
            BlockIdOrTag::Hash(hash) => {
                let block = self.client.get_block_with_tx_hashes(BlockIdOrTag::Hash(hash)).await?;
                if let MaybePreConfirmedBlockWithTxHashes::Block(b) = block {
                    if b.block_number > self.block {
                        return Err(Error::BlockOutOfRange);
                    }
                }
            }
            BlockIdOrTag::Tag(_) => {
                return Err(Error::BlockTagNotAllowed);
            }
            _ => {}
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

        let filter = EventFilter { address, from_block, to_block, keys };
        let result_page_request = ResultPageRequest { chunk_size, continuation_token };

        Ok(self.client.get_events(EventFilterWithPage { filter, result_page_request }).await?)
    }
}

impl From<Error> for StarknetApiError {
    fn from(value: Error) -> Self {
        match value {
            Error::CallError(error) => StarknetApiError::from(error),
            Error::ClientError(error) => StarknetApiError::unexpected(error),
            Error::BlockOutOfRange => StarknetApiError::BlockNotFound,
            Error::BlockTagNotAllowed | Error::UnexpectedPendingData => {
                StarknetApiError::unexpected(value)
            }
        }
    }
}

impl From<ClientError> for Error {
    fn from(error: ClientError) -> Self {
        if let ClientError::Call(call_error) = error {
            Self::CallError(call_error)
        } else {
            Self::ClientError(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use jsonrpsee::http_client::HttpClientBuilder;
    use katana_primitives::felt;

    use super::*;

    const SEPOLIA_URL: &str = "https://api.cartridge.gg/x/starknet/sepolia/rpc/v0_8";
    const FORK_BLOCK_NUMBER: BlockNumber = 268_471;

    #[tokio::test]
    async fn get_block_hash() {
        let http_client = HttpClientBuilder::new().build(SEPOLIA_URL).unwrap();
        let client = ForkedClient::new(http_client, FORK_BLOCK_NUMBER);

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
}
