use katana_primitives::chain::ChainId;
use katana_primitives::eth::Address as EthAddress;
use katana_primitives::execution::EntryPointSelector;
use katana_primitives::transaction::{L1HandlerTx, TxHash};
use katana_primitives::utils::transaction::compute_l1_to_l2_message_hash;
use katana_primitives::{ContractAddress, Felt};
use serde::{Deserialize, Serialize};

use crate::ExecutionResult;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct L1ToL2Message {
    /// The address of the L1 contract sending the message
    pub from_address: EthAddress,
    /// The target L2 address the message is sent to
    pub to_address: ContractAddress,
    /// The selector of the l1_handler in invoke in the target contract
    pub entry_point_selector: EntryPointSelector,
    /// The payload of the message
    pub payload: Vec<Felt>,

    pub nonce: Felt,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct L2ToL1Message {
    pub from_address: ContractAddress,
    pub to_address: Felt,
    pub payload: Vec<Felt>,
}

/// Finality status of an L1->L2 message.
///
/// Mirrors `TXN_FINALITY_STATUS` from the Starknet JSON-RPC spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageFinalityStatus {
    #[serde(rename = "PRE_CONFIRMED")]
    PreConfirmed,
    #[serde(rename = "ACCEPTED_ON_L2")]
    AcceptedOnL2,
    #[serde(rename = "ACCEPTED_ON_L1")]
    AcceptedOnL1,
}

/// Status of a single L1->L2 message returned by `starknet_getMessagesStatus`.
///
/// Mirrors the spec's `MESSAGE_STATUS` shape: the L2 L1Handler transaction hash, its
/// finality status, and the inline execution status / failure reason.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageStatus {
    /// L2 L1Handler transaction hash spawned by the L1 message.
    pub transaction_hash: TxHash,
    /// Finality of the message handler on L2.
    pub finality_status: MessageFinalityStatus,
    /// Execution outcome of the handler. Flattened: serializes as `execution_status`
    /// (+ `revert_reason` when reverted).
    #[serde(flatten)]
    pub execution_result: ExecutionResult,
}

/// Message from L1.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MsgFromL1 {
    /// The address of the L1 contract sending the message
    pub from_address: EthAddress,
    /// The target L2 address the message is sent to
    pub to_address: ContractAddress,
    /// The selector of the l1_handler in invoke in the target contract
    pub entry_point_selector: EntryPointSelector,
    /// The payload of the message
    pub payload: Vec<Felt>,
}

impl MsgFromL1 {
    pub fn into_tx_with_chain_id(self, chain_id: ChainId) -> L1HandlerTx {
        // Set the L1 to L2 message nonce to 0, because this is just used
        // for the `starknet_estimateMessageFee` RPC.
        let nonce = Felt::ZERO;

        let message_hash = compute_l1_to_l2_message_hash(
            self.from_address,
            self.to_address,
            self.entry_point_selector,
            &self.payload,
            nonce,
        );

        // When executing a l1 handler tx, blockifier just assert that the paid_fee_on_l1 is
        // anything but 0. See: https://github.com/dojoengine/sequencer/blob/d6951f24fc2082c7aa89cdbc063648915b131d74/crates/blockifier/src/transaction/transaction_execution.rs#L140-L145
        //
        // For fee estimation, this value is basically irrelevant.
        let paid_fee_on_l1 = 1u128;

        // In an l1_handler transaction, the first element of the calldata is always the Ethereum
        // address of the sender (msg.sender). https://docs.starknet.io/documentation/architecture_and_concepts/Network_Architecture/messaging-mechanism/#l1-l2-messages
        let from_address = Felt::from_bytes_be_slice(&self.from_address.into_array());
        let mut calldata = vec![from_address];
        calldata.extend(self.payload);

        L1HandlerTx {
            nonce,
            chain_id,
            calldata,
            message_hash,
            paid_fee_on_l1,
            version: Felt::ZERO,
            contract_address: self.to_address,
            entry_point_selector: self.entry_point_selector,
        }
    }
}
