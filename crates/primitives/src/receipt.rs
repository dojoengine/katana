use std::iter;

use alloy_primitives::B256;
use derive_more::{AsRef, Deref};
use starknet::core::utils::starknet_keccak;
use starknet_types_core::hash::{self, StarkHash};

use crate::contract::ContractAddress;
use crate::execution::VmResources;
use crate::fee::FeeInfo;
use crate::transaction::{TxHash, TxType};
use crate::Felt;

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Event {
    /// The contract address that emitted the event.
    pub from_address: ContractAddress,
    /// The event keys.
    pub keys: Vec<Felt>,
    /// The event data.
    pub data: Vec<Felt>,
}

/// Represents a message sent to L1.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct MessageToL1 {
    /// The L2 contract address that sent the message.
    pub from_address: ContractAddress,
    /// The L1 contract address that the message is sent to.
    pub to_address: Felt,
    /// The payload of the message.
    pub payload: Vec<Felt>,
}

/// Receipt for a `Invoke` transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct InvokeTxReceipt {
    /// Information about the transaction fee.
    pub fee: FeeInfo,
    /// Events emitted by contracts.
    pub events: Vec<Event>,
    /// Messages sent to L1.
    pub messages_sent: Vec<MessageToL1>,
    /// Revert error message if the transaction execution failed.
    pub revert_error: Option<String>,
    /// The execution resources used by the transaction.
    pub execution_resources: ExecutionResources,
}

/// Receipt for a `Declare` transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DeclareTxReceipt {
    /// Information about the transaction fee.
    pub fee: FeeInfo,
    /// Events emitted by contracts.
    pub events: Vec<Event>,
    /// Messages sent to L1.
    pub messages_sent: Vec<MessageToL1>,
    /// Revert error message if the transaction execution failed.
    pub revert_error: Option<String>,
    /// The execution resources used by the transaction.
    pub execution_resources: ExecutionResources,
}

/// Receipt for a `L1Handler` transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct L1HandlerTxReceipt {
    /// Information about the transaction fee.
    pub fee: FeeInfo,
    /// Events emitted by contracts.
    pub events: Vec<Event>,
    /// The hash of the L1 message
    pub message_hash: B256,
    /// Messages sent to L1.
    pub messages_sent: Vec<MessageToL1>,
    /// Revert error message if the transaction execution failed.
    pub revert_error: Option<String>,
    /// The execution resources used by the transaction.
    pub execution_resources: ExecutionResources,
}

/// Receipt for a `DeployAccount` transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DeployAccountTxReceipt {
    /// Information about the transaction fee.
    pub fee: FeeInfo,
    /// Events emitted by contracts.
    pub events: Vec<Event>,
    /// Messages sent to L1.
    pub messages_sent: Vec<MessageToL1>,
    /// Revert error message if the transaction execution failed.
    pub revert_error: Option<String>,
    /// The execution resources used by the transaction.
    pub execution_resources: ExecutionResources,
    /// Contract address of the deployed account contract.
    pub contract_address: ContractAddress,
}

/// The receipt of a transaction containing the outputs of its execution.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Receipt {
    Invoke(InvokeTxReceipt),
    Declare(DeclareTxReceipt),
    L1Handler(L1HandlerTxReceipt),
    DeployAccount(DeployAccountTxReceipt),
}

impl Receipt {
    /// Returns `true` if the transaction is reverted.
    ///
    /// A transaction is reverted if the `revert_error` field in the receipt is not `None`.
    pub fn is_reverted(&self) -> bool {
        self.revert_reason().is_some()
    }

    /// Returns the revert reason if the transaction is reverted.
    pub fn revert_reason(&self) -> Option<&str> {
        match self {
            Receipt::Invoke(rct) => rct.revert_error.as_deref(),
            Receipt::Declare(rct) => rct.revert_error.as_deref(),
            Receipt::L1Handler(rct) => rct.revert_error.as_deref(),
            Receipt::DeployAccount(rct) => rct.revert_error.as_deref(),
        }
    }

    /// Returns the L1 messages sent.
    pub fn messages_sent(&self) -> &[MessageToL1] {
        match self {
            Receipt::Invoke(rct) => &rct.messages_sent,
            Receipt::Declare(rct) => &rct.messages_sent,
            Receipt::L1Handler(rct) => &rct.messages_sent,
            Receipt::DeployAccount(rct) => &rct.messages_sent,
        }
    }

    /// Returns the events emitted.
    pub fn events(&self) -> &[Event] {
        match self {
            Receipt::Invoke(rct) => &rct.events,
            Receipt::Declare(rct) => &rct.events,
            Receipt::L1Handler(rct) => &rct.events,
            Receipt::DeployAccount(rct) => &rct.events,
        }
    }

    /// Returns the execution resources used.
    pub fn resources_used(&self) -> &ExecutionResources {
        match self {
            Receipt::Invoke(rct) => &rct.execution_resources,
            Receipt::Declare(rct) => &rct.execution_resources,
            Receipt::L1Handler(rct) => &rct.execution_resources,
            Receipt::DeployAccount(rct) => &rct.execution_resources,
        }
    }

    pub fn fee(&self) -> &FeeInfo {
        match self {
            Receipt::Invoke(rct) => &rct.fee,
            Receipt::Declare(rct) => &rct.fee,
            Receipt::L1Handler(rct) => &rct.fee,
            Receipt::DeployAccount(rct) => &rct.fee,
        }
    }

