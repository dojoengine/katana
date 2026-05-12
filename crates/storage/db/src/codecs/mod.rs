#[cfg(feature = "postcard")]
pub mod postcard;

use katana_primitives::block::FinalityStatus;
use katana_primitives::contract::ContractAddress;
use katana_primitives::Felt;

use crate::error::CodecError;

/// A trait for encoding the key of a table.
pub trait Encode {
    type Encoded: AsRef<[u8]> + Into<Vec<u8>>;
    fn encode(self) -> Self::Encoded;
}

pub trait Decode: Sized {
    fn decode<B: AsRef<[u8]>>(bytes: B) -> Result<Self, CodecError>;
}

/// A trait for compressing data that are stored in the db.
pub trait Compress {
    type Compressed: AsRef<[u8]> + Into<Vec<u8>>;
    fn compress(self) -> Result<Self::Compressed, CodecError>;
}

/// A trait for decompressing data that are read from the db.
pub trait Decompress: Sized {
    fn decompress<B: AsRef<[u8]>>(bytes: B) -> Result<Self, CodecError>;
}

macro_rules! impl_encode_and_decode_for_uints {
    ($($ty:ty),*) => {
        $(
            impl Encode for $ty {
                type Encoded = [u8; std::mem::size_of::<$ty>()];
                fn encode(self) -> Self::Encoded {
                    self.to_be_bytes()
                }
            }

            impl Decode for $ty {
                fn decode<B: AsRef<[u8]>>(bytes: B) -> Result<Self, CodecError> {
                    let mut buf = [0u8; std::mem::size_of::<$ty>()];
                    buf.copy_from_slice(bytes.as_ref());
                    Ok(Self::from_be_bytes(buf))
                }
            }
        )*
    }
}

macro_rules! impl_encode_and_decode_for_felts {
    ($($ty:ty),*) => {
        $(
            impl Encode for $ty {
                type Encoded = [u8; 32];
                fn encode(self) -> Self::Encoded {
                    self.to_bytes_be()
                }
            }

            impl Decode for $ty {
                fn decode<B: AsRef<[u8]>>(bytes: B) -> Result<Self, CodecError> {
                    Ok(Felt::from_bytes_be_slice(bytes.as_ref()).into())
                }
            }
        )*
    }
}

impl_encode_and_decode_for_uints!(u64);
impl_encode_and_decode_for_felts!(Felt, ContractAddress);

/// 32-byte fixed array support, used for chain-agnostic hash keys (Ethereum B256,
/// Starknet Felt). Stored as raw bytes in big-endian order.
impl Encode for [u8; 32] {
    type Encoded = [u8; 32];
    fn encode(self) -> Self::Encoded {
        self
    }
}

impl Decode for [u8; 32] {
    fn decode<B: AsRef<[u8]>>(bytes: B) -> Result<Self, CodecError> {
        let bytes = bytes.as_ref();
        if bytes.len() != 32 {
            return Err(CodecError::Decode(format!(
                "expected 32 bytes for [u8; 32], got {}",
                bytes.len()
            )));
        }
        let mut buf = [0u8; 32];
        buf.copy_from_slice(bytes);
        Ok(buf)
    }
}

impl Encode for String {
    type Encoded = Vec<u8>;
    fn encode(self) -> Self::Encoded {
        self.into_bytes()
    }
}

impl Decode for String {
    fn decode<B: AsRef<[u8]>>(bytes: B) -> Result<Self, CodecError> {
        String::from_utf8(bytes.as_ref().to_vec()).map_err(|e| CodecError::Decode(e.to_string()))
    }
}

impl Compress for FinalityStatus {
    type Compressed = [u8; 1];
    fn compress(self) -> Result<Self::Compressed, CodecError> {
        Ok([self as u8])
    }
}

impl Decompress for FinalityStatus {
    fn decompress<B: AsRef<[u8]>>(bytes: B) -> Result<Self, CodecError> {
        match bytes.as_ref().first() {
            Some(0) => Ok(FinalityStatus::AcceptedOnL2),
            Some(1) => Ok(FinalityStatus::AcceptedOnL1),
            _ => Err(CodecError::Decode("Invalid status".into())),
        }
    }
}
