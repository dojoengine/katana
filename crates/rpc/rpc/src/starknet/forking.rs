use katana_primitives::block::{BlockHash, BlockIdOrTag, BlockNumber};
use katana_primitives::contract::ContractAddress;
use katana_primitives::transaction::TxHash;
use katana_primitives::Felt;
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
}

#[derive(Debug, Clone)]
pub struct ForkedClient {
    /// The block number where the node is forked from.
    block: BlockIdOrTag,
    /// The Starknet JSON-RPC client for doing the request to the forked network.
    client: Client,
}

impl ForkedClient {
    /// Creates a new forked client from the given [`Client`] and block number.
    pub fn new(client: Client, block: BlockIdOrTag) -> Self {
        Self { block, client }
    }

    /// Returns the block number of the forked client.
    pub fn block(&self) -> &BlockIdOrTag {
        &self.block
    }
}

impl ForkedClient {
    pub async fn get_block_number_by_hash(&self, hash: BlockHash) -> Result<BlockNumber, Error> {
        let number = match self.client.get_block_with_tx_hashes(BlockIdOrTag::Hash(hash)).await? {
            GetBlockWithTxHashesResponse::Block(block) => block.block_number,
            GetBlockWithTxHashesResponse::PreConfirmed(block) => block.block_number,
        };

        match self.block {
            BlockIdOrTag::Number(fork_num) => {
                if number > fork_num {
                    Err(Error::BlockOutOfRange)
                } else {
                    Ok(number)
                }
            }
            _ => Ok(number),
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
            if let BlockIdOrTag::Number(fork_num) = self.block {
                if block_number > fork_num {
                    return Err(Error::BlockOutOfRange);
                }
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
                if let BlockIdOrTag::Number(fork_num) = self.block {
                    if num > fork_num {
                        return Err(Error::BlockOutOfRange);
                    }
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

                if let BlockIdOrTag::Number(fork_num) = self.block {
                    if number > fork_num {
                        return Err(Error::BlockOutOfRange);
                    }
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
        Ok(block)

        // match block {
        //     MaybePreConfirmedBlock::PreConfirmed(_) => Err(Error::UnexpectedPendingData),
        //     MaybePreConfirmedBlock::Confirmed(ref b) => {
        //         if let BlockIdOrTag::Number(fork_num) = self.block {
        //             if b.block_number > fork_num {
        //                 return Err(Error::BlockOutOfRange);
        //             }
        //         }

        //         Ok(block)
        //     }
        // }
    }

    pub async fn get_block_with_receipts(
        &self,
        block_id: BlockIdOrTag,
    ) -> Result<GetBlockWithReceiptsResponse, Error> {
        let block = self.client.get_block_with_receipts(block_id).await?;

        // match block {
        //     GetBlockWithReceiptsResponse::Block(ref b) => {
        //         if let BlockIdOrTag::Number(fork_num) = self.block {
        //             if b.block_number > fork_num {
        //                 return Err(Error::BlockOutOfRange);
        //             }
        //         }
        //     }
        //     GetBlockWithReceiptsResponse::PreConfirmed(_) => {
        //         return Err(Error::UnexpectedPendingData);
        //     }
        // }

        Ok(block)
    }

    pub async fn get_block_with_tx_hashes(
        &self,
        block_id: BlockIdOrTag,
    ) -> Result<GetBlockWithTxHashesResponse, Error> {
        let block = self.client.get_block_with_tx_hashes(block_id).await?;

        match block {
            GetBlockWithTxHashesResponse::Block(ref b) => {
                if let BlockIdOrTag::Number(fork_num) = self.block {
                    if b.block_number > fork_num {
                        return Err(Error::BlockOutOfRange);
                    }
                }
            }
            GetBlockWithTxHashesResponse::PreConfirmed(_) => {
                return Err(Error::UnexpectedPendingData);
            }
        }

        Ok(block)
    }

    pub async fn get_block_transaction_count(&self, block_id: BlockIdOrTag) -> Result<u64, Error> {
        Ok(self.client.get_block_transaction_count(block_id).await?)
    }

    pub async fn get_state_update(&self, block_id: BlockIdOrTag) -> Result<StateUpdate, Error> {
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
        // if from > self.block || to > self.block {
        //     return Err(Error::BlockOutOfRange);
        // }

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
}
