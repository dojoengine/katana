use std::fmt::{Display, LowerHex};
use std::num::NonZeroU128;
use std::str::FromStr;

use starknet::core::utils::cairo_short_string_to_felt;
use starknet::macros::short_string;

use crate::contract::ContractAddress;
use crate::da::L1DataAvailabilityMode;
use crate::transaction::{ExecutableTxWithHash, TxHash, TxWithHash};
use crate::version::StarknetVersion;
use crate::Felt;

pub type BlockIdOrTag = starknet::core::types::BlockId;
pub type BlockTag = starknet::core::types::BlockTag;

/// Block number type.
pub type BlockNumber = u64;
/// Block hash type.
pub type BlockHash = Felt;

#[derive(Debug, Copy, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum BlockHashOrNumber {
    Hash(BlockHash),
    Num(BlockNumber),
}

impl std::fmt::Display for BlockHashOrNumber {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BlockHashOrNumber::Num(num) => write!(f, "{num}"),
            BlockHashOrNumber::Hash(hash) => write!(f, "{hash:#x}"),
        }
    }
}

/// Finality status of a canonical block.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "arbitrary", derive(::arbitrary::Arbitrary))]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum FinalityStatus {
    AcceptedOnL2,
    AcceptedOnL1,
}

/// Represents a partial block header.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "arbitrary", derive(::arbitrary::Arbitrary))]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PartialHeader {
    pub parent_hash: BlockHash,
    pub number: BlockNumber,
    pub timestamp: u64,
    pub sequencer_address: ContractAddress,
    pub l1_gas_prices: GasPrices,
    pub l1_data_gas_prices: GasPrices,
    pub l2_gas_prices: GasPrices,
    pub l1_da_mode: L1DataAvailabilityMode,
    pub starknet_version: StarknetVersion,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "arbitrary", derive(::arbitrary::Arbitrary))]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct GasPrice(NonZeroU128);

impl GasPrice {
    pub const MIN: Self = Self(NonZeroU128::MIN);
    pub const MAX: Self = Self(NonZeroU128::MAX);

    /// Creates a new `GasPrice` instance.
    pub const fn new(value: NonZeroU128) -> Self {
        Self(value)
    }

    /// Returns the value of the gas price.
    pub const fn get(&self) -> u128 {
        self.0.get()
    }

    /// Creates a zero gas price.
    ///
    /// # Safety
    ///
    /// This is primarily used for testing purposes.
    pub const unsafe fn zero() -> Self {
        Self::new_unchecked(0)
    }

    /// Creates a non-zero gas price without checking whether the value is non-zero.
    /// This may results in undefined behaviour if the value is zero.
    ///
    /// # Safety
    ///
    /// The caller must ensure that gas price is not zero.
    pub const unsafe fn new_unchecked(value: u128) -> Self {
        Self(NonZeroU128::new_unchecked(value))
    }
}

impl Display for GasPrice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:#x}", self.0)
    }
}

impl LowerHex for GasPrice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        <NonZeroU128 as LowerHex>::fmt(&self.0, f)
    }
}

#[derive(thiserror::Error, Debug)]
#[error("gas price cannot be zero")]
pub struct GasPriceIsZeroError;

impl TryFrom<u128> for GasPrice {
    type Error = GasPriceIsZeroError;

    fn try_from(value: u128) -> Result<Self, Self::Error> {
        match NonZeroU128::new(value) {
            Some(non_zero) => Ok(Self(non_zero)),
            None => Err(GasPriceIsZeroError),
        }
    }
}

impl FromStr for GasPrice {
    type Err = std::num::ParseIntError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        <NonZeroU128 as FromStr>::from_str(s).map(Self)
    }
}

// TODO: Make sure the values can't be zero because in the blockifier executor, we fallback to 1 if
// the gas price value is 0.
/// The L1 gas prices.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "arbitrary", derive(::arbitrary::Arbitrary))]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "UPPERCASE"))]
pub struct GasPrices {
    /// The price of one unit of the given resource, denominated in wei
    pub eth: GasPrice,
    /// The price of one unit of the given resource, denominated in fri (the smallest unit of STRK,
    /// equivalent to 10^-18 STRK)
    pub strk: GasPrice,
}

impl GasPrices {
    pub const MIN: Self = Self::new(GasPrice::MIN, GasPrice::MIN);
    pub const MAX: Self = Self::new(GasPrice::MAX, GasPrice::MAX);

    pub const fn new(eth: GasPrice, strk: GasPrice) -> Self {
        Self { eth, strk }
    }

