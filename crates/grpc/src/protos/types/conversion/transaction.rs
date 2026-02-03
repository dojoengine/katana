//! Transaction type conversions.

use std::sync::Arc;

use cairo_lang_starknet_classes::contract_class::{ContractEntryPoint, ContractEntryPoints};
use katana_primitives::da::DataAvailabilityMode;
use katana_primitives::fee::{
    AllResourceBoundsMapping, L1GasResourceBoundsMapping,
    ResourceBounds as PrimitiveResourceBounds,
    ResourceBoundsMapping as PrimitiveResourceBoundsMapping,
};
use katana_primitives::Felt;
use katana_rpc_types::broadcasted::{
    BroadcastedDeclareTx, BroadcastedDeployAccountTx, BroadcastedInvokeTx, QUERY_VERSION_OFFSET,
};
use katana_rpc_types::class::RpcSierraContractClass;
use tonic::Status;

use super::{FeltVecExt, ProtoFeltVecExt};
use crate::proto;
use crate::proto::{
    BroadcastedDeclareTransaction, BroadcastedDeployAccountTransaction,
    BroadcastedInvokeTransaction, ContractClass as ProtoContractClass, DeployAccountTxn,
    DeployAccountTxnV3, DeployTxn, Felt as ProtoFelt, InvokeTxnV1, InvokeTxnV3, L1HandlerTxn,
    ResourceBounds, ResourceBoundsMapping, SierraEntryPoint as ProtoSierraEntryPoint,
    Transaction as ProtoTx,
};
use crate::protos::types::transaction::Transaction as ProtoTxVariant;

/// Convert DataAvailabilityMode to string representation for proto.
fn da_mode_to_string(mode: DataAvailabilityMode) -> String {
    match mode {
        DataAvailabilityMode::L1 => "L1".to_string(),
        DataAvailabilityMode::L2 => "L2".to_string(),
    }
}

/// Parse DataAvailabilityMode from string.
fn da_mode_from_string(s: &str) -> Result<DataAvailabilityMode, Status> {
    match s.to_uppercase().as_str() {
        "L1" => Ok(DataAvailabilityMode::L1),
        "L2" => Ok(DataAvailabilityMode::L2),
        _ => Err(Status::invalid_argument(format!("invalid data availability mode: {s}"))),
    }
}

/// Helper to extract a required Felt field from an Option.
fn required_felt(field: Option<&ProtoFelt>, field_name: &str) -> Result<Felt, Status> {
    field
        .ok_or_else(|| Status::invalid_argument(format!("missing required field: {field_name}")))?
        .try_into()
}

/// Derive is_query flag from version field.
fn is_query_from_version(version: Felt) -> Result<bool, Status> {
    if version == Felt::THREE {
        Ok(false)
    } else if version == Felt::THREE + QUERY_VERSION_OFFSET {
        Ok(true)
    } else {
        Err(Status::invalid_argument(format!(
            "invalid version {version:#x} for broadcasted transaction"
        )))
    }
}

/// Compute version from is_query flag.
fn version_from_is_query(is_query: bool) -> Felt {
    if is_query {
        Felt::THREE + QUERY_VERSION_OFFSET
    } else {
        Felt::THREE
    }
}

// ============================================================
// Proto -> RPC Type Conversions
// ============================================================

/// Convert proto ResourceBoundsMapping to primitives ResourceBoundsMapping.
impl TryFrom<&proto::ResourceBoundsMapping> for PrimitiveResourceBoundsMapping {
    type Error = Status;

