//! Example demonstrating the Versioned derive macro
//! 
//! This shows how to use the macro to automatically generate versioned types
//! instead of writing all the boilerplate manually.

use katana_db_versioned_derive::Versioned;
use katana_primitives::{chain, class, contract, da, fee, transaction, Felt};
use serde::{Deserialize, Serialize};

// Define version-specific types that differ from current primitives
pub mod v6 {
    use super::*;
    
    /// V6 version of ResourceBoundsMapping - only had L1 and L2 gas
    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    #[cfg_attr(test, derive(::arbitrary::Arbitrary))]
    pub struct ResourceBoundsMapping {
        pub l1_gas: fee::ResourceBounds,
        pub l2_gas: fee::ResourceBounds,
    }
    
    // User provides the conversion logic
    impl From<ResourceBoundsMapping> for fee::ResourceBoundsMapping {
        fn from(v6: ResourceBoundsMapping) -> Self {
            // Convert v6 format to current format
            fee::ResourceBoundsMapping::L1Gas(fee::L1GasResourceBoundsMapping {
                l1_gas: v6.l1_gas,
                l2_gas: v6.l2_gas,
            })
        }
    }
}

pub mod v7 {
    use super::*;
    
    /// V7 version of ResourceBoundsMapping - added enum variants
    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    #[cfg_attr(test, derive(::arbitrary::Arbitrary))]
    pub enum ResourceBoundsMapping {
        L1Gas(fee::ResourceBounds),
        All(fee::AllResourceBoundsMapping),
    }
    
    impl From<ResourceBoundsMapping> for fee::ResourceBoundsMapping {
        fn from(v7: ResourceBoundsMapping) -> Self {
            match v7 {
                ResourceBoundsMapping::L1Gas(bounds) => {
                    fee::ResourceBoundsMapping::L1Gas(fee::L1GasResourceBoundsMapping {
                        l1_gas: bounds,
                        l2_gas: fee::ResourceBounds::default(),
                    })
                }
                ResourceBoundsMapping::All(all) => {
                    fee::ResourceBoundsMapping::All(all)
                }
            }
        }
    }
}

// Now use the macro to define a versioned struct
// The macro will generate v6 and v7 modules with appropriate structs
#[derive(Versioned)]
#[versioned(current = "katana_primitives::transaction")]
pub struct InvokeTxV3 {
    pub chain_id: chain::ChainId,
    pub sender_address: contract::ContractAddress,
    pub nonce: Felt,
    pub calldata: Vec<Felt>,
    pub signature: Vec<Felt>,
    
    // This field has different types in different versions
    #[versioned(
        v6 = "v6::ResourceBoundsMapping",
        v7 = "v7::ResourceBoundsMapping"
    )]
    pub resource_bounds: fee::ResourceBoundsMapping,
    
    pub tip: u64,
    pub paymaster_data: Vec<Felt>,
    pub account_deployment_data: Vec<Felt>,
    pub nonce_data_availability_mode: da::DataAvailabilityMode,
    pub fee_data_availability_mode: da::DataAvailabilityMode,
}

// The macro also works for enums
#[derive(Versioned)]
#[versioned(current = "katana_primitives::transaction")]
pub enum InvokeTx {
    V0(transaction::InvokeTxV0),
    V1(transaction::InvokeTxV1),
    V3(InvokeTxV3),
}

// Example showing field additions
#[derive(Versioned)]
pub struct SimpleStruct {
    pub field1: String,
    pub field2: u32,
    
    // Field with version-specific types
    #[versioned(
        v6 = "v6::ResourceBoundsMapping"
    )]
    pub bounds: fee::ResourceBoundsMapping,
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_versioned_struct() {
        // Create instance using current types
        let tx = InvokeTxV3 {
            chain_id: chain::ChainId::default(),
            sender_address: contract::ContractAddress::default(),
            nonce: Felt::ZERO,
            calldata: vec![],
            signature: vec![],
            resource_bounds: fee::ResourceBoundsMapping::All(Default::default()),
            tip: 0,
            paymaster_data: vec![],
            account_deployment_data: vec![],
            nonce_data_availability_mode: da::DataAvailabilityMode::L1,
            fee_data_availability_mode: da::DataAvailabilityMode::L1,
        };
        
        assert_eq!(tx.tip, 0);
    }
    
    #[test]
    fn test_versioned_enum() {
        let invoke = InvokeTx::V0(transaction::InvokeTxV0::default());
        
        match invoke {
            InvokeTx::V0(_) => assert!(true),
            _ => panic!("Wrong variant"),
        }
    }
}