    /// # Safety
    ///
    /// The caller must ensure that gas price is not zero.
    pub const unsafe fn new_unchecked(eth: u128, fri: u128) -> Self {
        Self::new(GasPrice::new_unchecked(eth), GasPrice::new_unchecked(fri))
    }
}

impl Default for GasPrices {
    fn default() -> Self {
        Self::MIN
    }
}

// uncommited header ->  header (what is stored in the database)

/// Represents a block header.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "arbitrary", derive(::arbitrary::Arbitrary))]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Header {
    pub parent_hash: BlockHash,
    pub number: BlockNumber,
    pub state_diff_commitment: Felt,
    pub transactions_commitment: Felt,
    pub receipts_commitment: Felt,
    pub events_commitment: Felt,
    pub state_root: Felt,
    pub transaction_count: u32,
    pub events_count: u32,
    pub state_diff_length: u32,
    pub timestamp: u64,
    pub sequencer_address: ContractAddress,
    pub l1_gas_prices: GasPrices,
    pub l1_data_gas_prices: GasPrices,
    pub l2_gas_prices: GasPrices,
    pub l1_da_mode: L1DataAvailabilityMode,
    pub starknet_version: StarknetVersion,
}

impl Header {
    /// Computes the block hash.
    ///
    /// A block hash is defined as the Poseidon hash of the header’s fields, as follows:
    ///
    /// h(𝐵) = h(
    ///     "STARKNET_BLOCK_HASH0",
    ///     block_number,
    ///     global_state_root,
    ///     sequencer_address,
    ///     block_timestamp,
    ///     transaction_count || event_count || state_diff_length || l1_da_mode,
    ///     state_diff_commitment,
    ///     transactions_commitment
    ///     events_commitment,
    ///     receipts_commitment
    ///     l1_gas_price_in_wei,
    ///     l1_gas_price_in_fri,
    ///     l1_data_gas_price_in_wei,
    ///     l1_data_gas_price_in_fri
    ///     protocol_version,
    ///     0,
    ///     parent_block_hash
    /// )
    ///
    /// Based on StarkWare's [Sequencer implementation].
    ///
    /// [sequencer implementation]: https://github.com/starkware-libs/sequencer/blob/bb361ec67396660d5468fd088171913e11482708/crates/starknet_api/src/block_hash/block_hash_calculator.rs#l62-l93
    pub fn compute_hash(&self) -> Felt {
        use starknet_types_core::hash::{Poseidon, StarkHash};

        let concant = Self::concat_counts(
            self.transaction_count,
            self.events_count,
            self.state_diff_length,
            self.l1_da_mode,
        );

        Poseidon::hash_array(&[
            short_string!("STARKNET_BLOCK_HASH0"),
            self.number.into(),
            self.state_root,
            self.sequencer_address.into(),
            self.timestamp.into(),
            concant,
            self.state_diff_commitment,
            self.transactions_commitment,
            self.events_commitment,
            self.receipts_commitment,
            self.l1_gas_prices.eth.get().into(),
            self.l1_gas_prices.strk.get().into(),
            self.l1_data_gas_prices.eth.get().into(),
            self.l1_data_gas_prices.strk.get().into(),
            cairo_short_string_to_felt(&self.starknet_version.to_string()).unwrap(),
            Felt::ZERO,
            self.parent_hash,
        ])
    }

    // Concantenate the transaction_count, event_count and state_diff_length, and l1_da_mode into a
    // single felt.
    //
    // A single felt:
    //
    // +-------------------+----------------+----------------------+--------------+------------+
    // | transaction_count | event_count    | state_diff_length    | L1 DA mode   | padding    |
    // | (64 bits)         | (64 bits)      | (64 bits)            | (1 bit)      | (63 bit)   |
    // +-------------------+----------------+----------------------+--------------+------------+
    //
    // where, L1 DA mode is 0 for calldata, and 1 for blob.
    //
    // Based on https://github.com/starkware-libs/sequencer/blob/bb361ec67396660d5468fd088171913e11482708/crates/starknet_api/src/block_hash/block_hash_calculator.rs#L135-L164
    fn concat_counts(
        transaction_count: u32,
        event_count: u32,
        state_diff_length: u32,
        l1_data_availability_mode: L1DataAvailabilityMode,
    ) -> Felt {
        fn to_64_bits(num: u32) -> [u8; 8] {
            (num as u64).to_be_bytes()
        }

        let l1_data_availability_byte: u8 = match l1_data_availability_mode {
            L1DataAvailabilityMode::Calldata => 0,
            L1DataAvailabilityMode::Blob => 0b_1000_0000,
        };

        let concat_bytes = [
            to_64_bits(transaction_count).as_slice(),
            to_64_bits(event_count).as_slice(),
            to_64_bits(state_diff_length).as_slice(),
            &[l1_data_availability_byte],
            &[0_u8; 7], // zero padding
        ]
        .concat();

        Felt::from_bytes_be_slice(concat_bytes.as_slice())
    }
}