    /// Returns the transaction tyoe of the receipt.
    pub fn r#type(&self) -> TxType {
        match self {
            Receipt::Invoke(_) => TxType::Invoke,
            Receipt::Declare(_) => TxType::Declare,
            Receipt::L1Handler(_) => TxType::L1Handler,
            Receipt::DeployAccount(_) => TxType::DeployAccount,
        }
    }
}

#[derive(Debug, Clone, AsRef, Deref, PartialEq, Eq)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ReceiptWithTxHash {
    /// The hash of the transaction.
    pub tx_hash: TxHash,
    /// The raw transaction.
    #[deref]
    #[as_ref]
    pub receipt: Receipt,
}

impl ReceiptWithTxHash {
    pub fn new(hash: TxHash, receipt: Receipt) -> Self {
        Self { tx_hash: hash, receipt }
    }

    /// Computes the hash of the receipt. This is used for computing the receipts commitment.
    ///
    /// The hash of a transaction receipt is defined as:
    ///
    /// ```
    /// h(
    ///     transaction_hash,
    ///     actual_fee,
    ///     h(messages),
    ///     sn_keccak(revert_reason),
    ///     h(l2_gas_consumed, l1_gas_consumed, l1_data_gas_consumed),
    /// )
    /// ```
    ///
    /// See the Starknet [docs] for reference.
    ///
    /// [docs]: https://docs.starknet.io/architecture-and-concepts/network-architecture/block-structure/#receipt_hash
    //
    pub fn compute_hash(&self) -> Felt {
        let messages_hash = self.compute_messages_to_l1_hash();

        let revert_reason_hash = if let Some(reason) = self.revert_reason() {
            starknet_keccak(reason.as_bytes())
        } else {
            Felt::ZERO
        };

        match self.resources_used().gas {
            GasUsed::All { l2_gas, l1_gas, l1_data_gas } => {
                let gas_usage =
                    hash::Poseidon::hash_array(&[l2_gas.into(), l1_gas.into(), l1_data_gas.into()]);

                hash::Poseidon::hash_array(&[
                    self.tx_hash,
                    self.receipt.fee().overall_fee.into(),
                    messages_hash,
                    revert_reason_hash,
                    gas_usage,
                ])
            }

            GasUsed::L1Gas { l1_gas } => hash::Poseidon::hash_array(&[
                self.tx_hash,
                self.receipt.fee().overall_fee.into(),
                messages_hash,
                revert_reason_hash,
                Felt::ZERO, // L2 gas consumption.
                l1_gas.into(),
            ]),
        }
    }

    // H(n, from, to, H(payload), ...), where n, is the total number of messages, the payload is
    // prefixed by its length, and h is the Poseidon hash function.
    fn compute_messages_to_l1_hash(&self) -> Felt {
        let messages = self.messages_sent();
        let messages_len = messages.len();

        // Allocate all the memory in advance; times 3 because [ from, to, h(payload) ]
        let mut accumulator: Vec<Felt> = Vec::with_capacity((messages_len * 3) + 1);
        accumulator.push(Felt::from(messages_len));

        let elements = messages.iter().fold(accumulator, |mut acc, msg| {
            // Compute the payload hash; h(n, payload_1, ..., payload_n)
            let len = Felt::from(msg.payload.len());
            let payload = iter::once(len).chain(msg.payload.clone()).collect::<Vec<Felt>>();
            let payload_hash = hash::Poseidon::hash_array(&payload);

            acc.push(msg.from_address.into());
            acc.push(msg.to_address);
            acc.push(payload_hash);

            acc
        });

        hash::Poseidon::hash_array(&elements)
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ExecutionResources {
    /// The total gas used by the transaction execution.
    pub gas: GasUsed,
    /// Computation resources if the transaction is executed on the CairoVM.
    pub computation_resources: VmResources,
    pub da_resources: DataAvailabilityResources,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum GasUsed {
    All { l2_gas: u64, l1_gas: u64, l1_data_gas: u64 },
    L1Gas { l1_gas: u64 },
}

impl Default for GasUsed {
    fn default() -> Self {
        GasUsed::L1Gas { l1_gas: 0 }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DataAvailabilityResources {
    pub l1_gas: u64,
    pub l1_data_gas: u64,
}

#[cfg(feature = "arbitrary")]
impl<'a> arbitrary::Arbitrary<'a> for ExecutionResources {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        use std::collections::HashMap;

        use crate::execution::BuiltinName;

        let n_steps = u.arbitrary()?;
        let n_memory_holes = u.arbitrary()?;

        let mut builtin_instance_counter = HashMap::new();
        let num_builtins = u.int_in_range(0..=12)?; // There are 12 only builtin types

        for _ in 0..num_builtins {
            let builtin = u.arbitrary::<BuiltinName>()?;
            let count = u.arbitrary::<usize>()?;
            builtin_instance_counter.insert(builtin, count);
        }

        let computation_resources =
            VmResources { n_steps, n_memory_holes, builtin_instance_counter };

        let gas = u.arbitrary::<GasUsed>()?;
        let da_resources = u.arbitrary::<DataAvailabilityResources>()?;

        Ok(Self { da_resources, computation_resources, gas })
    }
}
