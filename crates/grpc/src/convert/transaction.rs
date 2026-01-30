//! Transaction type conversions.

use super::{to_proto_felt, to_proto_felts};
use crate::protos::types::{
    transaction::Transaction as ProtoTxVariant, DeployAccountTxn, DeployAccountTxnV3,
    InvokeTxnV1, InvokeTxnV3, ResourceBounds, ResourceBoundsMapping, Transaction as ProtoTx,
};

/// Converts an RPC transaction to a proto Transaction.
pub fn to_proto_transaction(tx: katana_rpc_types::transaction::RpcTxWithHash) -> ProtoTx {
    use katana_rpc_types::transaction::RpcTx;

    let transaction = match tx.transaction {
        RpcTx::Invoke(invoke) => match invoke {
            katana_rpc_types::transaction::InvokeTx::V1(v1) => {
                ProtoTxVariant::InvokeV1(InvokeTxnV1 {
                    max_fee: Some(to_proto_felt(v1.max_fee)),
                    version: "0x1".to_string(),
                    signature: to_proto_felts(&v1.signature),
                    nonce: Some(to_proto_felt(v1.nonce)),
                    r#type: "INVOKE".to_string(),
                    sender_address: Some(to_proto_felt(v1.sender_address.into())),
                    calldata: to_proto_felts(&v1.calldata),
                })
            }
            katana_rpc_types::transaction::InvokeTx::V3(v3) => {
                ProtoTxVariant::InvokeV3(InvokeTxnV3 {
                    r#type: "INVOKE".to_string(),
                    sender_address: Some(to_proto_felt(v3.sender_address.into())),
                    calldata: to_proto_felts(&v3.calldata),
                    version: "0x3".to_string(),
                    signature: to_proto_felts(&v3.signature),
                    nonce: Some(to_proto_felt(v3.nonce)),
                    resource_bounds: Some(to_proto_resource_bounds(&v3.resource_bounds)),
                    tip: Some(to_proto_felt(v3.tip.into())),
                    paymaster_data: to_proto_felts(&v3.paymaster_data),
                    account_deployment_data: to_proto_felts(&v3.account_deployment_data),
                    nonce_data_availability_mode: v3.nonce_data_availability_mode.to_string(),
                    fee_data_availability_mode: v3.fee_data_availability_mode.to_string(),
                })
            }
        },
        RpcTx::Declare(declare) => match declare {
            katana_rpc_types::transaction::DeclareTx::V1(v1) => {
                ProtoTxVariant::DeclareV1(crate::protos::types::DeclareTxnV1 {
                    max_fee: Some(to_proto_felt(v1.max_fee)),
                    version: "0x1".to_string(),
                    signature: to_proto_felts(&v1.signature),
                    nonce: Some(to_proto_felt(v1.nonce)),
                    r#type: "DECLARE".to_string(),
                    class_hash: Some(to_proto_felt(v1.class_hash)),
                    sender_address: Some(to_proto_felt(v1.sender_address.into())),
                })
            }
            katana_rpc_types::transaction::DeclareTx::V2(v2) => {
                ProtoTxVariant::DeclareV2(crate::protos::types::DeclareTxnV2 {
                    r#type: "DECLARE".to_string(),
                    sender_address: Some(to_proto_felt(v2.sender_address.into())),
                    compiled_class_hash: Some(to_proto_felt(v2.compiled_class_hash)),
                    max_fee: Some(to_proto_felt(v2.max_fee)),
                    version: "0x2".to_string(),
                    signature: to_proto_felts(&v2.signature),
                    nonce: Some(to_proto_felt(v2.nonce)),
                    class: Vec::new(), // Contract class is not included in transaction response
                })
            }
            katana_rpc_types::transaction::DeclareTx::V3(v3) => {
                ProtoTxVariant::DeclareV3(crate::protos::types::DeclareTxnV3 {
                    r#type: "DECLARE".to_string(),
                    sender_address: Some(to_proto_felt(v3.sender_address.into())),
                    compiled_class_hash: Some(to_proto_felt(v3.compiled_class_hash)),
                    version: "0x3".to_string(),
                    signature: to_proto_felts(&v3.signature),
                    nonce: Some(to_proto_felt(v3.nonce)),
                    class_hash: Some(to_proto_felt(v3.class_hash)),
                    resource_bounds: Some(to_proto_resource_bounds(&v3.resource_bounds)),
                    tip: Some(to_proto_felt(v3.tip.into())),
                    paymaster_data: to_proto_felts(&v3.paymaster_data),
                    account_deployment_data: to_proto_felts(&v3.account_deployment_data),
                    nonce_data_availability_mode: v3.nonce_data_availability_mode.to_string(),
                    fee_data_availability_mode: v3.fee_data_availability_mode.to_string(),
                })
            }
        },
        RpcTx::DeployAccount(deploy) => match deploy {
            katana_rpc_types::transaction::DeployAccountTx::V1(v1) => {
                ProtoTxVariant::DeployAccount(DeployAccountTxn {
                    max_fee: Some(to_proto_felt(v1.max_fee)),
                    version: "0x1".to_string(),
                    signature: to_proto_felts(&v1.signature),
                    nonce: Some(to_proto_felt(v1.nonce)),
                    r#type: "DEPLOY_ACCOUNT".to_string(),
                    class_hash: Some(to_proto_felt(v1.class_hash)),
                    contract_address_salt: Some(to_proto_felt(v1.contract_address_salt)),
                    constructor_calldata: to_proto_felts(&v1.constructor_calldata),
                })
            }
            katana_rpc_types::transaction::DeployAccountTx::V3(v3) => {
                ProtoTxVariant::DeployAccountV3(DeployAccountTxnV3 {
                    r#type: "DEPLOY_ACCOUNT".to_string(),
                    version: "0x3".to_string(),
                    signature: to_proto_felts(&v3.signature),
                    nonce: Some(to_proto_felt(v3.nonce)),
                    contract_address_salt: Some(to_proto_felt(v3.contract_address_salt)),
                    constructor_calldata: to_proto_felts(&v3.constructor_calldata),
                    class_hash: Some(to_proto_felt(v3.class_hash)),
                    resource_bounds: Some(to_proto_resource_bounds(&v3.resource_bounds)),
                    tip: Some(to_proto_felt(v3.tip.into())),
                    paymaster_data: to_proto_felts(&v3.paymaster_data),
                    nonce_data_availability_mode: v3.nonce_data_availability_mode.to_string(),
                    fee_data_availability_mode: v3.fee_data_availability_mode.to_string(),
                })
            }
        },
        RpcTx::L1Handler(l1) => {
            // L1Handler doesn't have a direct proto representation in the current schema
            // We'll represent it as an InvokeV1 with the relevant fields
            ProtoTxVariant::InvokeV1(InvokeTxnV1 {
                max_fee: Some(to_proto_felt(katana_primitives::Felt::ZERO)),
                version: "0x0".to_string(),
                signature: Vec::new(),
                nonce: Some(to_proto_felt(l1.nonce)),
                r#type: "L1_HANDLER".to_string(),
                sender_address: Some(to_proto_felt(l1.contract_address.into())),
                calldata: to_proto_felts(&l1.calldata),
            })
        }
    };

    ProtoTx { transaction: Some(transaction) }
}

fn to_proto_resource_bounds(
    bounds: &katana_rpc_types::transaction::ResourceBoundsMapping,
) -> ResourceBoundsMapping {
    ResourceBoundsMapping {
        l1_gas: Some(ResourceBounds {
            max_amount: Some(to_proto_felt(bounds.l1_gas.max_amount.into())),
            max_price_per_unit: Some(to_proto_felt(bounds.l1_gas.max_price_per_unit.into())),
        }),
        l2_gas: Some(ResourceBounds {
            max_amount: Some(to_proto_felt(bounds.l2_gas.max_amount.into())),
            max_price_per_unit: Some(to_proto_felt(bounds.l2_gas.max_price_per_unit.into())),
        }),
    }
}
