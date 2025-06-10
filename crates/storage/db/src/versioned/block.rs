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
    V6(HeaderV6),
    V7(Header),
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
    pub l1_da_mode: L1DataAvailabilityMode,
    pub protocol_version: ProtocolVersion,
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
            l1_da_mode: header.l1_da_mode,
            protocol_version: header.protocol_version,
            l2_gas_prices: unsafe { GasPrice::zero() },
        }
    }
}

impl From<Header> for VersionedHeader {
    fn from(header: Header) -> Self {
        Self::V7(header)
    }
}

impl From<VersionedHeader> for Header {
    fn from(versioned: VersionedHeader) -> Self {
        match versioned {
            VersionedHeader::V7(header) => header,
            VersionedHeader::V6(header) => header.into(),
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
        let bytes = bytes.as_ref();

        if let Ok(header) = postcard::from_bytes::<Self>(bytes) {
            return Ok(header);
        }

        // Helper for deserializing from older formats that may be serialized untagged
        #[derive(Debug, Deserialize)]
        #[serde(untagged)]
        enum UntaggedVersionedHeader {
            V6(HeaderV6),
            V7(Header),
        }

        impl From<UntaggedVersionedHeader> for VersionedHeader {
            fn from(header: UntaggedVersionedHeader) -> Self {
                match header {
                    UntaggedVersionedHeader::V6(header) => Self::V6(header),
                    UntaggedVersionedHeader::V7(header) => Self::V7(header),
                }
            }
        }

        postcard::from_bytes::<UntaggedVersionedHeader>(bytes)
            .map(Self::from)
            .map_err(|e| CodecError::Decompress(e.to_string()))
    }
}