    fn try_from(proto: &proto::ResourceBoundsMapping) -> Result<Self, Self::Error> {
        let l1_gas = proto
            .l1_gas
            .as_ref()
            .map(|rb| -> Result<PrimitiveResourceBounds, Status> {
                Ok(PrimitiveResourceBounds {
                    max_amount: required_felt(rb.max_amount.as_ref(), "l1_gas.max_amount")?
                        .try_into()
                        .map_err(|_| Status::invalid_argument("l1_gas.max_amount overflow"))?,
                    max_price_per_unit: required_felt(
                        rb.max_price_per_unit.as_ref(),
                        "l1_gas.max_price_per_unit",
                    )?
                    .try_into()
                    .map_err(|_| Status::invalid_argument("l1_gas.max_price_per_unit overflow"))?,
                })
            })
            .transpose()?
            .unwrap_or_default();

        let l2_gas = proto
            .l2_gas
            .as_ref()
            .map(|rb| -> Result<PrimitiveResourceBounds, Status> {
                Ok(PrimitiveResourceBounds {
                    max_amount: required_felt(rb.max_amount.as_ref(), "l2_gas.max_amount")?
                        .try_into()
                        .map_err(|_| Status::invalid_argument("l2_gas.max_amount overflow"))?,
                    max_price_per_unit: required_felt(
                        rb.max_price_per_unit.as_ref(),
                        "l2_gas.max_price_per_unit",
                    )?
                    .try_into()
                    .map_err(|_| Status::invalid_argument("l2_gas.max_price_per_unit overflow"))?,
                })
            })
            .transpose()?
            .unwrap_or_default();

        // Check if l1_data_gas is present to determine the mapping type
        if let Some(l1_data_gas_proto) = &proto.l1_data_gas {
            let l1_data_gas = PrimitiveResourceBounds {
                max_amount: required_felt(
                    l1_data_gas_proto.max_amount.as_ref(),
                    "l1_data_gas.max_amount",
                )?
                .try_into()
                .map_err(|_| Status::invalid_argument("l1_data_gas.max_amount overflow"))?,
                max_price_per_unit: required_felt(
                    l1_data_gas_proto.max_price_per_unit.as_ref(),
                    "l1_data_gas.max_price_per_unit",
                )?
                .try_into()
                .map_err(|_| Status::invalid_argument("l1_data_gas.max_price_per_unit overflow"))?,
            };

            Ok(PrimitiveResourceBoundsMapping::All(AllResourceBoundsMapping {
                l1_gas,
                l2_gas,
                l1_data_gas,
            }))
        } else {
            Ok(PrimitiveResourceBoundsMapping::L1Gas(L1GasResourceBoundsMapping { l1_gas, l2_gas }))
        }
    }
}

/// Convert proto BroadcastedInvokeTransaction to RPC BroadcastedInvokeTx.
impl TryFrom<BroadcastedInvokeTransaction> for BroadcastedInvokeTx {
    type Error = Status;

    fn try_from(proto: BroadcastedInvokeTransaction) -> Result<Self, Self::Error> {
        let sender_address = required_felt(proto.sender_address.as_ref(), "sender_address")?;
        let nonce = required_felt(proto.nonce.as_ref(), "nonce")?;
        let version = required_felt(proto.version.as_ref(), "version")?;
        let is_query = is_query_from_version(version)?;
        let tip = proto.tip.as_ref().map(Felt::try_from).transpose()?.unwrap_or(Felt::ZERO);

        let resource_bounds = proto
            .resource_bounds
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("missing required field: resource_bounds"))?;

        Ok(BroadcastedInvokeTx {
            sender_address: sender_address.into(),
            calldata: proto.calldata.to_felts()?,
            signature: proto.signature.to_felts()?,
            nonce,
            paymaster_data: proto.paymaster_data.to_felts()?,
            tip: u64::try_from(tip).map_err(|_| Status::invalid_argument("tip overflow"))?.into(),
            account_deployment_data: proto.account_deployment_data.to_felts()?,
            resource_bounds: PrimitiveResourceBoundsMapping::try_from(resource_bounds)?,
            fee_data_availability_mode: da_mode_from_string(&proto.fee_data_availability_mode)?,
            nonce_data_availability_mode: da_mode_from_string(&proto.nonce_data_availability_mode)?,
            is_query,
        })
    }
}

/// Convert proto BroadcastedDeployAccountTransaction to RPC BroadcastedDeployAccountTx.
impl TryFrom<BroadcastedDeployAccountTransaction> for BroadcastedDeployAccountTx {
    type Error = Status;

    fn try_from(proto: BroadcastedDeployAccountTransaction) -> Result<Self, Self::Error> {
        let nonce = required_felt(proto.nonce.as_ref(), "nonce")?;
        let class_hash = required_felt(proto.class_hash.as_ref(), "class_hash")?;
        let contract_address_salt =
            required_felt(proto.contract_address_salt.as_ref(), "contract_address_salt")?;
        let version = required_felt(proto.version.as_ref(), "version")?;
        let is_query = is_query_from_version(version)?;
        let tip = proto.tip.as_ref().map(Felt::try_from).transpose()?.unwrap_or(Felt::ZERO);

        let resource_bounds = proto
            .resource_bounds
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("missing required field: resource_bounds"))?;

        Ok(BroadcastedDeployAccountTx {
            signature: proto.signature.to_felts()?,
            nonce,
            contract_address_salt,
            constructor_calldata: proto.constructor_calldata.to_felts()?,
            class_hash,
            paymaster_data: proto.paymaster_data.to_felts()?,
            tip: u64::try_from(tip).map_err(|_| Status::invalid_argument("tip overflow"))?.into(),
            resource_bounds: PrimitiveResourceBoundsMapping::try_from(resource_bounds)?,
            fee_data_availability_mode: da_mode_from_string(&proto.fee_data_availability_mode)?,
            nonce_data_availability_mode: da_mode_from_string(&proto.nonce_data_availability_mode)?,
            is_query,
        })
    }
}

