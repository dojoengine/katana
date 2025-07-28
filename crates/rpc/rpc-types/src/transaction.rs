use katana_primitives::da::DataAvailabilityMode;
use katana_primitives::fee::ResourceBoundsMapping;
use katana_primitives::transaction::{DeclareTx, DeployAccountTx, InvokeTx, TxWithHash};
use num_traits::ToPrimitive;
use serde::{Deserialize, Serialize};
use starknet::core::types::{
    DeclareTransactionContent, DeclareTransactionV0Content, DeclareTransactionV1Content,
    DeclareTransactionV2Content, DeclareTransactionV3Content, DeployAccountTransactionContent,
    DeployAccountTransactionV1, DeployAccountTransactionV1Content, DeployAccountTransactionV3,
    DeployAccountTransactionV3Content, DeployTransactionContent, InvokeTransactionContent,
    InvokeTransactionV0Content, InvokeTransactionV1Content, InvokeTransactionV3Content,
    L1HandlerTransactionContent, TransactionContent,
};

use crate::receipt::TxReceiptWithBlockInfo;

pub const CHUNK_SIZE_DEFAULT: u64 = 100;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Tx(pub starknet::core::types::Transaction);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TxContent(pub starknet::core::types::TransactionContent);

impl From<TxWithHash> for Tx {
    fn from(value: TxWithHash) -> Self {
        use katana_primitives::transaction::Tx as InternalTx;

        let transaction_hash = value.hash;
        let tx = match value.transaction {
            InternalTx::Invoke(invoke) => match invoke {
                InvokeTx::V0(tx) => starknet::core::types::Transaction::Invoke(
                    starknet::core::types::InvokeTransaction::V0(
                        starknet::core::types::InvokeTransactionV0 {
                            transaction_hash,
                            calldata: tx.calldata,
                            signature: tx.signature,
                            max_fee: tx.max_fee.into(),
                            contract_address: tx.contract_address.into(),
                            entry_point_selector: tx.entry_point_selector,
                        },
                    ),
                ),

                InvokeTx::V1(tx) => starknet::core::types::Transaction::Invoke(
                    starknet::core::types::InvokeTransaction::V1(
                        starknet::core::types::InvokeTransactionV1 {
                            nonce: tx.nonce,
                            transaction_hash,
                            calldata: tx.calldata,
                            signature: tx.signature,
                            max_fee: tx.max_fee.into(),
                            sender_address: tx.sender_address.into(),
                        },
                    ),
                ),

                InvokeTx::V3(tx) => starknet::core::types::Transaction::Invoke(
                    starknet::core::types::InvokeTransaction::V3(
                        starknet::core::types::InvokeTransactionV3 {
                            nonce: tx.nonce,
                            transaction_hash,
                            calldata: tx.calldata,
                            signature: tx.signature,
                            sender_address: tx.sender_address.into(),
                            account_deployment_data: tx.account_deployment_data,
                            fee_data_availability_mode: to_rpc_da_mode(
                                tx.fee_data_availability_mode,
                            ),
                            nonce_data_availability_mode: to_rpc_da_mode(
                                tx.nonce_data_availability_mode,
                            ),
                            paymaster_data: tx.paymaster_data,
                            resource_bounds: to_rpc_resource_bounds(tx.resource_bounds),
                            tip: tx.tip,
                        },
                    ),
                ),
            },

            InternalTx::Declare(tx) => starknet::core::types::Transaction::Declare(match tx {
                DeclareTx::V0(tx) => starknet::core::types::DeclareTransaction::V0(
                    starknet::core::types::DeclareTransactionV0 {
                        transaction_hash,
                        signature: tx.signature,
                        class_hash: tx.class_hash,
                        max_fee: tx.max_fee.into(),
                        sender_address: tx.sender_address.into(),
                    },
                ),

                DeclareTx::V1(tx) => starknet::core::types::DeclareTransaction::V1(
                    starknet::core::types::DeclareTransactionV1 {
                        nonce: tx.nonce,
                        transaction_hash,
                        signature: tx.signature,
                        class_hash: tx.class_hash,
                        max_fee: tx.max_fee.into(),
                        sender_address: tx.sender_address.into(),
                    },
                ),

                DeclareTx::V2(tx) => starknet::core::types::DeclareTransaction::V2(
                    starknet::core::types::DeclareTransactionV2 {
                        nonce: tx.nonce,
                        transaction_hash,
                        signature: tx.signature,
                        class_hash: tx.class_hash,
                        max_fee: tx.max_fee.into(),
                        sender_address: tx.sender_address.into(),
                        compiled_class_hash: tx.compiled_class_hash,
                    },
                ),

                DeclareTx::V3(tx) => starknet::core::types::DeclareTransaction::V3(
                    starknet::core::types::DeclareTransactionV3 {
                        nonce: tx.nonce,
                        transaction_hash,
                        signature: tx.signature,
                        class_hash: tx.class_hash,
                        sender_address: tx.sender_address.into(),
                        compiled_class_hash: tx.compiled_class_hash,
                        account_deployment_data: tx.account_deployment_data,
                        fee_data_availability_mode: to_rpc_da_mode(tx.fee_data_availability_mode),
                        nonce_data_availability_mode: to_rpc_da_mode(
                            tx.nonce_data_availability_mode,
                        ),
                        paymaster_data: tx.paymaster_data,
                        resource_bounds: to_rpc_resource_bounds(tx.resource_bounds),
                        tip: tx.tip,
                    },
                ),
            }),

            InternalTx::L1Handler(tx) => starknet::core::types::Transaction::L1Handler(
                starknet::core::types::L1HandlerTransaction {
                    transaction_hash,
                    calldata: tx.calldata,
                    contract_address: tx.contract_address.into(),
                    entry_point_selector: tx.entry_point_selector,
                    nonce: tx.nonce.to_u64().expect("nonce should fit in u64"),
                    version: tx.version,
                },
            ),

            InternalTx::DeployAccount(tx) => {
                starknet::core::types::Transaction::DeployAccount(match tx {
                    DeployAccountTx::V1(tx) => starknet::core::types::DeployAccountTransaction::V1(
                        DeployAccountTransactionV1 {
                            transaction_hash,
                            nonce: tx.nonce,
                            signature: tx.signature,
                            class_hash: tx.class_hash,
                            max_fee: tx.max_fee.into(),
                            constructor_calldata: tx.constructor_calldata,
                            contract_address_salt: tx.contract_address_salt,
                        },
                    ),

                    DeployAccountTx::V3(tx) => starknet::core::types::DeployAccountTransaction::V3(
                        DeployAccountTransactionV3 {
                            transaction_hash,
                            nonce: tx.nonce,
                            signature: tx.signature,
                            class_hash: tx.class_hash,
                            constructor_calldata: tx.constructor_calldata,
                            contract_address_salt: tx.contract_address_salt,
                            fee_data_availability_mode: to_rpc_da_mode(
                                tx.fee_data_availability_mode,
                            ),
                            nonce_data_availability_mode: to_rpc_da_mode(
                                tx.nonce_data_availability_mode,
                            ),
                            paymaster_data: tx.paymaster_data,
                            resource_bounds: to_rpc_resource_bounds(tx.resource_bounds),
                            tip: tx.tip,
                        },
                    ),
                })
            }

            InternalTx::Deploy(tx) => starknet::core::types::Transaction::Deploy(
                starknet::core::types::DeployTransaction {
                    constructor_calldata: tx.constructor_calldata,
                    contract_address_salt: tx.contract_address_salt,
                    class_hash: tx.class_hash,
                    version: tx.version,
                    transaction_hash,
                },
            ),
        };

        Tx(tx)
    }
}

