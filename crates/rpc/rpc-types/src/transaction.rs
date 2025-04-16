use std::sync::Arc;

use anyhow::Result;
use derive_more::Deref;
use katana_primitives::chain::ChainId;
use katana_primitives::class::{ClassHash, ContractClass};
use katana_primitives::contract::ContractAddress;
use katana_primitives::da::DataAvailabilityMode;
use katana_primitives::fee::{ResourceBounds, ResourceBoundsMapping};
use katana_primitives::transaction::{
    DeclareTx, DeclareTxV3, DeclareTxWithClass, DeployAccountTx, DeployAccountTxV3, InvokeTx,
    InvokeTxV3, TxHash, TxWithHash,
};
use katana_primitives::Felt;
use num_traits::ToPrimitive;
use serde::{Deserialize, Serialize};
use starknet::core::types::{
    BroadcastedDeclareTransaction, BroadcastedDeployAccountTransaction,
    BroadcastedDeployAccountTransactionV3, BroadcastedInvokeTransaction, DeclareTransactionContent,
    DeclareTransactionResult, DeclareTransactionV0Content, DeclareTransactionV1Content,
    DeclareTransactionV2Content, DeclareTransactionV3Content, DeployAccountTransactionContent,
    DeployAccountTransactionResult, DeployAccountTransactionV1, DeployAccountTransactionV1Content,
    DeployAccountTransactionV3, DeployAccountTransactionV3Content, DeployTransactionContent,
    InvokeTransactionContent, InvokeTransactionResult, InvokeTransactionV0Content,
    InvokeTransactionV1Content, InvokeTransactionV3Content, L1HandlerTransactionContent,
    TransactionContent,
};
use starknet::core::utils::get_contract_address;

use crate::class::{RpcContractClass, RpcSierraContractClass};
use crate::receipt::TxReceiptWithBlockInfo;
use crate::utils::compiled_class_hash_from_flattened_sierra_class;

pub const CHUNK_SIZE_DEFAULT: u64 = 100;

#[derive(Debug, Clone, Serialize, Deserialize, Deref)]
#[serde(transparent)]
pub struct BroadcastedInvokeTx(pub BroadcastedInvokeTransaction);

impl BroadcastedInvokeTx {
    pub fn is_query(&self) -> bool {
        self.0.is_query
    }

