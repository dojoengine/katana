use katana_primitives::block::BlockHashOrNumber;
use katana_provider::api::block::{BlockHashProvider, BlockProvider, BlockStatusProvider};
use katana_provider::api::transaction::ReceiptProvider;
use katana_provider::ProviderResult;
use katana_rpc_types::block::{ConfirmedBlockWithReceipts, ConfirmedBlockWithTxHashes};
use katana_rpc_types::ConfirmedBlockWithTxs;

/// A builder for building RPC block types.
#[derive(Debug)]
pub struct BlockBuilder<P> {
    provider: P,
    block_id: BlockHashOrNumber,
}

impl<P> BlockBuilder<P> {
    pub fn new(block_id: BlockHashOrNumber, provider: P) -> Self {
        Self { provider, block_id }
    }
}

impl<P> BlockBuilder<P>
where
    P: BlockProvider + BlockHashProvider + ReceiptProvider,
{
    pub fn build(self) -> ProviderResult<Option<ConfirmedBlockWithTxs>> {
        let Some(hash) = BlockHashProvider::block_hash_by_id(&self.provider, self.block_id)? else {
            return Ok(None);
        };

        let block = BlockProvider::block(&self.provider, self.block_id)?
            .expect("should exist if hash exists");
        let finality_status = BlockStatusProvider::block_status(&self.provider, self.block_id)?
            .expect("should exist if block exists");

        Ok(Some(ConfirmedBlockWithTxs::new(hash, block, finality_status)))
    }

    pub fn build_with_tx_hash(self) -> ProviderResult<Option<ConfirmedBlockWithTxHashes>> {
        let Some(hash) = BlockHashProvider::block_hash_by_id(&self.provider, self.block_id)? else {
            return Ok(None);
        };

        let block = BlockProvider::block_with_tx_hashes(&self.provider, self.block_id)?
            .expect("should exist if block exists");
        let finality_status = BlockStatusProvider::block_status(&self.provider, self.block_id)?
            .expect("should exist if block exists");

        Ok(Some(ConfirmedBlockWithTxHashes::new(hash, block, finality_status)))
    }

    pub fn build_with_receipts(self) -> ProviderResult<Option<ConfirmedBlockWithReceipts>> {
        let Some(hash) = BlockHashProvider::block_hash_by_id(&self.provider, self.block_id)? else {
            return Ok(None);
        };

        let block = BlockProvider::block(&self.provider, self.block_id)?
            .expect("should exist if block exists");
        let finality_status = BlockStatusProvider::block_status(&self.provider, self.block_id)?
            .expect("should exist if block exists");
        let receipts = ReceiptProvider::receipts_by_block(&self.provider, self.block_id)?
            .expect("should exist if block exists");

        let receipts_with_txs = block.body.into_iter().zip(receipts);

        Ok(Some(ConfirmedBlockWithReceipts::new(
            hash,
            block.header,
            finality_status,
            receipts_with_txs,
        )))
    }
}
