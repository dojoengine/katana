use crate::contract::{ContractAddress, Nonce};
use crate::eth::Address as EthAddress;
use crate::Felt;

/// Message from L1.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "arbitrary", derive(::arbitrary::Arbitrary))]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct L1ToL2Message {
    /// The address of the L1 contract sending the message
    pub from_address: EthAddress,
    /// The target L2 address the message is sent to
    pub to_address: ContractAddress,
    /// The selector of the l1_handler in invoke in the target contract
    pub entry_point_selector: Felt,
    /// The payload of the message
    pub payload: Vec<Felt>,
    /// The nonce of the message
    pub nonce: Nonce,
}