    pub fn into_tx_with_chain_id(self, chain_id: ChainId) -> InvokeTx {
        InvokeTx::V3(InvokeTxV3 {
            chain_id,
            nonce: self.0.nonce,
            calldata: self.0.calldata,
            signature: self.0.signature,
            sender_address: self.0.sender_address.into(),
            account_deployment_data: self.0.account_deployment_data,
            fee_data_availability_mode: from_rpc_da_mode(self.0.fee_data_availability_mode),
            nonce_data_availability_mode: from_rpc_da_mode(self.0.nonce_data_availability_mode),
            paymaster_data: self.0.paymaster_data,
            resource_bounds: from_rpc_resource_bounds(self.0.resource_bounds),
            tip: self.0.tip,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Deref)]
#[serde(transparent)]
pub struct BroadcastedDeclareTx(pub BroadcastedDeclareTransaction);

impl BroadcastedDeclareTx {
    /// Validates that the provided compiled class hash is computed correctly from the class
    /// provided in the transaction.
    pub fn validate_compiled_class_hash(&self) -> Result<bool> {
        let hash = compiled_class_hash_from_flattened_sierra_class(&self.0.contract_class)?;
        Ok(hash == self.0.compiled_class_hash)
    }

    // TODO: change the contract class type for the broadcasted tx to katana-rpc-types instead for
    // easier conversion.
    /// This function assumes that the compiled class hash is valid.
    pub fn try_into_tx_with_chain_id(self, chain_id: ChainId) -> Result<DeclareTxWithClass> {
        let class_hash = self.0.contract_class.class_hash();

        let rpc_class = Arc::unwrap_or_clone(self.0.contract_class);
        let rpc_class = RpcSierraContractClass::try_from(rpc_class).unwrap();
        let class = ContractClass::try_from(RpcContractClass::Class(rpc_class)).unwrap();

        let tx = DeclareTx::V3(DeclareTxV3 {
            chain_id,
            class_hash,
            tip: self.0.tip,
            nonce: self.0.nonce,
            signature: self.0.signature,
            paymaster_data: self.0.paymaster_data,
            sender_address: self.0.sender_address.into(),
            compiled_class_hash: self.0.compiled_class_hash,
            account_deployment_data: self.0.account_deployment_data,
            resource_bounds: from_rpc_resource_bounds(self.0.resource_bounds),
            fee_data_availability_mode: from_rpc_da_mode(self.0.fee_data_availability_mode),
            nonce_data_availability_mode: from_rpc_da_mode(self.0.nonce_data_availability_mode),
        });

        Ok(DeclareTxWithClass::new(tx, class))
    }

    pub fn is_query(&self) -> bool {
        self.0.is_query
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Deref)]
#[serde(transparent)]
pub struct BroadcastedDeployAccountTx(pub BroadcastedDeployAccountTransaction);

impl BroadcastedDeployAccountTx {
    pub fn is_query(&self) -> bool {
        self.0.is_query
    }

    pub fn into_tx_with_chain_id(self, chain_id: ChainId) -> DeployAccountTx {
        let contract_address = get_contract_address(
            self.0.contract_address_salt,
            self.0.class_hash,
            &self.0.constructor_calldata,
            Felt::ZERO,
        );

        DeployAccountTx::V3(DeployAccountTxV3 {
            chain_id,
            nonce: self.0.nonce,
            signature: self.0.signature,
            class_hash: self.0.class_hash,
            contract_address: contract_address.into(),
            constructor_calldata: self.0.constructor_calldata,
            contract_address_salt: self.0.contract_address_salt,
            fee_data_availability_mode: from_rpc_da_mode(self.0.fee_data_availability_mode),
            nonce_data_availability_mode: from_rpc_da_mode(self.0.nonce_data_availability_mode),
            paymaster_data: self.0.paymaster_data,
            resource_bounds: from_rpc_resource_bounds(self.0.resource_bounds),
            tip: self.0.tip,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum BroadcastedTx {
    Invoke(BroadcastedInvokeTx),
    Declare(BroadcastedDeclareTx),
    DeployAccount(BroadcastedDeployAccountTx),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Tx(pub starknet::core::types::Transaction);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TxContent(pub starknet::core::types::TransactionContent);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DeployAccountTxResult(DeployAccountTransactionResult);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DeclareTxResult(DeclareTransactionResult);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct InvokeTxResult(InvokeTransactionResult);

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

impl DeployAccountTxResult {
    pub fn new(transaction_hash: TxHash, contract_address: ContractAddress) -> Self {
        Self(DeployAccountTransactionResult {
            transaction_hash,
            contract_address: contract_address.into(),
        })
    }
}

impl DeclareTxResult {
    pub fn new(transaction_hash: TxHash, class_hash: ClassHash) -> Self {
        Self(DeclareTransactionResult { transaction_hash, class_hash })
    }
}

impl InvokeTxResult {
    pub fn new(transaction_hash: TxHash) -> Self {
        Self(InvokeTransactionResult { transaction_hash })
    }
}

impl From<(TxHash, ContractAddress)> for DeployAccountTxResult {
    fn from((transaction_hash, contract_address): (TxHash, ContractAddress)) -> Self {
        Self::new(transaction_hash, contract_address)
    }
}

impl From<(TxHash, ClassHash)> for DeclareTxResult {
    fn from((transaction_hash, class_hash): (TxHash, ClassHash)) -> Self {
        Self::new(transaction_hash, class_hash)
    }
}

impl From<TxHash> for InvokeTxResult {
    fn from(transaction_hash: TxHash) -> Self {
        Self::new(transaction_hash)
    }
}

impl From<BroadcastedInvokeTx> for InvokeTx {
    fn from(tx: BroadcastedInvokeTx) -> Self {
        InvokeTx::V3(InvokeTxV3 {
            nonce: tx.0.nonce,
            calldata: tx.0.calldata,
            signature: tx.0.signature,
            chain_id: ChainId::default(),
            sender_address: tx.0.sender_address.into(),
            account_deployment_data: tx.0.account_deployment_data,
            fee_data_availability_mode: from_rpc_da_mode(tx.0.fee_data_availability_mode),
            nonce_data_availability_mode: from_rpc_da_mode(tx.0.nonce_data_availability_mode),
            paymaster_data: tx.0.paymaster_data,
            resource_bounds: from_rpc_resource_bounds(tx.0.resource_bounds),
            tip: tx.0.tip,
        })
    }
}

impl From<BroadcastedDeployAccountTx> for DeployAccountTx {
    fn from(tx: BroadcastedDeployAccountTx) -> Self {
        let BroadcastedDeployAccountTransactionV3 {
            tip,
            nonce,
            signature,
            class_hash,
            paymaster_data,
            resource_bounds,
            constructor_calldata,
            contract_address_salt,
            fee_data_availability_mode,
            nonce_data_availability_mode,
            ..
        } = tx.0;

        let contract_address = get_contract_address(
            contract_address_salt,
            class_hash,
            &constructor_calldata,
            Felt::ZERO,
        );

        DeployAccountTx::V3(DeployAccountTxV3 {
            nonce,
            class_hash,
            chain_id: ChainId::default(),
            contract_address: contract_address.into(),
            contract_address_salt,
            fee_data_availability_mode: from_rpc_da_mode(fee_data_availability_mode),
            nonce_data_availability_mode: from_rpc_da_mode(nonce_data_availability_mode),
            resource_bounds: from_rpc_resource_bounds(resource_bounds),
            constructor_calldata,
            paymaster_data,
            signature,
            tip,
        })
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

fn from_rpc_da_mode(mode: starknet::core::types::DataAvailabilityMode) -> DataAvailabilityMode {
    match mode {
        starknet::core::types::DataAvailabilityMode::L1 => DataAvailabilityMode::L1,
        starknet::core::types::DataAvailabilityMode::L2 => DataAvailabilityMode::L2,
    }
}

fn to_rpc_da_mode(mode: DataAvailabilityMode) -> starknet::core::types::DataAvailabilityMode {
    match mode {
        DataAvailabilityMode::L1 => starknet::core::types::DataAvailabilityMode::L1,
        DataAvailabilityMode::L2 => starknet::core::types::DataAvailabilityMode::L2,
    }
}

fn from_rpc_resource_bounds(
    rpc_bounds: starknet::core::types::ResourceBoundsMapping,
) -> ResourceBoundsMapping {
    ResourceBoundsMapping {
        l1_gas: ResourceBounds {
            max_amount: rpc_bounds.l1_gas.max_amount,
            max_price_per_unit: rpc_bounds.l1_gas.max_price_per_unit,
        },
        l2_gas: ResourceBounds {
            max_amount: rpc_bounds.l2_gas.max_amount,
            max_price_per_unit: rpc_bounds.l2_gas.max_price_per_unit,
        },
    }
}

fn to_rpc_resource_bounds(
    bounds: ResourceBoundsMapping,
) -> starknet::core::types::ResourceBoundsMapping {
    starknet::core::types::ResourceBoundsMapping {
        l1_gas: starknet::core::types::ResourceBounds {
            max_amount: bounds.l1_gas.max_amount,
            max_price_per_unit: bounds.l1_gas.max_price_per_unit,
        },
        l2_gas: starknet::core::types::ResourceBounds {
            max_amount: bounds.l2_gas.max_amount,
            max_price_per_unit: bounds.l2_gas.max_price_per_unit,
        },
        l1_data_gas: starknet::core::types::ResourceBounds {
            max_amount: bounds.l2_gas.max_amount,
            max_price_per_unit: bounds.l2_gas.max_price_per_unit,
        },
    }
}
