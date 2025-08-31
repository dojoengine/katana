use katana_primitives::class::{ClassHash, CompiledClassHash};
use katana_primitives::contract::Nonce;
use katana_primitives::execution::EntryPointSelector;
use katana_primitives::transaction::{TxHash, TxWithHash};
use katana_primitives::{ContractAddress, Felt};
use serde::{Deserialize, Serialize};
use starknet::core::types::{DataAvailabilityMode, ResourceBoundsMapping};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Tx {
    /// An `INVOKE` transaction.
    Invoke(InvokeTx),
    /// An `L1_HANDLER` transaction.
    L1Handler(L1HandlerTx),
    /// A `DECLARE` transaction.
    Declare(DeclareTx),
    /// A `DEPLOY` transaction.
    Deploy(DeployTx),
    /// A `DEPLOY_ACCOUNT` transaction.
    DeployAccount(DeployAccountTx),
}

/// An `INVOKE` Starknet transaction.

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "version")]
pub enum InvokeTx {
    #[serde(rename = "0x0")]
    V0(InvokeTxV0),
    #[serde(rename = "0x1")]
    V1(InvokeTxV1),
    #[serde(rename = "0x3")]
    V3(InvokeTxV3),
}

/// Invoke transaction v0.
///
/// Invokes a specific function in the desired contract (not necessarily an account).

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvokeTxV0 {
    /// Transaction hash
    pub transaction_hash: TxHash,
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

/// Invoke transaction v1.
///
/// Initiates a transaction from a given account.

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvokeTxV1 {
    /// Transaction hash
    pub transaction_hash: TxHash,
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

/// Invoke transaction v3.
///
/// Initiates a transaction from a given account.

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvokeTxV3 {
    /// Transaction hash
    pub transaction_hash: TxHash,
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
    pub tip: u64,
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
pub struct L1HandlerTx {
    /// Transaction hash
    pub transaction_hash: TxHash,
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

/// A `DECLARE` Starknet transaction.

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "version")]
pub enum DeclareTx {
    /// Version 0 `DECLARE` transaction.
    #[serde(rename = "0x0")]
    V0(DeclareTxV0),
    /// Version 1 `DECLARE` transaction.
    #[serde(rename = "0x1")]
    V1(DeclareTxV1),
    /// Version 2 `DECLARE` transaction.
    #[serde(rename = "0x2")]
    V2(DeclareTxV2),
    /// Version 3 `DECLARE` transaction.
    #[serde(rename = "0x3")]
    V3(DeclareTxV3),
}

/// Declare contract transaction v0.
///
/// Declare contract transaction v0.

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeclareTxV0 {
    /// Transaction hash
    pub transaction_hash: TxHash,
    /// The address of the account contract sending the declaration transaction
    pub sender_address: ContractAddress,
    /// The maximal fee that can be charged for including the transaction
    pub max_fee: Felt,
    /// Signature
    pub signature: Vec<Felt>,
    /// The hash of the declared class
    pub class_hash: ClassHash,
}

/// Declare contract transaction v1.
///
/// Declare contract transaction v1.

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeclareTxV1 {
    /// Transaction hash
    pub transaction_hash: TxHash,
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

/// Declare transaction v2.
///
/// Declare contract transaction v2.

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeclareTxV2 {
    /// Transaction hash
    pub transaction_hash: TxHash,
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
pub struct DeclareTxV3 {
    /// Transaction hash
    pub transaction_hash: TxHash,
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
    pub tip: u64,
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
pub struct DeployTx {
    /// Transaction hash
    pub transaction_hash: TxHash,
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
pub enum DeployAccountTx {
    /// Version 1 `DEPLOY_ACCOUNT` transaction.
    #[serde(rename = "0x1")]
    V1(DeployAccountTxV1),
    /// Version 3 `DEPLOY_ACCOUNT` transaction.
    #[serde(rename = "0x3")]
    V3(DeployAccountTxV3),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeployAccountTxV1 {
    /// Transaction hash
    pub transaction_hash: TxHash,
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
pub struct DeployAccountTxV3 {
    /// Transaction hash
    pub transaction_hash: TxHash,
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
    pub tip: u64,
    /// Data needed to allow the paymaster to pay for the transaction in native tokens
    pub paymaster_data: Vec<Felt>,
    /// The storage domain of the account's nonce (an account has a nonce per da mode)
    pub nonce_data_availability_mode: DataAvailabilityMode,
    /// The storage domain of the account's balance from which fee will be charged
    pub fee_data_availability_mode: DataAvailabilityMode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TxContent(pub starknet::core::types::TransactionContent);

impl From<TxWithHash> for Tx {
    fn from(value: TxWithHash) -> Self {
        use katana_primitives::transaction as primitives;

        let transaction_hash = value.hash;
        match value.transaction {
            primitives::Tx::Invoke(invoke) => match invoke {
                primitives::InvokeTx::V0(tx) => Tx::Invoke(InvokeTx::V0(InvokeTxV0 {
                    transaction_hash,
                    calldata: tx.calldata,
                    signature: tx.signature,
                    max_fee: tx.max_fee.into(),
                    contract_address: tx.contract_address.into(),
                    entry_point_selector: tx.entry_point_selector,
                })),

                primitives::InvokeTx::V1(tx) => Tx::Invoke(InvokeTx::V1(InvokeTxV1 {
                    nonce: tx.nonce,
                    transaction_hash,
                    calldata: tx.calldata,
                    signature: tx.signature,
                    max_fee: tx.max_fee.into(),
                    sender_address: tx.sender_address.into(),
                })),

                primitives::InvokeTx::V3(tx) => Tx::Invoke(InvokeTx::V3(InvokeTxV3 {
                    nonce: tx.nonce,
                    transaction_hash,
                    calldata: tx.calldata,
                    signature: tx.signature,
                    sender_address: tx.sender_address.into(),
                    account_deployment_data: tx.account_deployment_data,
                    fee_data_availability_mode: to_rpc_da_mode(tx.fee_data_availability_mode),
                    nonce_data_availability_mode: to_rpc_da_mode(tx.nonce_data_availability_mode),
                    paymaster_data: tx.paymaster_data,
                    resource_bounds: to_rpc_resource_bounds(tx.resource_bounds),
                    tip: tx.tip,
                })),
            },

            primitives::Tx::Declare(tx) => Tx::Declare(match tx {
                primitives::DeclareTx::V0(tx) => DeclareTx::V0(DeclareTxV0 {
                    transaction_hash,
                    signature: tx.signature,
                    class_hash: tx.class_hash,
                    max_fee: tx.max_fee.into(),
                    sender_address: tx.sender_address.into(),
                }),

                primitives::DeclareTx::V1(tx) => DeclareTx::V1(DeclareTxV1 {
                    nonce: tx.nonce,
                    transaction_hash,
                    signature: tx.signature,
                    class_hash: tx.class_hash,
                    max_fee: tx.max_fee.into(),
                    sender_address: tx.sender_address.into(),
                }),

                primitives::DeclareTx::V2(tx) => DeclareTx::V2(DeclareTxV2 {
                    nonce: tx.nonce,
                    transaction_hash,
                    signature: tx.signature,
                    class_hash: tx.class_hash,
                    max_fee: tx.max_fee.into(),
                    sender_address: tx.sender_address.into(),
                    compiled_class_hash: tx.compiled_class_hash,
                }),

                primitives::DeclareTx::V3(tx) => DeclareTx::V3(DeclareTxV3 {
                    nonce: tx.nonce,
                    transaction_hash,
                    signature: tx.signature,
                    class_hash: tx.class_hash,
                    sender_address: tx.sender_address.into(),
                    compiled_class_hash: tx.compiled_class_hash,
                    account_deployment_data: tx.account_deployment_data,
                    fee_data_availability_mode: to_rpc_da_mode(tx.fee_data_availability_mode),
                    nonce_data_availability_mode: to_rpc_da_mode(tx.nonce_data_availability_mode),
                    paymaster_data: tx.paymaster_data,
                    resource_bounds: to_rpc_resource_bounds(tx.resource_bounds),
                    tip: tx.tip,
                }),
            }),

            primitives::Tx::L1Handler(tx) => Tx::L1Handler(L1HandlerTx {
                transaction_hash,
                calldata: tx.calldata,
                contract_address: tx.contract_address.into(),
                entry_point_selector: tx.entry_point_selector,
                nonce: tx.nonce,
                version: tx.version,
            }),

            primitives::Tx::DeployAccount(tx) => Tx::DeployAccount(match tx {
                primitives::DeployAccountTx::V1(tx) => DeployAccountTx::V1(DeployAccountTxV1 {
                    transaction_hash,
                    nonce: tx.nonce,
                    signature: tx.signature,
                    class_hash: tx.class_hash,
                    max_fee: tx.max_fee.into(),
                    constructor_calldata: tx.constructor_calldata,
                    contract_address_salt: tx.contract_address_salt,
                }),

                primitives::DeployAccountTx::V3(tx) => DeployAccountTx::V3(DeployAccountTxV3 {
                    transaction_hash,
                    nonce: tx.nonce,
                    signature: tx.signature,
                    class_hash: tx.class_hash,
                    constructor_calldata: tx.constructor_calldata,
                    contract_address_salt: tx.contract_address_salt,
                    fee_data_availability_mode: to_rpc_da_mode(tx.fee_data_availability_mode),
                    nonce_data_availability_mode: to_rpc_da_mode(tx.nonce_data_availability_mode),
                    paymaster_data: tx.paymaster_data,
                    resource_bounds: to_rpc_resource_bounds(tx.resource_bounds),
                    tip: tx.tip,
                }),
            }),

            primitives::Tx::Deploy(tx) => Tx::Deploy(DeployTx {
                constructor_calldata: tx.constructor_calldata,
                contract_address_salt: tx.contract_address_salt,
                class_hash: tx.class_hash,
                version: tx.version,
                transaction_hash,
            }),
        }
    }
}

// impl From<TxWithHash> for TxContent {
//     fn from(value: TxWithHash) -> Self {
//         use katana_primitives::transaction::Tx as InternalTx;

//         let tx = match value.transaction {
//             InternalTx::Invoke(invoke) => match invoke {
//                 InvokeTx::V0(tx) => TransactionContent::Invoke(InvokeTransactionContent::V0(
//                     InvokeTransactionV0Content {
//                         calldata: tx.calldata,
//                         signature: tx.signature,
//                         max_fee: tx.max_fee.into(),
//                         contract_address: tx.contract_address.into(),
//                         entry_point_selector: tx.entry_point_selector,
//                     },
//                 )),

//                 InvokeTx::V1(tx) => TransactionContent::Invoke(InvokeTransactionContent::V1(
//                     InvokeTransactionV1Content {
//                         nonce: tx.nonce,
//                         calldata: tx.calldata,
//                         signature: tx.signature,
//                         max_fee: tx.max_fee.into(),
//                         sender_address: tx.sender_address.into(),
//                     },
//                 )),

//                 InvokeTx::V3(tx) => TransactionContent::Invoke(InvokeTransactionContent::V3(
//                     InvokeTransactionV3Content {
//                         nonce: tx.nonce,
//                         calldata: tx.calldata,
//                         signature: tx.signature,
//                         sender_address: tx.sender_address.into(),
//                         account_deployment_data: tx.account_deployment_data,
//                         fee_data_availability_mode:
// to_rpc_da_mode(tx.fee_data_availability_mode),
// nonce_data_availability_mode: to_rpc_da_mode(
// tx.nonce_data_availability_mode,                         ),
//                         paymaster_data: tx.paymaster_data,
//                         resource_bounds: to_rpc_resource_bounds(tx.resource_bounds),
//                         tip: tx.tip,
//                     },
//                 )),
//             },

//             InternalTx::Declare(tx) => TransactionContent::Declare(match tx {
//                 DeclareTx::V0(tx) => DeclareTransactionContent::V0(DeclareTransactionV0Content {
//                     signature: tx.signature,
//                     class_hash: tx.class_hash,
//                     max_fee: tx.max_fee.into(),
//                     sender_address: tx.sender_address.into(),
//                 }),

//                 DeclareTx::V1(tx) => DeclareTransactionContent::V1(DeclareTransactionV1Content {
//                     nonce: tx.nonce,
//                     signature: tx.signature,
//                     class_hash: tx.class_hash,
//                     max_fee: tx.max_fee.into(),
//                     sender_address: tx.sender_address.into(),
//                 }),

//                 DeclareTx::V2(tx) => DeclareTransactionContent::V2(DeclareTransactionV2Content {
//                     nonce: tx.nonce,
//                     signature: tx.signature,
//                     class_hash: tx.class_hash,
//                     max_fee: tx.max_fee.into(),
//                     sender_address: tx.sender_address.into(),
//                     compiled_class_hash: tx.compiled_class_hash,
//                 }),

//                 DeclareTx::V3(tx) => DeclareTransactionContent::V3(DeclareTransactionV3Content {
//                     nonce: tx.nonce,
//                     signature: tx.signature,
//                     class_hash: tx.class_hash,
//                     sender_address: tx.sender_address.into(),
//                     compiled_class_hash: tx.compiled_class_hash,
//                     account_deployment_data: tx.account_deployment_data,
//                     fee_data_availability_mode: to_rpc_da_mode(tx.fee_data_availability_mode),
//                     nonce_data_availability_mode:
// to_rpc_da_mode(tx.nonce_data_availability_mode),                     paymaster_data:
// tx.paymaster_data,                     resource_bounds:
// to_rpc_resource_bounds(tx.resource_bounds),                     tip: tx.tip,
//                 }),
//             }),

//             InternalTx::L1Handler(tx) => {
//                 TransactionContent::L1Handler(L1HandlerTransactionContent {
//                     calldata: tx.calldata,
//                     contract_address: tx.contract_address.into(),
//                     entry_point_selector: tx.entry_point_selector,
//                     nonce: tx.nonce.to_u64().expect("nonce should fit in u64"),
//                     version: tx.version,
//                 })
//             }

//             InternalTx::DeployAccount(tx) => TransactionContent::DeployAccount(match tx {
//                 DeployAccountTx::V1(tx) => {
//                     DeployAccountTransactionContent::V1(DeployAccountTransactionV1Content {
//                         nonce: tx.nonce,
//                         signature: tx.signature,
//                         class_hash: tx.class_hash,
//                         max_fee: tx.max_fee.into(),
//                         constructor_calldata: tx.constructor_calldata,
//                         contract_address_salt: tx.contract_address_salt,
//                     })
//                 }

//                 DeployAccountTx::V3(tx) => {
//                     DeployAccountTransactionContent::V3(DeployAccountTransactionV3Content {
//                         nonce: tx.nonce,
//                         signature: tx.signature,
//                         class_hash: tx.class_hash,
//                         constructor_calldata: tx.constructor_calldata,
//                         contract_address_salt: tx.contract_address_salt,
//                         fee_data_availability_mode:
// to_rpc_da_mode(tx.fee_data_availability_mode),
// nonce_data_availability_mode: to_rpc_da_mode(
// tx.nonce_data_availability_mode,                         ),
//                         paymaster_data: tx.paymaster_data,
//                         resource_bounds: to_rpc_resource_bounds(tx.resource_bounds),
//                         tip: tx.tip,
//                     })
//                 }
//             }),

//             InternalTx::Deploy(tx) => TransactionContent::Deploy(DeployTransactionContent {
//                 constructor_calldata: tx.constructor_calldata,
//                 contract_address_salt: tx.contract_address_salt,
//                 class_hash: tx.class_hash,
//                 version: tx.version,
//             }),
//         };

//         TxContent(tx)
//     }
// }

// TODO: find a solution to avoid doing this conversion, this is not pretty at all. the reason why
// we had to do this in the first place is because of the orphan rule. i think eventually we should
// not rely on `starknet-rs` rpc types anymore and should instead define the types ourselves to have
// more flexibility.

fn to_rpc_da_mode(
    mode: katana_primitives::da::DataAvailabilityMode,
) -> starknet::core::types::DataAvailabilityMode {
    match mode {
        katana_primitives::da::DataAvailabilityMode::L1 => {
            starknet::core::types::DataAvailabilityMode::L1
        }
        katana_primitives::da::DataAvailabilityMode::L2 => {
            starknet::core::types::DataAvailabilityMode::L2
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
