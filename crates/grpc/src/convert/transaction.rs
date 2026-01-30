//! Transaction type conversions.

use katana_primitives::Felt;

use super::FeltVecExt;
use crate::protos::types::transaction::Transaction as ProtoTxVariant;
use crate::protos::types::{
    DeployAccountTxn, DeployAccountTxnV3, Felt as ProtoFelt, InvokeTxnV1, InvokeTxnV3,
    ResourceBounds, ResourceBoundsMapping, Transaction as ProtoTx,
};

/// Convert RPC transaction to proto Transaction.
impl From<katana_rpc_types::transaction::RpcTxWithHash> for ProtoTx {
    fn from(tx: katana_rpc_types::transaction::RpcTxWithHash) -> Self {
        use katana_rpc_types::transaction::RpcTx;

        let transaction = match tx.transaction {
            RpcTx::Invoke(invoke) => match invoke {
                katana_rpc_types::transaction::InvokeTx::V1(v1) => {
                    ProtoTxVariant::InvokeV1(InvokeTxnV1 {
                        max_fee: Some(v1.max_fee.into()),
                        version: "0x1".to_string(),
                        signature: v1.signature.to_proto_felts(),
                        nonce: Some(v1.nonce.into()),
                        r#type: "INVOKE".to_string(),
                        sender_address: Some(ProtoFelt::from(Felt::from(v1.sender_address))),
                        calldata: v1.calldata.to_proto_felts(),
                    })
                }
                katana_rpc_types::transaction::InvokeTx::V3(v3) => {
                    ProtoTxVariant::InvokeV3(InvokeTxnV3 {
                        r#type: "INVOKE".to_string(),
                        sender_address: Some(ProtoFelt::from(Felt::from(v3.sender_address))),
                        calldata: v3.calldata.to_proto_felts(),
                        version: "0x3".to_string(),
                        signature: v3.signature.to_proto_felts(),
                        nonce: Some(v3.nonce.into()),
                        resource_bounds: Some(ResourceBoundsMapping::from(&v3.resource_bounds)),
                        tip: Some(ProtoFelt::from(Felt::from(v3.tip))),
                        paymaster_data: v3.paymaster_data.to_proto_felts(),
                        account_deployment_data: v3.account_deployment_data.to_proto_felts(),
                        nonce_data_availability_mode: v3.nonce_data_availability_mode.to_string(),
                        fee_data_availability_mode: v3.fee_data_availability_mode.to_string(),
                    })
                }
            },
            RpcTx::Declare(declare) => match declare {
                katana_rpc_types::transaction::DeclareTx::V1(v1) => {
                    ProtoTxVariant::DeclareV1(crate::protos::types::DeclareTxnV1 {
                        max_fee: Some(v1.max_fee.into()),
                        version: "0x1".to_string(),
                        signature: v1.signature.to_proto_felts(),
                        nonce: Some(v1.nonce.into()),
                        r#type: "DECLARE".to_string(),
                        class_hash: Some(v1.class_hash.into()),
                        sender_address: Some(ProtoFelt::from(Felt::from(v1.sender_address))),
                    })
                }
                katana_rpc_types::transaction::DeclareTx::V2(v2) => {
                    ProtoTxVariant::DeclareV2(crate::protos::types::DeclareTxnV2 {
                        r#type: "DECLARE".to_string(),
                        sender_address: Some(ProtoFelt::from(Felt::from(v2.sender_address))),
                        compiled_class_hash: Some(v2.compiled_class_hash.into()),
                        max_fee: Some(v2.max_fee.into()),
                        version: "0x2".to_string(),
                        signature: v2.signature.to_proto_felts(),
                        nonce: Some(v2.nonce.into()),
                        class: Vec::new(), // Contract class is not included in transaction response
                    })
                }
                katana_rpc_types::transaction::DeclareTx::V3(v3) => {
                    ProtoTxVariant::DeclareV3(crate::protos::types::DeclareTxnV3 {
                        r#type: "DECLARE".to_string(),
                        sender_address: Some(ProtoFelt::from(Felt::from(v3.sender_address))),
                        compiled_class_hash: Some(v3.compiled_class_hash.into()),
                        version: "0x3".to_string(),
                        signature: v3.signature.to_proto_felts(),
                        nonce: Some(v3.nonce.into()),
                        class_hash: Some(v3.class_hash.into()),
                        resource_bounds: Some(ResourceBoundsMapping::from(&v3.resource_bounds)),
                        tip: Some(ProtoFelt::from(Felt::from(v3.tip))),
                        paymaster_data: v3.paymaster_data.to_proto_felts(),
                        account_deployment_data: v3.account_deployment_data.to_proto_felts(),
                        nonce_data_availability_mode: v3.nonce_data_availability_mode.to_string(),
                        fee_data_availability_mode: v3.fee_data_availability_mode.to_string(),
                    })
                }
            },
            RpcTx::DeployAccount(deploy) => match deploy {
                katana_rpc_types::transaction::DeployAccountTx::V1(v1) => {
                    ProtoTxVariant::DeployAccount(DeployAccountTxn {
                        max_fee: Some(v1.max_fee.into()),
                        version: "0x1".to_string(),
                        signature: v1.signature.to_proto_felts(),
                        nonce: Some(v1.nonce.into()),
                        r#type: "DEPLOY_ACCOUNT".to_string(),
                        class_hash: Some(v1.class_hash.into()),
                        contract_address_salt: Some(v1.contract_address_salt.into()),
                        constructor_calldata: v1.constructor_calldata.to_proto_felts(),
                    })
                }
                katana_rpc_types::transaction::DeployAccountTx::V3(v3) => {
                    ProtoTxVariant::DeployAccountV3(DeployAccountTxnV3 {
                        r#type: "DEPLOY_ACCOUNT".to_string(),
                        version: "0x3".to_string(),
                        signature: v3.signature.to_proto_felts(),
                        nonce: Some(v3.nonce.into()),
                        contract_address_salt: Some(v3.contract_address_salt.into()),
                        constructor_calldata: v3.constructor_calldata.to_proto_felts(),
                        class_hash: Some(v3.class_hash.into()),
                        resource_bounds: Some(ResourceBoundsMapping::from(&v3.resource_bounds)),
                        tip: Some(ProtoFelt::from(Felt::from(v3.tip))),
                        paymaster_data: v3.paymaster_data.to_proto_felts(),
                        nonce_data_availability_mode: v3.nonce_data_availability_mode.to_string(),
                        fee_data_availability_mode: v3.fee_data_availability_mode.to_string(),
                    })
                }
            },
            RpcTx::L1Handler(l1) => {
                // L1Handler doesn't have a direct proto representation in the current schema
                // We'll represent it as an InvokeV1 with the relevant fields
                ProtoTxVariant::InvokeV1(InvokeTxnV1 {
                    max_fee: Some(Felt::ZERO.into()),
                    version: "0x0".to_string(),
                    signature: Vec::new(),
                    nonce: Some(l1.nonce.into()),
                    r#type: "L1_HANDLER".to_string(),
                    sender_address: Some(ProtoFelt::from(Felt::from(l1.contract_address))),
                    calldata: l1.calldata.to_proto_felts(),
                })
            }
        };

        ProtoTx { transaction: Some(transaction) }
    }
}

impl From<&katana_rpc_types::transaction::ResourceBoundsMapping> for ResourceBoundsMapping {
    fn from(bounds: &katana_rpc_types::transaction::ResourceBoundsMapping) -> Self {
        ResourceBoundsMapping {
            l1_gas: Some(ResourceBounds {
                max_amount: Some(ProtoFelt::from(Felt::from(bounds.l1_gas.max_amount))),
                max_price_per_unit: Some(ProtoFelt::from(Felt::from(
                    bounds.l1_gas.max_price_per_unit,
                ))),
            }),
            l2_gas: Some(ResourceBounds {
                max_amount: Some(ProtoFelt::from(Felt::from(bounds.l2_gas.max_amount))),
                max_price_per_unit: Some(ProtoFelt::from(Felt::from(
                    bounds.l2_gas.max_price_per_unit,
                ))),
            }),
        }
    }
}
