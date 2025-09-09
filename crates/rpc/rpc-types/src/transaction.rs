use katana_primitives::class::{ClassHash, CompiledClassHash};
use katana_primitives::contract::Nonce;
use katana_primitives::da::DataAvailabilityMode;
use katana_primitives::execution::EntryPointSelector;
use katana_primitives::fee::Tip;
use katana_primitives::transaction::TxHash;
use katana_primitives::{transaction as primitives, ContractAddress, Felt};
use serde::{Deserialize, Serialize};
use starknet::core::types::ResourceBoundsMapping;

use crate::ExecutionResult;

/// Represents the status of a transaction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "finality_status")]
pub enum TxStatus {
    /// Transaction received by sequencer and awaiting processing.
    #[serde(rename = "RECEIVED")]
    Received,

    /// Transaction is scheduled to be executed by sequencer.
    #[serde(rename = "CANDIDATE")]
    Candidate,

    /// Transaction pre-confirmed by sequencer but is not guaranteed to be included in a block.
    #[serde(rename = "PRE_CONFIRMED")]
    PreConfirmed(ExecutionResult),

    /// Transaction accepted on Layer 2 with a specific execution status.
    #[serde(rename = "ACCEPTED_ON_L2")]
    AcceptedOnL2(ExecutionResult),

    /// Transaction accepted on Layer 1 with a specific execution status.
    #[serde(rename = "ACCEPTED_ON_L1")]
    AcceptedOnL1(ExecutionResult),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RpcTxWithHash {
    /// The hash of the transaction.
    pub transaction_hash: TxHash,
    #[serde(flatten)]
    pub transaction: RpcTx,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum RpcTx {
    /// An `INVOKE` transaction.
    #[serde(rename = "INVOKE")]
    Invoke(RpcInvokeTx),

    /// An `L1_HANDLER` transaction.
    #[serde(rename = "L1_HANDLER")]
    L1Handler(RpcL1HandlerTx),

    /// A `DECLARE` transaction.
    #[serde(rename = "DECLARE")]
    Declare(RpcDeclareTx),

    /// A `DEPLOY` transaction.
    #[serde(rename = "DEPLOY")]
    Deploy(RpcDeployTx),

    /// A `DEPLOY_ACCOUNT` transaction.
    #[serde(rename = "DEPLOY_ACCOUNT")]
    DeployAccount(RpcDeployAccountTx),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "version")]
pub enum RpcInvokeTx {
    #[serde(rename = "0x0")]
    V0(RpcInvokeTxV0),

    #[serde(rename = "0x1")]
    V1(RpcInvokeTxV1),