impl From<TxWithHash> for TxContent {
    fn from(value: TxWithHash) -> Self {
        use katana_primitives::transaction::Tx as InternalTx;

        let tx = match value.transaction {
            InternalTx::Invoke(invoke) => match invoke {
                InvokeTx::V0(tx) => TransactionContent::Invoke(InvokeTransactionContent::V0(
                    InvokeTransactionV0Content {
                        calldata: tx.calldata,
                        signature: tx.signature,
                        max_fee: tx.max_fee.into(),
                        contract_address: tx.contract_address.into(),
                        entry_point_selector: tx.entry_point_selector,
                    },
                )),

                InvokeTx::V1(tx) => TransactionContent::Invoke(InvokeTransactionContent::V1(
                    InvokeTransactionV1Content {
                        nonce: tx.nonce,
                        calldata: tx.calldata,
                        signature: tx.signature,
                        max_fee: tx.max_fee.into(),
                        sender_address: tx.sender_address.into(),
                    },
                )),

                InvokeTx::V3(tx) => TransactionContent::Invoke(InvokeTransactionContent::V3(
                    InvokeTransactionV3Content {
                        nonce: tx.nonce,
                        calldata: tx.calldata,
                        signature: tx.signature,
                        sender_address: tx.sender_address.into(),
                        account_deployment_data: tx.account_deployment_data,
                        fee_data_availability_mode: to_rpc_da_mode(tx.fee_data_availability_mode),
                        nonce_data_availability_mode: to_rpc_da_mode(
                            tx.nonce_data_availability_mode,
                        ),
                        paymaster_data: tx.paymaster_data,
                        resource_bounds: to_rpc_resource_bounds(tx.resource_bounds),
                        tip: tx.tip,
                    },
                )),
            },

            InternalTx::Declare(tx) => TransactionContent::Declare(match tx {
                DeclareTx::V0(tx) => DeclareTransactionContent::V0(DeclareTransactionV0Content {
                    signature: tx.signature,
                    class_hash: tx.class_hash,
                    max_fee: tx.max_fee.into(),
                    sender_address: tx.sender_address.into(),
                }),

                DeclareTx::V1(tx) => DeclareTransactionContent::V1(DeclareTransactionV1Content {
                    nonce: tx.nonce,
                    signature: tx.signature,
                    class_hash: tx.class_hash,
                    max_fee: tx.max_fee.into(),
                    sender_address: tx.sender_address.into(),
                }),

                DeclareTx::V2(tx) => DeclareTransactionContent::V2(DeclareTransactionV2Content {
                    nonce: tx.nonce,
                    signature: tx.signature,
                    class_hash: tx.class_hash,
                    max_fee: tx.max_fee.into(),
                    sender_address: tx.sender_address.into(),
                    compiled_class_hash: tx.compiled_class_hash,
                }),

                DeclareTx::V3(tx) => DeclareTransactionContent::V3(DeclareTransactionV3Content {
                    nonce: tx.nonce,
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

            InternalTx::L1Handler(tx) => {
                TransactionContent::L1Handler(L1HandlerTransactionContent {
                    calldata: tx.calldata,
                    contract_address: tx.contract_address.into(),
                    entry_point_selector: tx.entry_point_selector,
                    nonce: tx.nonce.to_u64().expect("nonce should fit in u64"),
                    version: tx.version,
                })
            }

            InternalTx::DeployAccount(tx) => TransactionContent::DeployAccount(match tx {
                DeployAccountTx::V1(tx) => {
                    DeployAccountTransactionContent::V1(DeployAccountTransactionV1Content {
                        nonce: tx.nonce,
                        signature: tx.signature,
                        class_hash: tx.class_hash,
                        max_fee: tx.max_fee.into(),
                        constructor_calldata: tx.constructor_calldata,
                        contract_address_salt: tx.contract_address_salt,
                    })
                }

                DeployAccountTx::V3(tx) => {
                    DeployAccountTransactionContent::V3(DeployAccountTransactionV3Content {
                        nonce: tx.nonce,
                        signature: tx.signature,
                        class_hash: tx.class_hash,
                        constructor_calldata: tx.constructor_calldata,
                        contract_address_salt: tx.contract_address_salt,
                        fee_data_availability_mode: to_rpc_da_mode(tx.fee_data_availability_mode),
                        nonce_data_availability_mode: to_rpc_da_mode(
                            tx.nonce_data_availability_mode,
                        ),
                        paymaster_data: tx.paymaster_data,
                        resource_bounds: to_rpc_resource_bounds(tx.resource_bounds),
                        tip: tx.tip,
                    })
                }
            }),

            InternalTx::Deploy(tx) => TransactionContent::Deploy(DeployTransactionContent {
                constructor_calldata: tx.constructor_calldata,
                contract_address_salt: tx.contract_address_salt,
                class_hash: tx.class_hash,
                version: tx.version,
            }),
        };

        TxContent(tx)
    }
}

impl From<starknet::core::types::Transaction> for Tx {
    fn from(value: starknet::core::types::Transaction) -> Self {
        Self(value)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Copy)]
pub struct TransactionsPageCursor {
    pub block_number: u64,
    pub transaction_index: u64,
    pub chunk_size: u64,
}

impl Default for TransactionsPageCursor {
    fn default() -> Self {
        Self { block_number: 0, transaction_index: 0, chunk_size: CHUNK_SIZE_DEFAULT }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionsPage {
    pub transactions: Vec<(TxWithHash, TxReceiptWithBlockInfo)>,
    pub cursor: TransactionsPageCursor,
}

// TODO: find a solution to avoid doing this conversion, this is not pretty at all. the reason why
// we had to do this in the first place is because of the orphan rule. i think eventually we should
// not rely on `starknet-rs` rpc types anymore and should instead define the types ourselves to have
// more flexibility.

fn to_rpc_da_mode(mode: DataAvailabilityMode) -> starknet::core::types::DataAvailabilityMode {
    match mode {
        DataAvailabilityMode::L1 => starknet::core::types::DataAvailabilityMode::L1,
        DataAvailabilityMode::L2 => starknet::core::types::DataAvailabilityMode::L2,
    }
}

fn to_rpc_resource_bounds(
    bounds: ResourceBoundsMapping,
) -> starknet::core::types::ResourceBoundsMapping {
    match bounds {
        ResourceBoundsMapping::All(all_bounds) => starknet::core::types::ResourceBoundsMapping {
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
        },
        // The `l1_data_gas` bounds should actually be ommitted but because `starknet-rs` doesn't
        // support older RPC spec, we default to zero. This aren't technically accurate so should
        // find an alternative or completely remove legacy support. But we need to support in order
        // to maintain backward compatibility from older database version.
        ResourceBoundsMapping::L1Gas(l1_gas_bounds) => {
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
