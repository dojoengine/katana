use std::sync::Arc;

use katana_primitives::chain::ChainId;
use katana_primitives::class::{ClassHash, CompiledClassHash, ContractClass, SierraContractClass};
use katana_primitives::contract::Nonce;
use katana_primitives::da::DataAvailabilityMode;
use katana_primitives::fee::ResourceBoundsMapping;
use katana_primitives::transaction::{
    DeclareTx, DeclareTxV3, DeclareTxWithClass, DeployAccountTx, DeployAccountTxV3, InvokeTx,
    InvokeTxV3,
};
use katana_primitives::{ContractAddress, Felt};
use serde::{Deserialize, Deserializer, Serialize};
use starknet::core::utils::get_contract_address;

use crate::class::RpcSierraContractClass;

const QUERY_VERSION_OFFSET: Felt =
    Felt::from_raw([576460752142434320, 18446744073709551584, 17407, 18446744073700081665]);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BroadcastedTx {
    Invoke(BroadcastedInvokeTx),
    Declare(BroadcastedDeclareTx),
    DeployAccount(BroadcastedDeployAccountTx),
}

#[derive(Debug, Clone)]
pub struct BroadcastedInvokeTx {
    pub version: Felt,
    pub sender_address: ContractAddress,
    pub calldata: Vec<Felt>,
    pub signature: Vec<Felt>,
    pub nonce: Felt,
    pub paymaster_data: Vec<Felt>,
    pub tip: u64,
    pub account_deployment_data: Vec<Felt>,
    pub resource_bounds: ResourceBoundsMapping,
    pub fee_data_availability_mode: DataAvailabilityMode,
    pub nonce_data_availability_mode: DataAvailabilityMode,
}

impl BroadcastedInvokeTx {
    pub fn is_query(&self) -> bool {
        todo!()
    }

    pub fn into_inner(self, chain_id: ChainId) -> InvokeTx {
        InvokeTx::V3(InvokeTxV3 {
            chain_id,
            tip: self.tip,
            nonce: self.nonce,
            calldata: self.calldata,
            signature: self.signature,
            sender_address: self.sender_address,
            paymaster_data: self.paymaster_data,
            resource_bounds: self.resource_bounds,
            account_deployment_data: self.account_deployment_data,
            fee_data_availability_mode: self.fee_data_availability_mode,
            nonce_data_availability_mode: self.nonce_data_availability_mode,
        })
    }
}

impl serde::Serialize for BroadcastedInvokeTx {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;

        let mut state = serializer.serialize_struct("BroadcastedInvokeTx", 11)?;
        state.serialize_field("type", "INVOKE")?;
        state.serialize_field("version", &self.version)?;
        state.serialize_field("sender_address", &self.sender_address)?;
        state.serialize_field("calldata", &self.calldata)?;
        state.serialize_field("signature", &self.signature)?;
        state.serialize_field("nonce", &self.nonce)?;
        state.serialize_field("paymaster_data", &self.paymaster_data)?;
        state.serialize_field("tip", &self.tip)?;
        state.serialize_field("account_deployment_data", &self.account_deployment_data)?;
        state.serialize_field("resource_bounds", &self.resource_bounds)?;
        state.serialize_field("fee_data_availability_mode", &self.fee_data_availability_mode)?;
        state
            .serialize_field("nonce_data_availability_mode", &self.nonce_data_availability_mode)?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for BroadcastedInvokeTx {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // "version": {
        //             "title": "Version",
        //             "description": "Version of the transaction scheme",
        //             "type": "string",
        //             "enum": ["0x3", "0x100000000000000000000000000000003"]
        //           },

        #[derive(Deserialize)]
        struct Tagged {
            r#type: Option<String>,
            sender_address: ContractAddress,
            calldata: Vec<Felt>,
            version: Felt,
            signature: Vec<Felt>,
            nonce: Felt,
            resource_bounds: ResourceBoundsMapping,
            tip: u64,
            paymaster_data: Vec<Felt>,
            account_deployment_data: Vec<Felt>,
            nonce_data_availability_mode: DataAvailabilityMode,
            fee_data_availability_mode: DataAvailabilityMode,
        }

        let tagged = Tagged::deserialize(deserializer)?;

        if let Some(tag_field) = &tagged.r#type {
            if tag_field != "INVOKE" {
                return Err(serde::de::Error::custom("invalid `type` value"));
            }
        }