    #[serde(rename = "0x3")]
    V3(RpcInvokeTxV3),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RpcInvokeTxV0 {
    /// The maximal fee that can be charged for including the transaction
    pub max_fee: Felt,
    /// Signature
    pub signature: Vec<Felt>,
    /// Contract address
    pub contract_address: ContractAddress,
    /// Entry point selector
    pub entry_point_selector: Felt,
    /// The parameters passed to the function
    pub calldata: Vec<Felt>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RpcInvokeTxV1 {
    /// Sender address
    pub sender_address: ContractAddress,
    /// The data expected by the account's `execute` function (in most usecases, this includes the
    /// called contract address and a function selector)
    pub calldata: Vec<Felt>,
    /// The maximal fee that can be charged for including the transaction
    pub max_fee: Felt,
    /// Signature
    pub signature: Vec<Felt>,
    /// Nonce
    pub nonce: Nonce,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RpcInvokeTxV3 {
    /// Sender address
    pub sender_address: ContractAddress,
    /// The data expected by the account's `execute` function (in most usecases, this includes the
    /// called contract address and a function selector)
    pub calldata: Vec<Felt>,
    /// Signature
    pub signature: Vec<Felt>,
    /// Nonce
    pub nonce: Nonce,
    /// Resource bounds for the transaction execution
    pub resource_bounds: ResourceBoundsMapping,
    /// The tip for the transaction
    pub tip: Tip,
    /// Data needed to allow the paymaster to pay for the transaction in native tokens
    pub paymaster_data: Vec<Felt>,
    /// Data needed to deploy the account contract from which this tx will be initiated
    pub account_deployment_data: Vec<Felt>,
    /// The storage domain of the account's nonce (an account has a nonce per da mode)
    pub nonce_data_availability_mode: DataAvailabilityMode,
    /// The storage domain of the account's balance from which fee will be charged
    pub fee_data_availability_mode: DataAvailabilityMode,
}

/// L1 handler transaction.
///
/// A call to an l1_handler on an L2 contract induced by a message from L1.

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RpcL1HandlerTx {
    /// Version of the transaction scheme
    pub version: Felt,
    /// The L1->L2 message nonce field of the sn core L1 contract at the time the transaction was
    /// sent
    pub nonce: Nonce,
    /// Contract address
    pub contract_address: ContractAddress,
    /// Entry point selector
    pub entry_point_selector: EntryPointSelector,
    /// The parameters passed to the function
    pub calldata: Vec<Felt>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "version")]
pub enum RpcDeclareTx {
    /// Version 0 `DECLARE` transaction.
    #[serde(rename = "0x0")]
    V0(RpcDeclareTxV0),

    /// Version 1 `DECLARE` transaction.
    #[serde(rename = "0x1")]
    V1(RpcDeclareTxV1),

    /// Version 2 `DECLARE` transaction.
    #[serde(rename = "0x2")]
    V2(RpcDeclareTxV2),

    /// Version 3 `DECLARE` transaction.
    #[serde(rename = "0x3")]
    V3(RpcDeclareTxV3),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RpcDeclareTxV0 {
    /// The address of the account contract sending the declaration transaction
    pub sender_address: ContractAddress,
    /// The maximal fee that can be charged for including the transaction
    pub max_fee: Felt,
    /// Signature
    pub signature: Vec<Felt>,
    /// The hash of the declared class
    pub class_hash: ClassHash,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RpcDeclareTxV1 {
    /// The address of the account contract sending the declaration transaction
    pub sender_address: ContractAddress,
    /// The maximal fee that can be charged for including the transaction
    pub max_fee: Felt,
    /// Signature
    pub signature: Vec<Felt>,
    /// Nonce
    pub nonce: Nonce,
    /// The hash of the declared class
    pub class_hash: ClassHash,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RpcDeclareTxV2 {
    /// The address of the account contract sending the declaration transaction
    pub sender_address: ContractAddress,
    /// The hash of the cairo assembly resulting from the sierra compilation
    pub compiled_class_hash: ClassHash,
    /// The maximal fee that can be charged for including the transaction
    pub max_fee: Felt,
    /// Signature
    pub signature: Vec<Felt>,
    /// Nonce
    pub nonce: Nonce,
    /// The hash of the declared class
    pub class_hash: ClassHash,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RpcDeclareTxV3 {
    /// The address of the account contract sending the declaration transaction
    pub sender_address: ContractAddress,
    /// The hash of the cairo assembly resulting from the sierra compilation
    pub compiled_class_hash: CompiledClassHash,
    /// Signature
    pub signature: Vec<Felt>,
    /// Nonce
    pub nonce: Nonce,
    /// The hash of the declared class
    pub class_hash: ClassHash,
    /// Resource bounds for the transaction execution
    pub resource_bounds: ResourceBoundsMapping,
    /// The tip for the transaction
    pub tip: Tip,
    /// Data needed to allow the paymaster to pay for the transaction in native tokens
    pub paymaster_data: Vec<Felt>,
    /// Data needed to deploy the account contract from which this tx will be initiated
    pub account_deployment_data: Vec<Felt>,
    /// The storage domain of the account's nonce (an account has a nonce per da mode)
    pub nonce_data_availability_mode: DataAvailabilityMode,
    /// The storage domain of the account's balance from which fee will be charged
    pub fee_data_availability_mode: DataAvailabilityMode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RpcDeployTx {
    /// Version of the transaction scheme
    pub version: Felt,
    /// The salt for the address of the deployed contract
    pub contract_address_salt: Felt,
    /// The parameters passed to the constructor
    pub constructor_calldata: Vec<Felt>,
    /// The hash of the deployed contract's class
    pub class_hash: ClassHash,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "version")]
pub enum RpcDeployAccountTx {
    /// Version 1 `DEPLOY_ACCOUNT` transaction.
    #[serde(rename = "0x1")]
    V1(RpcDeployAccountTxV1),

    /// Version 3 `DEPLOY_ACCOUNT` transaction.
    #[serde(rename = "0x3")]
    V3(RpcDeployAccountTxV3),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RpcDeployAccountTxV1 {
    /// The maximal fee that can be charged for including the transaction
    pub max_fee: Felt,
    /// Signature
    pub signature: Vec<Felt>,
    /// Nonce
    pub nonce: Nonce,
    /// The salt for the address of the deployed contract
    pub contract_address_salt: Felt,
    /// The parameters passed to the constructor
    pub constructor_calldata: Vec<Felt>,
    /// The hash of the deployed contract's class
    pub class_hash: ClassHash,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RpcDeployAccountTxV3 {
    /// Signature
    pub signature: Vec<Felt>,
    /// Nonce
    pub nonce: Nonce,
    /// The salt for the address of the deployed contract
    pub contract_address_salt: Felt,
    /// The parameters passed to the constructor
    pub constructor_calldata: Vec<Felt>,
    /// The hash of the deployed contract's class
    pub class_hash: ClassHash,
    /// Resource bounds for the transaction execution
    pub resource_bounds: ResourceBoundsMapping,
    /// The tip for the transaction
    pub tip: Tip,
    /// Data needed to allow the paymaster to pay for the transaction in native tokens
    pub paymaster_data: Vec<Felt>,
    /// The storage domain of the account's nonce (an account has a nonce per da mode)
    pub nonce_data_availability_mode: DataAvailabilityMode,
    /// The storage domain of the account's balance from which fee will be charged
    pub fee_data_availability_mode: DataAvailabilityMode,
}

impl From<primitives::TxWithHash> for RpcTxWithHash {
    fn from(value: primitives::TxWithHash) -> Self {
        let transaction_hash = value.hash;
        let transaction = RpcTx::from(value.transaction);
        RpcTxWithHash { transaction_hash, transaction }
    }
}

impl From<primitives::Tx> for RpcTx {
    fn from(value: primitives::Tx) -> Self {
        match value {
            primitives::Tx::Invoke(invoke) => match invoke {
                primitives::InvokeTx::V0(tx) => RpcTx::Invoke(RpcInvokeTx::V0(RpcInvokeTxV0 {
                    calldata: tx.calldata,
                    signature: tx.signature,
                    max_fee: tx.max_fee.into(),
                    contract_address: tx.contract_address,
                    entry_point_selector: tx.entry_point_selector,
                })),

                primitives::InvokeTx::V1(tx) => RpcTx::Invoke(RpcInvokeTx::V1(RpcInvokeTxV1 {
                    nonce: tx.nonce,
                    calldata: tx.calldata,
                    signature: tx.signature,
                    max_fee: tx.max_fee.into(),
                    sender_address: tx.sender_address,
                })),

                primitives::InvokeTx::V3(tx) => RpcTx::Invoke(RpcInvokeTx::V3(RpcInvokeTxV3 {
                    nonce: tx.nonce,
                    calldata: tx.calldata,
                    signature: tx.signature,
                    sender_address: tx.sender_address,
                    account_deployment_data: tx.account_deployment_data,
                    fee_data_availability_mode: tx.fee_data_availability_mode,
                    nonce_data_availability_mode: tx.nonce_data_availability_mode,
                    paymaster_data: tx.paymaster_data,
                    resource_bounds: to_rpc_resource_bounds(tx.resource_bounds),
                    tip: tx.tip.into(),
                })),
            },

            primitives::Tx::Declare(tx) => RpcTx::Declare(match tx {
                primitives::DeclareTx::V0(tx) => RpcDeclareTx::V0(RpcDeclareTxV0 {
                    signature: tx.signature,
                    class_hash: tx.class_hash,
                    max_fee: tx.max_fee.into(),
                    sender_address: tx.sender_address,
                }),

                primitives::DeclareTx::V1(tx) => RpcDeclareTx::V1(RpcDeclareTxV1 {
                    nonce: tx.nonce,
                    signature: tx.signature,
                    class_hash: tx.class_hash,
                    max_fee: tx.max_fee.into(),
                    sender_address: tx.sender_address,
                }),

                primitives::DeclareTx::V2(tx) => RpcDeclareTx::V2(RpcDeclareTxV2 {
                    nonce: tx.nonce,
                    signature: tx.signature,
                    class_hash: tx.class_hash,
                    max_fee: tx.max_fee.into(),
                    sender_address: tx.sender_address,
                    compiled_class_hash: tx.compiled_class_hash,
                }),

                primitives::DeclareTx::V3(tx) => RpcDeclareTx::V3(RpcDeclareTxV3 {
                    nonce: tx.nonce,
                    signature: tx.signature,
                    class_hash: tx.class_hash,
                    sender_address: tx.sender_address,
                    compiled_class_hash: tx.compiled_class_hash,
                    account_deployment_data: tx.account_deployment_data,
                    fee_data_availability_mode: tx.fee_data_availability_mode,
                    nonce_data_availability_mode: tx.nonce_data_availability_mode,
                    paymaster_data: tx.paymaster_data,
                    resource_bounds: to_rpc_resource_bounds(tx.resource_bounds),
                    tip: tx.tip.into(),
                }),
            }),

            primitives::Tx::L1Handler(tx) => RpcTx::L1Handler(RpcL1HandlerTx {
                calldata: tx.calldata,
                contract_address: tx.contract_address,
                entry_point_selector: tx.entry_point_selector,
                nonce: tx.nonce,
                version: tx.version,
            }),

            primitives::Tx::DeployAccount(tx) => RpcTx::DeployAccount(match tx {
                primitives::DeployAccountTx::V1(tx) => {
                    RpcDeployAccountTx::V1(RpcDeployAccountTxV1 {
                        nonce: tx.nonce,
                        signature: tx.signature,
                        class_hash: tx.class_hash,
                        max_fee: tx.max_fee.into(),
                        constructor_calldata: tx.constructor_calldata,
                        contract_address_salt: tx.contract_address_salt,
                    })
                }

                primitives::DeployAccountTx::V3(tx) => {
                    RpcDeployAccountTx::V3(RpcDeployAccountTxV3 {
                        nonce: tx.nonce,
                        signature: tx.signature,
                        class_hash: tx.class_hash,
                        constructor_calldata: tx.constructor_calldata,
                        contract_address_salt: tx.contract_address_salt,
                        fee_data_availability_mode: tx.fee_data_availability_mode,
                        nonce_data_availability_mode: tx.nonce_data_availability_mode,
                        paymaster_data: tx.paymaster_data,
                        resource_bounds: to_rpc_resource_bounds(tx.resource_bounds),
                        tip: tx.tip.into(),
                    })
                }
            }),

            primitives::Tx::Deploy(tx) => RpcTx::Deploy(RpcDeployTx {
                constructor_calldata: tx.constructor_calldata,
                contract_address_salt: tx.contract_address_salt,
                class_hash: tx.class_hash,
                version: tx.version,
            }),
        }
    }
}

fn to_rpc_resource_bounds(
    bounds: katana_primitives::fee::ResourceBoundsMapping,
) -> starknet::core::types::ResourceBoundsMapping {
    match bounds {
        katana_primitives::fee::ResourceBoundsMapping::All(all_bounds) => {
            starknet::core::types::ResourceBoundsMapping {
                l1_gas: starknet::core::types::ResourceBounds {
                    max_amount: all_bounds.l1_gas.max_amount,
                    max_price_per_unit: all_bounds.l1_gas.max_price_per_unit,
                },
                l2_gas: starknet::core::types::ResourceBounds {
                    max_amount: all_bounds.l2_gas.max_amount,
                    max_price_per_unit: all_bounds.l2_gas.max_price_per_unit,
                },
                l1_data_gas: starknet::core::types::ResourceBounds {
                    max_amount: all_bounds.l1_data_gas.max_amount,
                    max_price_per_unit: all_bounds.l1_data_gas.max_price_per_unit,
                },
            }
        }
        // The `l1_data_gas` bounds should actually be ommitted but because `starknet-rs` doesn't
        // support older RPC spec, we default to zero. This aren't technically accurate so should
        // find an alternative or completely remove legacy support. But we need to support in order
        // to maintain backward compatibility from older database version.
        katana_primitives::fee::ResourceBoundsMapping::L1Gas(l1_gas_bounds) => {
            starknet::core::types::ResourceBoundsMapping {
                l1_gas: starknet::core::types::ResourceBounds {
                    max_amount: l1_gas_bounds.max_amount,
                    max_price_per_unit: l1_gas_bounds.max_price_per_unit,
                },
                l2_gas: starknet::core::types::ResourceBounds {
                    max_amount: 0,
                    max_price_per_unit: 0,
                },
                l1_data_gas: starknet::core::types::ResourceBounds {
                    max_amount: 0,
                    max_price_per_unit: 0,
                },
            }
        }
    }
}
