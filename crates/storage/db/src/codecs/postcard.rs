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
                fn compress(self) -> Self::Compressed {
                    postcard::to_stdvec(&self).unwrap()
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

// TxExecInfo gets special handling with zstd compression
impl Compress for TxExecInfo {
    type Compressed = Vec<u8>;
    fn compress(self) -> Self::Compressed {
        let serialized = postcard::to_stdvec(&self).unwrap();

        match zstd::encode_all(serialized.as_slice(), 0) {
            Ok(compressed) => {
                let mut result = vec![1]; // 1 = compressed
                result.extend(compressed);
                result
            }
            Err(_) => {
                let mut result = vec![0]; // 0 = uncompressed
                result.extend(serialized);
                result
            }
        }
    }
}

impl Decompress for TxExecInfo {
    fn decompress<B: AsRef<[u8]>>(bytes: B) -> Result<Self, crate::error::CodecError> {
        let bytes_ref = bytes.as_ref();
        if bytes_ref.is_empty() {
            return Err(CodecError::Decompress("Empty input".to_string()));
        }

        let (indicator, data) = (bytes_ref[0], &bytes_ref[1..]);

        let decompressed = if indicator == 1 {
            match zstd::decode_all(data) {
                Ok(d) => d,
                Err(e) => {
                    return Err(CodecError::Decompress(format!("zstd decompression error: {}", e)))
                }
            }
        } else {
            data.to_vec()
        };

        postcard::from_bytes(&decompressed).map_err(|e| CodecError::Decompress(e.to_string()))
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