        // let is_query = if tagged.version == Felt::THREE {
        //     false
        // } else if tagged.version == Felt::THREE + QUERY_VERSION_OFFSET {
        //     true
        // } else {
        //     return Err(serde::de::Error::custom("invalid `version` value"));
        // };

        Ok(Self {
            version: tagged.version,
            sender_address: tagged.sender_address,
            calldata: tagged.calldata,
            signature: tagged.signature,
            nonce: tagged.nonce,
            resource_bounds: tagged.resource_bounds,
            tip: tagged.tip,
            paymaster_data: tagged.paymaster_data,
            account_deployment_data: tagged.account_deployment_data,
            nonce_data_availability_mode: tagged.nonce_data_availability_mode,
            fee_data_availability_mode: tagged.fee_data_availability_mode,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BroadcastedDeclareTx {
    pub version: Felt,
    pub sender_address: ContractAddress,
    pub compiled_class_hash: CompiledClassHash,
    pub signature: Vec<Felt>,
    pub nonce: Nonce,
    pub contract_class: Arc<RpcSierraContractClass>,
    pub resource_bounds: ResourceBoundsMapping,
    pub tip: u64,
    pub paymaster_data: Vec<Felt>,
    pub account_deployment_data: Vec<Felt>,
    pub nonce_data_availability_mode: DataAvailabilityMode,
    pub fee_data_availability_mode: DataAvailabilityMode,
}

impl BroadcastedDeclareTx {
    pub fn is_query(&self) -> bool {
        todo!()
    }

    pub fn into_inner(self, chain_id: ChainId) -> DeclareTxWithClass {
        let class_hash = self.contract_class.hash();

        let rpc_class = Arc::unwrap_or_clone(self.contract_class);
        let class = ContractClass::Class(SierraContractClass::try_from(rpc_class).unwrap());

        let tx = DeclareTx::V3(DeclareTxV3 {
            chain_id,
            class_hash,
            tip: self.tip,
            nonce: self.nonce,
            signature: self.signature,
            paymaster_data: self.paymaster_data,
            sender_address: self.sender_address,
            resource_bounds: self.resource_bounds,
            compiled_class_hash: self.compiled_class_hash,
            account_deployment_data: self.account_deployment_data,
            fee_data_availability_mode: self.fee_data_availability_mode,
            nonce_data_availability_mode: self.nonce_data_availability_mode,
        });

        DeclareTxWithClass::new(tx, class)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BroadcastedDeployAccountTx {
    pub version: Felt,
    pub signature: Vec<Felt>,
    pub nonce: Nonce,
    pub contract_address_salt: Felt,
    pub constructor_calldata: Vec<Felt>,
    pub class_hash: ClassHash,
    pub resource_bounds: ResourceBoundsMapping,
    pub tip: u64,
    pub paymaster_data: Vec<Felt>,
    pub nonce_data_availability_mode: DataAvailabilityMode,
    pub fee_data_availability_mode: DataAvailabilityMode,
}

impl BroadcastedDeployAccountTx {
    pub fn is_query(&self) -> bool {
        todo!()
    }

    pub fn into_inner(self, chain_id: ChainId) -> DeployAccountTx {
        let contract_address = get_contract_address(
            self.contract_address_salt,
            self.class_hash,
            &self.constructor_calldata,
            Felt::ZERO,
        );

        DeployAccountTx::V3(DeployAccountTxV3 {
            chain_id,
            tip: self.tip,
            nonce: self.nonce,
            signature: self.signature,
            class_hash: self.class_hash,
            paymaster_data: self.paymaster_data,
            resource_bounds: self.resource_bounds,
            contract_address: contract_address.into(),
            constructor_calldata: self.constructor_calldata,
            contract_address_salt: self.contract_address_salt,
            fee_data_availability_mode: self.fee_data_availability_mode,
            nonce_data_availability_mode: self.nonce_data_availability_mode,
        })
    }
}
