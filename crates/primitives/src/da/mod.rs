pub mod blob;
pub mod eip4844;
pub mod encoding;
pub mod math;
pub mod serde;

/// L1 da mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "arbitrary", derive(::arbitrary::Arbitrary))]
#[cfg_attr(feature = "serde", derive(::serde::Serialize, ::serde::Deserialize))]
pub enum L1DataAvailabilityMode {
    #[serde(rename = "BLOB")]
    Blob,
    #[serde(rename = "CALLDATA")]
    Calldata,
}

/// Data availability mode.
///
/// Specifies a storage domain in Starknet. Each domain has different guarantees regarding
/// availability
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "arbitrary", derive(::arbitrary::Arbitrary))]
#[cfg_attr(feature = "serde", derive(::serde::Serialize, ::serde::Deserialize))]
pub enum DataAvailabilityMode {
    #[serde(rename = "L1")]
    L1,
    #[serde(rename = "L2")]
    L2,
}