/// Convert proto ContractClass to RPC RpcSierraContractClass.
impl TryFrom<&ProtoContractClass> for RpcSierraContractClass {
    type Error = Status;

    fn try_from(proto: &ProtoContractClass) -> Result<Self, Self::Error> {
        let sierra_program = proto.sierra_program.to_felts()?;

        let entry_points = proto
            .entry_points_by_type
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("missing entry_points_by_type"))?;

        let convert_entry_points =
            |eps: &[ProtoSierraEntryPoint]| -> Result<Vec<ContractEntryPoint>, Status> {
                eps.iter()
                    .map(|ep| {
                        let selector = required_felt(ep.selector.as_ref(), "entry_point.selector")?;
                        // Convert Felt to BigUint via bytes
                        let selector_bytes = selector.to_bytes_be();
                        let selector_biguint = num_bigint::BigUint::from_bytes_be(&selector_bytes);
                        Ok(ContractEntryPoint {
                            selector: selector_biguint,
                            function_idx: ep.function_idx as usize,
                        })
                    })
                    .collect()
            };

        let entry_points_by_type = ContractEntryPoints {
            external: convert_entry_points(&entry_points.external)?,
            l1_handler: convert_entry_points(&entry_points.l1_handler)?,
            constructor: convert_entry_points(&entry_points.constructor)?,
        };

        Ok(RpcSierraContractClass {
            sierra_program,
            contract_class_version: proto.contract_class_version.clone(),
            entry_points_by_type,
            abi: if proto.abi.is_empty() { None } else { Some(proto.abi.clone()) },
        })
    }
}

/// Convert proto BroadcastedDeclareTransaction to RPC BroadcastedDeclareTx.
impl TryFrom<BroadcastedDeclareTransaction> for BroadcastedDeclareTx {
    type Error = Status;

    fn try_from(proto: BroadcastedDeclareTransaction) -> Result<Self, Self::Error> {
        let sender_address = required_felt(proto.sender_address.as_ref(), "sender_address")?;
        let nonce = required_felt(proto.nonce.as_ref(), "nonce")?;
        let compiled_class_hash =
            required_felt(proto.compiled_class_hash.as_ref(), "compiled_class_hash")?;
        let version = required_felt(proto.version.as_ref(), "version")?;
        let is_query = is_query_from_version(version)?;
        let tip = proto.tip.as_ref().map(Felt::try_from).transpose()?.unwrap_or(Felt::ZERO);

        let resource_bounds = proto
            .resource_bounds
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("missing required field: resource_bounds"))?;

        let contract_class = proto
            .contract_class
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("missing required field: contract_class"))?;

        let rpc_contract_class = RpcSierraContractClass::try_from(contract_class)?;

        Ok(BroadcastedDeclareTx {
            sender_address: sender_address.into(),
            compiled_class_hash,
            signature: proto.signature.to_felts()?,
            nonce,
            contract_class: Arc::new(rpc_contract_class),
            paymaster_data: proto.paymaster_data.to_felts()?,
            tip: u64::try_from(tip).map_err(|_| Status::invalid_argument("tip overflow"))?.into(),
            account_deployment_data: proto.account_deployment_data.to_felts()?,
            resource_bounds: PrimitiveResourceBoundsMapping::try_from(resource_bounds)?,
            fee_data_availability_mode: da_mode_from_string(&proto.fee_data_availability_mode)?,
            nonce_data_availability_mode: da_mode_from_string(&proto.nonce_data_availability_mode)?,
            is_query,
        })
    }
}

// ============================================================
// RPC Type -> Proto Conversions
// ============================================================

/// Convert RPC BroadcastedInvokeTx to proto BroadcastedInvokeTransaction.
impl From<&BroadcastedInvokeTx> for BroadcastedInvokeTransaction {
    fn from(tx: &BroadcastedInvokeTx) -> Self {
        BroadcastedInvokeTransaction {
            sender_address: Some(ProtoFelt::from(Felt::from(tx.sender_address))),
            calldata: tx.calldata.to_proto_felts(),
            signature: tx.signature.to_proto_felts(),
            nonce: Some(tx.nonce.into()),
            paymaster_data: tx.paymaster_data.to_proto_felts(),
            tip: Some(ProtoFelt::from(Felt::from(u64::from(tx.tip)))),
            account_deployment_data: tx.account_deployment_data.to_proto_felts(),
            resource_bounds: Some(ResourceBoundsMapping::from(&tx.resource_bounds)),
            fee_data_availability_mode: da_mode_to_string(tx.fee_data_availability_mode),
            nonce_data_availability_mode: da_mode_to_string(tx.nonce_data_availability_mode),
            version: Some(version_from_is_query(tx.is_query).into()),
        }
    }
}

