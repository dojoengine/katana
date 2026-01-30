//! Transaction type conversions.

use katana_primitives::da::DataAvailabilityMode;
use katana_primitives::Felt;

use super::FeltVecExt;
use crate::protos::types::transaction::Transaction as ProtoTxVariant;
use crate::protos::types::{
    DeployAccountTxn, DeployAccountTxnV3, DeployTxn, Felt as ProtoFelt, InvokeTxnV1, InvokeTxnV3,
    L1HandlerTxn, ResourceBounds, ResourceBoundsMapping, Transaction as ProtoTx,
};

/// Convert DataAvailabilityMode to string representation for proto.
fn da_mode_to_string(mode: DataAvailabilityMode) -> String {
    match mode {
        DataAvailabilityMode::L1 => "L1".to_string(),
        DataAvailabilityMode::L2 => "L2".to_string(),
    }
}

/// Convert RPC transaction to proto Transaction.
impl From<katana_rpc_types::transaction::RpcTxWithHash> for ProtoTx {
    fn from(tx: katana_rpc_types::transaction::RpcTxWithHash) -> Self {
        use katana_rpc_types::transaction::{RpcDeclareTx, RpcDeployAccountTx, RpcInvokeTx, RpcTx};

        let transaction = match tx.transaction {
            RpcTx::Invoke(invoke) => match invoke {
                RpcInvokeTx::V0(v0) => ProtoTxVariant::InvokeV1(InvokeTxnV1 {
                    max_fee: Some(Felt::from(v0.max_fee).into()),
                    version: "0x0".to_string(),
                    signature: v0.signature.to_proto_felts(),
                    nonce: Some(Felt::ZERO.into()),
                    r#type: "INVOKE".to_string(),
                    sender_address: Some(ProtoFelt::from(Felt::from(v0.contract_address))),
                    calldata: v0.calldata.to_proto_felts(),
                }),
                RpcInvokeTx::V1(v1) => ProtoTxVariant::InvokeV1(InvokeTxnV1 {
                    max_fee: Some(Felt::from(v1.max_fee).into()),
                    version: "0x1".to_string(),
                    signature: v1.signature.to_proto_felts(),
                    nonce: Some(v1.nonce.into()),
                    r#type: "INVOKE".to_string(),
                    sender_address: Some(ProtoFelt::from(Felt::from(v1.sender_address))),
                    calldata: v1.calldata.to_proto_felts(),
                }),
                RpcInvokeTx::V3(v3) => ProtoTxVariant::InvokeV3(InvokeTxnV3 {
                    r#type: "INVOKE".to_string(),
                    sender_address: Some(ProtoFelt::from(Felt::from(v3.sender_address))),
                    calldata: v3.calldata.to_proto_felts(),
                    version: "0x3".to_string(),
                    signature: v3.signature.to_proto_felts(),
                    nonce: Some(v3.nonce.into()),
                    resource_bounds: Some(ResourceBoundsMapping::from(&v3.resource_bounds)),
                    tip: Some(ProtoFelt::from(Felt::from(u64::from(v3.tip)))),
                    paymaster_data: v3.paymaster_data.to_proto_felts(),
                    account_deployment_data: v3.account_deployment_data.to_proto_felts(),
                    nonce_data_availability_mode: da_mode_to_string(
                        v3.nonce_data_availability_mode,
                    ),
                    fee_data_availability_mode: da_mode_to_string(v3.fee_data_availability_mode),
                }),
            },
            RpcTx::Declare(declare) => match declare {
                RpcDeclareTx::V0(v0) => {
                    ProtoTxVariant::DeclareV1(crate::protos::types::DeclareTxnV1 {
                        max_fee: Some(Felt::from(v0.max_fee).into()),
                        version: "0x0".to_string(),
                        signature: v0.signature.to_proto_felts(),
                        nonce: Some(Felt::ZERO.into()),
                        r#type: "DECLARE".to_string(),
                        class_hash: Some(v0.class_hash.into()),
                        sender_address: Some(ProtoFelt::from(Felt::from(v0.sender_address))),
                    })
                }
                RpcDeclareTx::V1(v1) => {
                    ProtoTxVariant::DeclareV1(crate::protos::types::DeclareTxnV1 {
                        max_fee: Some(Felt::from(v1.max_fee).into()),
                        version: "0x1".to_string(),
                        signature: v1.signature.to_proto_felts(),
                        nonce: Some(v1.nonce.into()),
                        r#type: "DECLARE".to_string(),
                        class_hash: Some(v1.class_hash.into()),
                        sender_address: Some(ProtoFelt::from(Felt::from(v1.sender_address))),
                    })
                }
                RpcDeclareTx::V2(v2) => {
                    ProtoTxVariant::DeclareV2(crate::protos::types::DeclareTxnV2 {
                        r#type: "DECLARE".to_string(),
                        sender_address: Some(ProtoFelt::from(Felt::from(v2.sender_address))),
                        compiled_class_hash: Some(v2.compiled_class_hash.into()),
                        max_fee: Some(Felt::from(v2.max_fee).into()),
                        version: "0x2".to_string(),
                        signature: v2.signature.to_proto_felts(),
                        nonce: Some(v2.nonce.into()),
                        class: Vec::new(), // Contract class is not included in transaction response
                    })
                }
                RpcDeclareTx::V3(v3) => {
                    ProtoTxVariant::DeclareV3(crate::protos::types::DeclareTxnV3 {
                        r#type: "DECLARE".to_string(),
                        sender_address: Some(ProtoFelt::from(Felt::from(v3.sender_address))),
                        compiled_class_hash: Some(v3.compiled_class_hash.into()),
                        version: "0x3".to_string(),
                        signature: v3.signature.to_proto_felts(),
                        nonce: Some(v3.nonce.into()),
                        class_hash: Some(v3.class_hash.into()),
                        resource_bounds: Some(ResourceBoundsMapping::from(&v3.resource_bounds)),
                        tip: Some(ProtoFelt::from(Felt::from(u64::from(v3.tip)))),
                        paymaster_data: v3.paymaster_data.to_proto_felts(),
                        account_deployment_data: v3.account_deployment_data.to_proto_felts(),
                        nonce_data_availability_mode: da_mode_to_string(
                            v3.nonce_data_availability_mode,
                        ),
                        fee_data_availability_mode: da_mode_to_string(
                            v3.fee_data_availability_mode,
                        ),
                    })
                }
            },
            RpcTx::DeployAccount(deploy) => match deploy {
                RpcDeployAccountTx::V1(v1) => ProtoTxVariant::DeployAccount(DeployAccountTxn {
                    max_fee: Some(Felt::from(v1.max_fee).into()),
                    version: "0x1".to_string(),
                    signature: v1.signature.to_proto_felts(),
                    nonce: Some(v1.nonce.into()),
                    r#type: "DEPLOY_ACCOUNT".to_string(),
                    class_hash: Some(v1.class_hash.into()),
                    contract_address_salt: Some(v1.contract_address_salt.into()),
                    constructor_calldata: v1.constructor_calldata.to_proto_felts(),
                }),
                RpcDeployAccountTx::V3(v3) => ProtoTxVariant::DeployAccountV3(DeployAccountTxnV3 {
                    r#type: "DEPLOY_ACCOUNT".to_string(),
                    version: "0x3".to_string(),
                    signature: v3.signature.to_proto_felts(),
                    nonce: Some(v3.nonce.into()),
                    contract_address_salt: Some(v3.contract_address_salt.into()),
                    constructor_calldata: v3.constructor_calldata.to_proto_felts(),
                    class_hash: Some(v3.class_hash.into()),
                    resource_bounds: Some(ResourceBoundsMapping::from(&v3.resource_bounds)),
                    tip: Some(ProtoFelt::from(Felt::from(u64::from(v3.tip)))),
                    paymaster_data: v3.paymaster_data.to_proto_felts(),
                    nonce_data_availability_mode: da_mode_to_string(
                        v3.nonce_data_availability_mode,
                    ),
                    fee_data_availability_mode: da_mode_to_string(v3.fee_data_availability_mode),
                }),
            },
            RpcTx::L1Handler(l1) => {
                // Use dedicated L1HandlerTxn type
                ProtoTxVariant::L1Handler(L1HandlerTxn {
                    r#type: "L1_HANDLER".to_string(),
                    version: format!("{:#x}", l1.version),
                    nonce: Some(l1.nonce.into()),
                    contract_address: Some(ProtoFelt::from(Felt::from(l1.contract_address))),
                    entry_point_selector: Some(l1.entry_point_selector.into()),
                    calldata: l1.calldata.to_proto_felts(),
                })
            }
            RpcTx::Deploy(deploy) => {
                // Use dedicated DeployTxn type for legacy deploy transactions
                ProtoTxVariant::Deploy(DeployTxn {
                    r#type: "DEPLOY".to_string(),
                    version: format!("{:#x}", deploy.version),
                    class_hash: Some(deploy.class_hash.into()),
                    contract_address_salt: Some(deploy.contract_address_salt.into()),
                    constructor_calldata: deploy.constructor_calldata.to_proto_felts(),
                })
            }
        };

        ProtoTx { transaction: Some(transaction) }
    }
}

