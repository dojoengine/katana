use katana_primitives::block::Header;
use katana_primitives::contract::{ContractAddress, GenericContractInfo};
use katana_primitives::receipt::Receipt;
use katana_primitives::trace::TxExecInfo;
use katana_primitives::transaction::Tx;
use katana_primitives::Felt;
use {postcard, zstd};

use super::{Compress, Decompress};
use crate::error::CodecError;
use crate::models::block::StoredBlockBodyIndices;
use crate::models::contract::ContractInfoChangeList;
use crate::models::list::BlockList;
use crate::models::stage::StageCheckpoint;
use crate::models::trie::TrieDatabaseValue;

macro_rules! impl_compress_and_decompress_for_table_values {
    ($($name:ty),*) => {
        $(
            impl Compress for $name {
                type Compressed = Vec<u8>;
                fn compress(self) -> Result<Self::Compressed, crate::error::CodecError> {
                    postcard::to_stdvec(&self)
                        .map_err(|e| CodecError::Compress(e.to_string()))
                }
            }

            impl Decompress for $name {
                fn decompress<B: AsRef<[u8]>>(bytes: B) -> Result<Self, crate::error::CodecError> {
                    postcard::from_bytes(bytes.as_ref()).map_err(|e| CodecError::Decompress(e.to_string()))
                }
            }
        )*
    }
}

impl Compress for TxExecInfo {
    type Compressed = Vec<u8>;
    fn compress(self) -> Result<Self::Compressed, crate::error::CodecError> {
        let serialized = postcard::to_stdvec(&self).unwrap();
        zstd::encode_all(serialized.as_slice(), 0).map_err(|e| CodecError::Compress(e.to_string()))
    }
}

impl Decompress for TxExecInfo {
    fn decompress<B: AsRef<[u8]>>(bytes: B) -> Result<Self, crate::error::CodecError> {
        let compressed = bytes.as_ref();
        let serialized =
            zstd::decode_all(compressed).map_err(|e| CodecError::Decompress(e.to_string()))?;
        postcard::from_bytes(&serialized).map_err(|e| CodecError::Decompress(e.to_string()))
    }
}

impl_compress_and_decompress_for_table_values!(
    u64,
    Tx,
    Header,
    Receipt,
    Felt,
    TrieDatabaseValue,
    ContractAddress,
    BlockList,
    StageCheckpoint,
    GenericContractInfo,
    StoredBlockBodyIndices,
    ContractInfoChangeList
);