impl Default for Header {
    fn default() -> Self {
        Self {
            timestamp: 0,
            events_count: 0,
            transaction_count: 0,
            state_diff_length: 0,
            state_root: Felt::ZERO,
            events_commitment: Felt::ZERO,
            number: BlockNumber::default(),
            receipts_commitment: Felt::ZERO,
            state_diff_commitment: Felt::ZERO,
            parent_hash: BlockHash::default(),
            l2_gas_prices: GasPrices::default(),
            l1_gas_prices: GasPrices::default(),
            transactions_commitment: Felt::ZERO,
            l1_data_gas_prices: GasPrices::default(),
            sequencer_address: ContractAddress::default(),
            l1_da_mode: L1DataAvailabilityMode::Calldata,
            starknet_version: StarknetVersion::default(),
        }
    }
}

/// Represents a Starknet full block.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Block {
    pub header: Header,
    pub body: Vec<TxWithHash>,
}

/// A block with only the transaction hashes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockWithTxHashes {
    pub header: Header,
    pub body: Vec<TxHash>,
}

impl Block {
    /// Seals the block. This computes the hash of the block.
    pub fn seal(self) -> SealedBlock {
        let hash = self.header.compute_hash();
        SealedBlock { hash, header: self.header, body: self.body }
    }

    /// Seals the block with a given hash.
    pub fn seal_with_hash(self, hash: BlockHash) -> SealedBlock {
        SealedBlock { hash, header: self.header, body: self.body }
    }

    /// Seals the block with a given block hash and status.
    pub fn seal_with_hash_and_status(
        self,
        hash: BlockHash,
        status: FinalityStatus,
    ) -> SealedBlockWithStatus {
        SealedBlockWithStatus { block: self.seal_with_hash(hash), status }
    }
}

/// A full Starknet block that has been sealed.
#[derive(Debug, Clone)]
pub struct SealedBlock {
    /// The block hash.
    pub hash: BlockHash,
    /// The block header.
    pub header: Header,
    /// The block transactions.
    pub body: Vec<TxWithHash>,
}

impl SealedBlock {
    /// Unseal the block.
    pub fn unseal(self) -> Block {
        Block { header: self.header, body: self.body }
    }
}

/// A sealed block along with its status.
///
/// Block whose commitment has been computed.
#[derive(Debug, Clone)]
pub struct SealedBlockWithStatus {
    pub block: SealedBlock,
    /// The block status.
    pub status: FinalityStatus,
}

impl From<BlockNumber> for BlockHashOrNumber {
    fn from(number: BlockNumber) -> Self {
        Self::Num(number)
    }
}

impl From<BlockHash> for BlockHashOrNumber {
    fn from(hash: BlockHash) -> Self {
        Self::Hash(hash)
    }
}

impl From<BlockHashOrNumber> for BlockIdOrTag {
    fn from(value: BlockHashOrNumber) -> Self {
        match value {
            BlockHashOrNumber::Hash(hash) => BlockIdOrTag::Hash(hash),
            BlockHashOrNumber::Num(number) => BlockIdOrTag::Number(number),
        }
    }
}

/// A block that can executed. This is a block whose transactions includes
/// all the necessary information to be executed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutableBlock {
    pub header: PartialHeader,
    pub body: Vec<ExecutableTxWithHash>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::felt;

    #[test]
    fn header_concat_counts() {
        let expected = felt!("0x6400000000000000c8000000000000012c0000000000000000");
        let actual = Header::concat_counts(100, 200, 300, L1DataAvailabilityMode::Calldata);
        assert_eq!(actual, expected);

        let expected = felt!("0x1000000000000000200000000000000038000000000000000");
        let actual = Header::concat_counts(1, 2, 3, L1DataAvailabilityMode::Blob);
        assert_eq!(actual, expected);

        let expected = felt!("0xffffffff000000000000000000000000000000000000000000000000");
        let actual = Header::concat_counts(0xFFFFFFFF, 0, 0, L1DataAvailabilityMode::Calldata);
        assert_eq!(actual, expected);
    }
}