impl From<&katana_primitives::fee::ResourceBoundsMapping> for ResourceBoundsMapping {
    fn from(bounds: &katana_primitives::fee::ResourceBoundsMapping) -> Self {
        use katana_primitives::fee::ResourceBoundsMapping as RpcResourceBoundsMapping;

        match bounds {
            RpcResourceBoundsMapping::L1Gas(l1_gas_bounds) => ResourceBoundsMapping {
                l1_gas: Some(ResourceBounds {
                    max_amount: Some(ProtoFelt::from(Felt::from(l1_gas_bounds.l1_gas.max_amount))),
                    max_price_per_unit: Some(ProtoFelt::from(Felt::from(
                        l1_gas_bounds.l1_gas.max_price_per_unit,
                    ))),
                }),
                l2_gas: Some(ResourceBounds {
                    max_amount: Some(ProtoFelt::from(Felt::from(l1_gas_bounds.l2_gas.max_amount))),
                    max_price_per_unit: Some(ProtoFelt::from(Felt::from(
                        l1_gas_bounds.l2_gas.max_price_per_unit,
                    ))),
                }),
                l1_data_gas: None, // L1Gas variant doesn't have l1_data_gas
            },
            RpcResourceBoundsMapping::All(all_bounds) => ResourceBoundsMapping {
                l1_gas: Some(ResourceBounds {
                    max_amount: Some(ProtoFelt::from(Felt::from(all_bounds.l1_gas.max_amount))),
                    max_price_per_unit: Some(ProtoFelt::from(Felt::from(
                        all_bounds.l1_gas.max_price_per_unit,
                    ))),
                }),
                l2_gas: Some(ResourceBounds {
                    max_amount: Some(ProtoFelt::from(Felt::from(all_bounds.l2_gas.max_amount))),
                    max_price_per_unit: Some(ProtoFelt::from(Felt::from(
                        all_bounds.l2_gas.max_price_per_unit,
                    ))),
                }),
                l1_data_gas: Some(ResourceBounds {
                    max_amount: Some(ProtoFelt::from(Felt::from(
                        all_bounds.l1_data_gas.max_amount,
                    ))),
                    max_price_per_unit: Some(ProtoFelt::from(Felt::from(
                        all_bounds.l1_data_gas.max_price_per_unit,
                    ))),
                }),
            },
        }
    }
}
