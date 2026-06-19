//! Client for the Piltover appchain core contract on the settlement chain.
//!
//! The settlement service settles to a Starknet chain via Piltover; this is
//! the chain-side half — it owns the RPC client, the settlement account used
//! to sign `update_state` transactions, and the contract binding.

use std::sync::Arc;
use std::time::Duration;

use katana_primitives::block::BlockNumber;
use katana_primitives::chain::ChainId;
use katana_primitives::transaction::TxHash;
use katana_primitives::{ContractAddress, Felt};
use piltover::{AppchainContract, PiltoverInput};
use starknet::accounts::{ExecutionEncoding, SingleOwnerAccount};
use starknet::core::types::{BlockId, BlockTag, TransactionReceipt};
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::{JsonRpcClient, Provider};
use starknet::signers::{LocalWallet, SigningKey};
use tracing::debug;
use url::Url;

/// Receipt polling interval while waiting for a settlement transaction.
const RECEIPT_POLL_INTERVAL: Duration = Duration::from_secs(2);

type SettlementAccount = SingleOwnerAccount<Arc<JsonRpcClient<HttpTransport>>, LocalWallet>;

/// Errors from interacting with the Piltover core contract on the settlement chain.
#[derive(Debug, thiserror::Error)]
pub enum PiltoverError {
    #[error("reading piltover state failed: {0}")]
    GetState(String),

    #[error("piltover block number {0:#x} does not fit in u64")]
    InvalidBlockNumber(Felt),

    #[error("estimating update_state fee failed: {0}")]
    EstimateFee(String),

    #[error("submitting update_state failed: {0}")]
    SendTransaction(String),

    #[error("settlement transaction reverted: {0}")]
    TransactionReverted(String),

    #[error("polling settlement transaction receipt failed: {0}")]
    Receipt(String),
}

/// Client for the Piltover appchain core contract on the settlement chain.
#[allow(missing_debug_implementations)]
pub struct PiltoverClient {
    provider: Arc<JsonRpcClient<HttpTransport>>,
    contract: AppchainContract<SettlementAccount>,
}

impl PiltoverClient {
    /// `chain_id` is the settlement chain's id as recorded in the rollup chain
    /// spec at init time — used for transaction signing without an upfront RPC
    /// round trip.
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

        let contract = AppchainContract::new(core_contract.into(), account);

        Self { provider, contract }
    }

    /// Reads the settled block number from the contract's `AppchainState`.
    ///
    /// `None` corresponds to Piltover's `Felt::MAX` genesis sentinel (nothing
    /// settled yet).
    pub async fn settled_block(&self) -> Result<Option<BlockNumber>, PiltoverError> {
        // AppchainState: (state_root, block_number, block_hash).
        let (_, block_number, _) = self
            .contract
            .get_state()
            .block_id(BlockId::Tag(BlockTag::Latest))
            .call()
            .await
            .map_err(|e| PiltoverError::GetState(e.to_string()))?;

        if block_number == Felt::MAX {
            return Ok(None);
        }

        let block_number = u64::try_from(block_number)
            .map_err(|_| PiltoverError::InvalidBlockNumber(block_number))?;

        Ok(Some(block_number))
    }

    /// Submits `update_state` and waits for the transaction to be confirmed.
    pub async fn update_state(&self, update: PiltoverInput) -> Result<TxHash, PiltoverError> {
        let tx = self
            .contract
            .update_state(&update)
            .send()
            .await
            .map_err(|e| PiltoverError::SendTransaction(e.to_string()))?;

        let tx_hash = tx.transaction_hash;
        self.watch_tx(tx_hash).await?;

        Ok(tx_hash)
    }

    /// Polls the settlement chain until the transaction lands; errors on revert.
    async fn watch_tx(&self, tx_hash: TxHash) -> Result<(), PiltoverError> {
        loop {
            tokio::time::sleep(RECEIPT_POLL_INTERVAL).await;
            match self.provider.get_transaction_receipt(tx_hash).await {
                Ok(receipt) => match receipt.receipt {
                    TransactionReceipt::Invoke(r) => {
                        use starknet::core::types::ExecutionResult;
                        match r.execution_result {
                            ExecutionResult::Succeeded => return Ok(()),
                            ExecutionResult::Reverted { reason } => {
                                return Err(PiltoverError::TransactionReverted(reason))
                            }
                        }
                    }
                    _ => return Ok(()),
                },

                Err(starknet::providers::ProviderError::StarknetError(
                    starknet::core::types::StarknetError::TransactionHashNotFound,
                )) => {
                    debug!(tx_hash = %format!("{tx_hash:#x}"), "Waiting for settlement transaction.");
                    continue;
                }

                Err(e) => return Err(PiltoverError::Receipt(e.to_string())),
            }
        }
    }
}
