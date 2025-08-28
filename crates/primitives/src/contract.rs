use std::fmt;
use std::str::FromStr;

use num_bigint::BigUint;
use starknet::core::utils::normalize_address;

use crate::class::ClassHash;
use crate::Felt;

/// Represents the type for a contract storage key.
pub type StorageKey = Felt;
/// Represents the type for a contract storage value.
pub type StorageValue = Felt;

/// Represents the type for a contract nonce.
pub type Nonce = Felt;

/// Represents a contract address.
#[derive(Default, Clone, Copy, Eq, PartialEq, PartialOrd, Ord, Hash, Debug)]
#[cfg_attr(feature = "arbitrary", derive(::arbitrary::Arbitrary))]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ContractAddress(pub Felt);

impl ContractAddress {
    pub const ZERO: Self = Self(Felt::ZERO);
    pub const ONE: Self = Self(Felt::ONE);
    pub const TWO: Self = Self(Felt::TWO);
    pub const THREE: Self = Self(Felt::THREE);

    pub fn new(address: Felt) -> Self {
        ContractAddress(normalize_address(address))
    }
}

impl core::ops::Deref for ContractAddress {
    type Target = Felt;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl fmt::Display for ContractAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:#x}", self.0)
    }
}

impl FromStr for ContractAddress {
    type Err = <Felt as FromStr>::Err;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Felt::from_str(s).map(ContractAddress::new)
    }
}

impl From<Felt> for ContractAddress {
    fn from(value: Felt) -> Self {
        ContractAddress::new(value)
    }
}

impl AsRef<Felt> for ContractAddress {
    fn as_ref(&self) -> &Felt {
        &self.0
    }
}

impl From<ContractAddress> for Felt {
    fn from(value: ContractAddress) -> Self {
        value.0
    }
}

impl From<&BigUint> for ContractAddress {
    fn from(biguint: &BigUint) -> Self {
        Self::new(Felt::from_bytes_le_slice(&biguint.to_bytes_le()))
    }
}

impl From<BigUint> for ContractAddress {
    fn from(biguint: BigUint) -> Self {
        Self::new(Felt::from_bytes_le_slice(&biguint.to_bytes_le()))
    }
}

impl cainome_cairo_serde::CairoSerde for ContractAddress {
    type RustType = Self;

    fn cairo_serialize(rust: &Self::RustType) -> Vec<Felt> {
        vec![rust.0]
    }

    fn cairo_deserialize(
        felts: &[Felt],
        offset: usize,
    ) -> cainome_cairo_serde::Result<Self::RustType> {
        Ok(Self::from(<Felt as cainome_cairo_serde::CairoSerde>::cairo_deserialize(felts, offset)?))
    }
}

#[macro_export]
macro_rules! address {
    ($value:expr) => {
        ContractAddress::new($crate::felt!($value))
    };
}

/// Represents a generic contract instance information.
#[derive(Debug, Copy, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "arbitrary", derive(::arbitrary::Arbitrary))]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct GenericContractInfo {
    /// The nonce of the contract.
    pub nonce: Nonce,
    /// The hash of the contract class.
    pub class_hash: ClassHash,
}

impl From<starknet_api::core::ContractAddress> for ContractAddress {
    fn from(value: starknet_api::core::ContractAddress) -> Self {
        // Had to do this workaround until we have this PR https://github.com/starkware-libs/cairo/pull/8285
        //
        // Before this we could simply do Felt::from(value).
        Self(Felt::from(value.0.key().to_biguint()))
    }
}

impl From<ContractAddress> for starknet_api::core::ContractAddress {
    fn from(value: ContractAddress) -> Self {
        // Had to do this workaround until we have this PR https://github.com/starkware-libs/cairo/pull/8285
        //
        // Before this we could simply do Felt::from(value.0).
        use starknet_api::hash::StarkHash;
        let sn_api_felt = StarkHash::from(value.to_biguint());
        Self::try_from(sn_api_felt).expect("valid address")
    }
}
