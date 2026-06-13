//! Starknet settlement chain — the Piltover core contract.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use katana_primitives::block::BlockNumber;
use katana_primitives::chain::ChainId;
use katana_primitives::{ContractAddress, Felt};
use piltover::{AppchainContract, AppchainContractReader, PiltoverInput};
use starknet::accounts::{ExecutionEncoding, SingleOwnerAccount};
use starknet::core::types::{BlockId, BlockTag, TransactionReceipt};
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::{JsonRpcClient, Provider};
use starknet::signers::{LocalWallet, SigningKey};
use tracing::debug;
use url::Url;

use super::SettlementChain;
use crate::LOG_TARGET;

/// Receipt polling interval while waiting for a settlement transaction.
const RECEIPT_POLL_INTERVAL: Duration = Duration::from_secs(2);

type SettlementAccount = SingleOwnerAccount<Arc<JsonRpcClient<HttpTransport>>, LocalWallet>;

/// A Starknet settlement chain, settled to via the Piltover appchain core
/// contract. Accepts [`PiltoverInput`] state updates.
#[allow(missing_debug_implementations)]
pub struct StarknetSettlementChain {
    provider: Arc<JsonRpcClient<HttpTransport>>,
    reader: AppchainContractReader<Arc<JsonRpcClient<HttpTransport>>>,
    contract: AppchainContract<SettlementAccount>,
}

impl StarknetSettlementChain {
    /// `chain_id` is the settlement chain's id as recorded in the rollup chain
    /// spec at init time — used for transaction signing without an upfront
    /// RPC round trip.
    pub fn new(
        rpc_url: Url,
        chain_id: ChainId,
        core_contract: ContractAddress,
        account_address: ContractAddress,
        account_private_key: Felt,
    ) -> Self {
        let provider = Arc::new(JsonRpcClient::new(HttpTransport::new(rpc_url)));

        let mut account = SingleOwnerAccount::new(
            provider.clone(),
            LocalWallet::from_signing_key(SigningKey::from_secret_scalar(account_private_key)),
            account_address.into(),
            chain_id.id(),
            ExecutionEncoding::New,
        );
        account.set_block_id(BlockId::Tag(BlockTag::Latest));

        let reader = AppchainContractReader::new(core_contract.into(), provider.clone());
        let contract = AppchainContract::new(core_contract.into(), account);

        Self { provider, reader, contract }
    }

    /// Polls the settlement chain until the transaction lands; errors on revert.
    async fn watch_tx(&self, tx_hash: Felt) -> Result<()> {
        loop {
            tokio::time::sleep(RECEIPT_POLL_INTERVAL).await;
            match self.provider.get_transaction_receipt(tx_hash).await {
                Ok(receipt) => match receipt.receipt {
                    TransactionReceipt::Invoke(r) => {
                        use starknet::core::types::ExecutionResult;
                        match r.execution_result {
                            ExecutionResult::Succeeded => return Ok(()),
                            ExecutionResult::Reverted { reason } => {
                                return Err(anyhow!("settlement transaction reverted: {reason}"))
                            }
                        }
                    }
                    _ => return Ok(()),
                },
                Err(starknet::providers::ProviderError::StarknetError(
                    starknet::core::types::StarknetError::TransactionHashNotFound,
                )) => {
                    debug!(target: LOG_TARGET, tx_hash = %format!("{tx_hash:#x}"), "Waiting for settlement transaction.");
                    continue;
                }
                Err(e) => return Err(e.into()),
            }
        }
    }
}

#[async_trait]
impl SettlementChain for StarknetSettlementChain {
    type StateUpdate = PiltoverInput;

    fn name(&self) -> &'static str {
        "starknet (piltover)"
    }

    /// Reads the settled block number from the contract's `AppchainState`.
    ///
    /// `None` corresponds to Piltover's `Felt::MAX` genesis sentinel (nothing
    /// settled yet).
    async fn settled_block(&self) -> Result<Option<BlockNumber>> {
        // AppchainState: (state_root, block_number, block_hash).
        let (_, block_number, _) = self
            .reader
            .get_state()
            .block_id(BlockId::Tag(BlockTag::Latest))
            .call()
            .await
            .context("piltover get_state")?;

        if block_number == Felt::MAX {
            return Ok(None);
        }

        let block_number = u64::try_from(block_number)
            .map_err(|_| anyhow!("piltover block number {block_number:#x} does not fit in u64"))?;

        Ok(Some(block_number))
    }

    /// Submits `update_state` and waits for the transaction to be confirmed.
    async fn update_state(&self, update: PiltoverInput) -> Result<String> {
        let execution = self.contract.update_state(&update);

        execution.estimate_fee().await.context("estimate update_state fee")?;
        let tx = execution.send().await.context("send update_state")?;

        self.watch_tx(tx.transaction_hash).await?;

        Ok(format!("{:#x}", tx.transaction_hash))
    }
}