/// Convert RPC BroadcastedDeployAccountTx to proto BroadcastedDeployAccountTransaction.
impl From<&BroadcastedDeployAccountTx> for BroadcastedDeployAccountTransaction {
    fn from(tx: &BroadcastedDeployAccountTx) -> Self {
        BroadcastedDeployAccountTransaction {
            signature: tx.signature.to_proto_felts(),
            nonce: Some(tx.nonce.into()),
            contract_address_salt: Some(tx.contract_address_salt.into()),
            constructor_calldata: tx.constructor_calldata.to_proto_felts(),
            class_hash: Some(tx.class_hash.into()),
            paymaster_data: tx.paymaster_data.to_proto_felts(),
            tip: Some(ProtoFelt::from(Felt::from(u64::from(tx.tip)))),
            resource_bounds: Some(ResourceBoundsMapping::from(&tx.resource_bounds)),
            fee_data_availability_mode: da_mode_to_string(tx.fee_data_availability_mode),
            nonce_data_availability_mode: da_mode_to_string(tx.nonce_data_availability_mode),
            version: Some(version_from_is_query(tx.is_query).into()),
        }
    }
}

/// Convert RPC BroadcastedDeclareTx to proto BroadcastedDeclareTransaction.
impl From<&BroadcastedDeclareTx> for BroadcastedDeclareTransaction {
    fn from(tx: &BroadcastedDeclareTx) -> Self {
        // Convert RpcSierraContractClass to proto ContractClass
        let contract_class = ProtoContractClass {
            sierra_program: tx.contract_class.sierra_program.to_proto_felts(),
            contract_class_version: tx.contract_class.contract_class_version.clone(),
            entry_points_by_type: Some(proto::EntryPointsByType {
                constructor: tx
                    .contract_class
                    .entry_points_by_type
                    .constructor
                    .iter()
                    .map(|ep| ProtoSierraEntryPoint {
                        selector: Some(ProtoFelt::from(Felt::from_bytes_be(
                            &ep.selector.to_bytes_be().try_into().unwrap_or([0u8; 32]),
                        ))),
                        function_idx: ep.function_idx as u64,
                    })
                    .collect(),
                external: tx
                    .contract_class
                    .entry_points_by_type
                    .external
                    .iter()
                    .map(|ep| ProtoSierraEntryPoint {
                        selector: Some(ProtoFelt::from(Felt::from_bytes_be(
                            &ep.selector.to_bytes_be().try_into().unwrap_or([0u8; 32]),
                        ))),
                        function_idx: ep.function_idx as u64,
                    })
                    .collect(),
                l1_handler: tx
                    .contract_class
                    .entry_points_by_type
                    .l1_handler
                    .iter()
                    .map(|ep| ProtoSierraEntryPoint {
                        selector: Some(ProtoFelt::from(Felt::from_bytes_be(
                            &ep.selector.to_bytes_be().try_into().unwrap_or([0u8; 32]),
                        ))),
                        function_idx: ep.function_idx as u64,
                    })
                    .collect(),
            }),
            abi: tx.contract_class.abi.clone().unwrap_or_default(),
        };

        BroadcastedDeclareTransaction {
            sender_address: Some(ProtoFelt::from(Felt::from(tx.sender_address))),
            compiled_class_hash: Some(tx.compiled_class_hash.into()),
            signature: tx.signature.to_proto_felts(),
            nonce: Some(tx.nonce.into()),
            contract_class: Some(contract_class),
            paymaster_data: tx.paymaster_data.to_proto_felts(),
            tip: Some(ProtoFelt::from(Felt::from(u64::from(tx.tip)))),
            account_deployment_data: tx.account_deployment_data.to_proto_felts(),
            resource_bounds: Some(ResourceBoundsMapping::from(&tx.resource_bounds)),
            fee_data_availability_mode: da_mode_to_string(tx.fee_data_availability_mode),
            nonce_data_availability_mode: da_mode_to_string(tx.nonce_data_availability_mode),
            version: Some(version_from_is_query(tx.is_query).into()),
        }
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

impl From<&PrimitiveResourceBoundsMapping> for ResourceBoundsMapping {
    fn from(bounds: &PrimitiveResourceBoundsMapping) -> Self {
        match bounds {
            PrimitiveResourceBoundsMapping::L1Gas(l1_gas_bounds) => ResourceBoundsMapping {
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
            PrimitiveResourceBoundsMapping::All(all_bounds) => ResourceBoundsMapping {
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
