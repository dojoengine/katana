use katana_primitives::block::{BlockHash, BlockNumber, GasPrice, Header};
use katana_primitives::contract::ContractAddress;
use katana_primitives::da::L1DataAvailabilityMode;
use katana_primitives::version::ProtocolVersion;
use katana_primitives::Felt;
use serde::{Deserialize, Serialize};

use crate::codecs::{Compress, Decompress};
use crate::error::CodecError;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum VersionedHeader {
    V5(HeaderV5),
    V6(HeaderV6),
    V7(Header),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeaderV5 {
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
    pub l1_gas_prices: GasPrice,
    pub l1_data_gas_prices: GasPrice,
    pub l2_gas_prices: GasPrice,
    pub l1_da_mode: L1DataAvailabilityMode,
    pub protocol_version: ProtocolVersion,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeaderV6 {
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
    pub l1_gas_prices: GasPrice,
    pub l1_data_gas_prices: GasPrice,
    pub l2_gas_prices: GasPrice,
    pub l1_da_mode: L1DataAvailabilityMode,
    pub protocol_version: ProtocolVersion,
}

impl From<HeaderV5> for Header {
    fn from(header: HeaderV5) -> Self {
        Header {
            parent_hash: header.parent_hash,
            number: header.number,
            state_diff_commitment: header.state_diff_commitment,
            transactions_commitment: header.transactions_commitment,
            receipts_commitment: header.receipts_commitment,
            events_commitment: header.events_commitment,
            state_root: header.state_root,
            transaction_count: header.transaction_count,
            events_count: header.events_count,
            state_diff_length: header.state_diff_length,
            timestamp: header.timestamp,
            sequencer_address: header.sequencer_address,
            l1_gas_prices: header.l1_gas_prices,
            l1_data_gas_prices: header.l1_data_gas_prices,
            l2_gas_prices: header.l2_gas_prices,
            l1_da_mode: header.l1_da_mode,
            protocol_version: header.protocol_version,
        }
    }
}

impl From<HeaderV6> for Header {
    fn from(header: HeaderV6) -> Self {
        Header {
            parent_hash: header.parent_hash,
            number: header.number,
            state_diff_commitment: header.state_diff_commitment,
            transactions_commitment: header.transactions_commitment,
            receipts_commitment: header.receipts_commitment,
            events_commitment: header.events_commitment,
            state_root: header.state_root,
            transaction_count: header.transaction_count,
            events_count: header.events_count,
            state_diff_length: header.state_diff_length,
            timestamp: header.timestamp,
            sequencer_address: header.sequencer_address,
            l1_gas_prices: header.l1_gas_prices,
            l1_data_gas_prices: header.l1_data_gas_prices,
            l2_gas_prices: header.l2_gas_prices,
            l1_da_mode: header.l1_da_mode,
            protocol_version: header.protocol_version,
        }
    }
}

impl From<VersionedHeader> for Header {
    fn from(versioned: VersionedHeader) -> Self {
        match versioned {
            VersionedHeader::V7(header) => header,
            VersionedHeader::V6(header) => header.into(),
            VersionedHeader::V5(header) => header.into(),
        }
    }
}

impl Compress for VersionedHeader {
    type Compressed = Vec<u8>;
    fn compress(self) -> Result<Self::Compressed, CodecError> {
        postcard::to_stdvec(&self).map_err(|e| CodecError::Compress(e.to_string()))
    }
}

impl Decompress for VersionedHeader {
    fn decompress<B: AsRef<[u8]>>(bytes: B) -> Result<Self, CodecError> {
        if let Ok(header) = postcard::from_bytes::<Header>(bytes.as_ref()) {
            return Ok(VersionedHeader::V7(header));
        }

        if let Ok(header) = postcard::from_bytes::<HeaderV6>(bytes.as_ref()) {
            return Ok(VersionedHeader::V6(header));
        }

        if let Ok(header) = postcard::from_bytes::<HeaderV5>(bytes.as_ref()) {
            return Ok(VersionedHeader::V5(header));
        }

        postcard::from_bytes::<VersionedHeader>(bytes.as_ref())
            .map_err(|e| CodecError::Decompress(e.to_string()))
    }
}
