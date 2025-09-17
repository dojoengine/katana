use katana_primitives::contract::Nonce;
use katana_primitives::execution::{EntryPointSelector, VmResources};
use katana_primitives::receipt::{DataAvailabilityResources, Event, GasUsed, MessageToL1};
use katana_primitives::transaction::TxHash;
use katana_primitives::{eth, ContractAddress, Felt};
use serde::{Deserialize, Serialize};

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfirmedReceipt {
    /// The hash of the transaction the receipt belongs to.
    pub transaction_hash: TxHash,
    /// The index of the transaction in the block.
    pub transaction_index: u64,
    /// The status of the transaction execution.
    pub execution_status: Option<ExecutionStatus>,
    /// The error message if the transaction was reverted.
    ///
    /// This field should only be present if the transaction was reverted ie the `execution_status`
    /// field is `REVERTED`.
    pub revert_error: Option<String>,
    pub execution_resources: Option<ExecutionResources>,
    pub l1_to_l2_consumed_message: Option<L1ToL2Message>,
    pub l2_to_l1_messages: Vec<MessageToL1>,
    pub events: Vec<Event>,
    pub actual_fee: Felt,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExecutionStatus {
    #[serde(rename = "SUCCEEDED")]
    Succeeded,

    #[serde(rename = "REVERTED")]
    Reverted,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionResources {
    #[serde(flatten)]
    pub vm_resources: VmResources,
    pub data_availability: Option<DataAvailabilityResources>,
    pub total_gas_consumed: Option<GasUsed>,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct L1ToL2Message {
    /// The address of the Ethereum (L1) contract that sent the message.
    pub from_address: eth::Address,
    pub to_address: ContractAddress,
    pub selector: EntryPointSelector,
    pub payload: Vec<Felt>,
    pub nonce: Option<Nonce>,
}
