use std::fmt::Debug;

use katana_core::service::block_producer::BlockProducer;
use katana_primitives::block::PartialHeader;
use katana_primitives::da::L1DataAvailabilityMode;
use katana_primitives::execution::TypedTransactionExecutionInfo;
use katana_primitives::transaction::{TxHash, TxNumber};
use katana_primitives::version::CURRENT_STARKNET_VERSION;
use katana_provider::api::state::StateProvider;
use katana_provider::{ProviderFactory, ProviderRO, ProviderRW};
use katana_rpc_types::{
    FinalityStatus, PreConfirmedBlockWithReceipts, PreConfirmedBlockWithTxHashes,
    PreConfirmedBlockWithTxs, PreConfirmedStateUpdate, ReceiptBlockInfo, RpcTxWithHash,
    TxReceiptWithBlockInfo, TxTrace,
};

use crate::starknet::StarknetApiResult;

#[auto_impl::auto_impl(Box)]
pub trait PendingBlockProvider: Debug + Send + Sync + 'static {
    fn pending_state(&self) -> StarknetApiResult<Option<Box<dyn StateProvider>>>;

    fn get_pending_state_update(&self) -> StarknetApiResult<Option<PreConfirmedStateUpdate>>;

    fn get_pending_block_with_txs(&self) -> StarknetApiResult<Option<PreConfirmedBlockWithTxs>>;

    fn get_pending_block_with_receipts(
        &self,
    ) -> StarknetApiResult<Option<PreConfirmedBlockWithReceipts>>;

    fn get_pending_block_with_tx_hashes(
        &self,
    ) -> StarknetApiResult<Option<PreConfirmedBlockWithTxHashes>>;

    fn get_pending_transaction(&self, hash: TxHash) -> StarknetApiResult<Option<RpcTxWithHash>>;

    fn get_pending_receipt(
        &self,
        hash: TxHash,
    ) -> StarknetApiResult<Option<TxReceiptWithBlockInfo>>;

    fn get_pending_trace(&self, hash: TxHash) -> StarknetApiResult<Option<TxTrace>>;

    fn get_pending_transaction_by_index(
        &self,
        index: TxNumber,
    ) -> StarknetApiResult<Option<RpcTxWithHash>>;
}

fn pending_header(block_env: &katana_primitives::env::BlockEnv) -> PartialHeader {
    PartialHeader {
        l1_da_mode: L1DataAvailabilityMode::Calldata,
        l2_gas_prices: block_env.l2_gas_prices.clone(),
        l1_gas_prices: block_env.l1_gas_prices.clone(),
        l1_data_gas_prices: block_env.l1_data_gas_prices.clone(),
        number: block_env.number,
        parent_hash: Default::default(),
        timestamp: block_env.timestamp,
        sequencer_address: block_env.sequencer_address,
        starknet_version: CURRENT_STARKNET_VERSION,
    }
}

impl<PF> PendingBlockProvider for BlockProducer<PF>
where
    PF: ProviderFactory,
    <PF as ProviderFactory>::Provider: ProviderRO,
    <PF as ProviderFactory>::ProviderMut: ProviderRW,
{
    fn pending_state(&self) -> StarknetApiResult<Option<Box<dyn StateProvider>>> {
        Ok(BlockProducer::pending_state(self))
    }

    fn get_pending_transaction(&self, hash: TxHash) -> StarknetApiResult<Option<RpcTxWithHash>> {
        let Some(executor) = self.pending_executor() else {
            return Ok(None);
        };

        let result = executor
            .read()
            .transactions()
            .iter()
            .find(|(tx, ..)| tx.hash == hash)
            .map(|(tx, ..)| RpcTxWithHash::from(tx.clone()));

        Ok(result)
    }

    fn get_pending_block_with_txs(&self) -> StarknetApiResult<Option<PreConfirmedBlockWithTxs>> {
        let Some(executor) = self.pending_executor() else {
            return Ok(None);
        };

        let lock = executor.read();
        let header = pending_header(&lock.block_env());

        // A block should only include successful transactions, we filter out the failed ones.
        let body = lock
            .transactions()
            .iter()
            .filter(|(_, receipt)| receipt.is_success())
            .map(|(tx, _)| tx.clone())
            .collect::<Vec<_>>();

        Ok(Some(PreConfirmedBlockWithTxs::new(header, body)))
    }

    fn get_pending_block_with_receipts(
        &self,
    ) -> StarknetApiResult<Option<PreConfirmedBlockWithReceipts>> {
        let Some(executor) = self.pending_executor() else {
            return Ok(None);
        };

        let lock = executor.read();
        let header = pending_header(&lock.block_env());

        let body = lock
            .transactions()
            .iter()
            .filter(|(_, receipt)| receipt.is_success())
            .map(|(tx, receipt)| (tx.clone(), receipt.receipt().cloned().unwrap()));

        Ok(Some(PreConfirmedBlockWithReceipts::new(header, body)))
    }

    fn get_pending_block_with_tx_hashes(
        &self,
    ) -> StarknetApiResult<Option<PreConfirmedBlockWithTxHashes>> {
        let Some(executor) = self.pending_executor() else {
            return Ok(None);
        };

        let lock = executor.read();
        let header = pending_header(&lock.block_env());

        let body = lock
            .transactions()
            .iter()
            .filter(|(_, receipt)| receipt.is_success())
            .map(|(tx, _)| tx.hash)
            .collect::<Vec<TxHash>>();

        Ok(Some(PreConfirmedBlockWithTxHashes::new(header, body)))
    }

    fn get_pending_receipt(
        &self,
        hash: TxHash,
    ) -> StarknetApiResult<Option<TxReceiptWithBlockInfo>> {
        let Some(executor) = self.pending_executor() else {
            return Ok(None);
        };

        let lock = executor.read();

        let result = lock.transactions().iter().find_map(|(tx, receipt)| {
            if tx.hash == hash {
                receipt.receipt().cloned()
            } else {
                None
            }
        });

        if let Some(receipt) = result {
            let pending_block_number = lock.block_env().number;

            let receipt = TxReceiptWithBlockInfo::new(
                ReceiptBlockInfo::PreConfirmed { block_number: pending_block_number },
                hash,
                FinalityStatus::AcceptedOnL2,
                receipt,
            );

            Ok(Some(receipt))
        } else {
            Ok(None)
        }
    }

    fn get_pending_trace(&self, hash: TxHash) -> StarknetApiResult<Option<TxTrace>> {
        let Some(executor) = self.pending_executor() else {
            return Ok(None);
        };

        let lock = executor.read();

        let result = lock.transactions().iter().find(|(t, _)| t.hash == hash);

        if let Some((tx, res)) = result {
            if let Some(trace) = res.trace() {
                let trace = TypedTransactionExecutionInfo::new(tx.r#type(), trace.clone());
                return Ok(Some(TxTrace::from(trace)));
            }
        }

        Ok(None)
    }

    fn get_pending_state_update(&self) -> StarknetApiResult<Option<PreConfirmedStateUpdate>> {
        Ok(None)
    }

    fn get_pending_transaction_by_index(
        &self,
        index: TxNumber,
    ) -> StarknetApiResult<Option<RpcTxWithHash>> {
        let Some(executor) = self.pending_executor() else {
            return Ok(None);
        };

        let result = executor
            .read()
            .transactions()
            .get(index as usize)
            .map(|(tx, ..)| tx.clone())
            .map(RpcTxWithHash::from);

        Ok(result)
    }
}
