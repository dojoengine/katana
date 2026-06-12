//! Piltover core contract client — the settlement chain side of the service.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use cainome::cairo_serde::ContractAddress as CainomeContractAddress;
use katana_primitives::block::BlockNumber;
use katana_primitives::transaction::TxHash;
use katana_primitives::utils::transaction::compute_starknet_to_appchain_message_hash;
use katana_primitives::{ContractAddress, Felt};
use katana_rpc_types::tee::BlockAttestation;
use katana_rpc_types::{L1ToL2Message, L2ToL1Message};
use piltover::{
    AppchainContract, AppchainContractReader, MessageToAppchain, MessageToStarknet, PiltoverInput,
    TEEInput,
};
use starknet::accounts::{ExecutionEncoding, SingleOwnerAccount};
use starknet::core::types::{BlockId, BlockTag, TransactionReceipt};
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::{JsonRpcClient, Provider};
use starknet::signers::{LocalWallet, SigningKey};
use tracing::debug;
use url::Url;

use crate::LOG_TARGET;

/// Receipt polling interval while waiting for a settlement transaction.
const RECEIPT_POLL_INTERVAL: Duration = Duration::from_secs(2);

type SettlementAccount = SingleOwnerAccount<Arc<JsonRpcClient<HttpTransport>>, LocalWallet>;

/// Client for the Piltover appchain core contract on the settlement chain.
#[allow(missing_debug_implementations)]
pub struct PiltoverClient {
    provider: Arc<JsonRpcClient<HttpTransport>>,
    reader: AppchainContractReader<Arc<JsonRpcClient<HttpTransport>>>,
    contract: AppchainContract<SettlementAccount>,
}

impl PiltoverClient {
    pub async fn new(
        rpc_url: Url,
        core_contract: ContractAddress,
        account_address: ContractAddress,
        account_private_key: Felt,
    ) -> Result<Self> {
        let provider = Arc::new(JsonRpcClient::new(HttpTransport::new(rpc_url)));
        let chain_id = provider.chain_id().await.context("fetch settlement chain id")?;

        let mut account = SingleOwnerAccount::new(
            provider.clone(),
            LocalWallet::from_signing_key(SigningKey::from_secret_scalar(account_private_key)),
            account_address.into(),
            chain_id,
            ExecutionEncoding::New,
        );
        account.set_block_id(BlockId::Tag(BlockTag::Latest));

        let reader = AppchainContractReader::new(core_contract.into(), provider.clone());
        let contract = AppchainContract::new(core_contract.into(), account);

        Ok(Self { provider, reader, contract })
    }

    /// Reads the settled block number from the contract's `AppchainState`.
    ///
    /// Returns `None` when nothing has been settled yet (Piltover's `Felt::MAX` genesis
    /// sentinel).
    pub async fn settled_block(&self) -> Result<Option<BlockNumber>> {
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

    /// Submits `update_state(TeeInput)` and waits for the transaction to be confirmed.
    pub async fn update_state(&self, input: PiltoverInput) -> Result<TxHash> {
        let execution = self.contract.update_state(&input);

        execution.estimate_fee().await.context("estimate update_state fee")?;
        let tx = execution.send().await.context("send update_state")?;

        self.watch_tx(tx.transaction_hash).await?;

        Ok(tx.transaction_hash)
    }

    /// Polls the settlement chain until the transaction lands; errors on revert.
    async fn watch_tx(&self, tx_hash: TxHash) -> Result<()> {
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

/// Builds the `PiltoverInput::TeeInput` payload from an attestation and its proof felts.
pub fn build_tee_input(attestation: &BlockAttestation, sp1_proof: Vec<Felt>) -> PiltoverInput {
    let l1_to_l2_msg_hashes =
        attestation.l1_to_l2_messages.iter().map(compute_l1_to_l2_msg_hash).collect();

    PiltoverInput::TeeInput(TEEInput {
        sp1_proof,
        prev_state_root: attestation.prev_state_root,
        state_root: attestation.state_root,
        prev_block_hash: attestation.prev_block_hash,
        block_hash: attestation.block_hash,
        prev_block_number: attestation.prev_block_number,
        block_number: attestation.block_number,
        messages_commitment: attestation.messages_commitment,
        messages_to_starknet: messages_to_starknet(&attestation.l2_to_l1_messages),
        messages_to_appchain: messages_to_appchain(&attestation.l1_to_l2_messages),
        l1_to_l2_msg_hashes,
        katana_tee_config_hash: attestation.katana_tee_config_hash,
    })
}

/// Computes the settlement-to-appchain message hash for a Starknet settlement layer.
///
/// Must match the hash Katana's messaging collector stamps into `Receipt::L1Handler` — the
/// attestation's `messages_commitment` is built from those receipt hashes, and Piltover asserts
/// this list re-hashes to the same commitment.
fn compute_l1_to_l2_msg_hash(msg: &L1ToL2Message) -> Felt {
    // calldata = [from_address, ...payload], mirroring the L1HandlerTx calldata layout.
    let mut calldata = Vec::with_capacity(msg.payload.len() + 1);
    calldata.push(msg.from_address);
    calldata.extend_from_slice(&msg.payload);

    compute_starknet_to_appchain_message_hash(
        msg.from_address,
        msg.to_address.into(),
        msg.nonce,
        msg.entry_point_selector,
        &calldata,
    )
}

fn messages_to_starknet(msgs: &[L2ToL1Message]) -> Vec<MessageToStarknet> {
    msgs.iter()
        .map(|m| MessageToStarknet {
            from_address: CainomeContractAddress(m.from_address.into()),
            to_address: CainomeContractAddress(m.to_address),
            payload: m.payload.clone(),
        })
        .collect()
}

fn messages_to_appchain(msgs: &[L1ToL2Message]) -> Vec<MessageToAppchain> {
    msgs.iter()
        .map(|m| MessageToAppchain {
            from_address: CainomeContractAddress(m.from_address),
            to_address: CainomeContractAddress(m.to_address.into()),
            nonce: m.nonce,
            selector: m.entry_point_selector,
            payload: m.payload.clone(),
        })
        .collect()
}
