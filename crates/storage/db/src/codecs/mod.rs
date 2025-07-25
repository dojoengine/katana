#[cfg(feature = "postcard")]
pub mod postcard;

use katana_primitives::block::FinalityStatus;
use katana_primitives::class::ContractClass;
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
    type Compressed: AsRef<[u8]>;
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

impl Compress for ContractClass {
    type Compressed = Vec<u8>;
    fn compress(self) -> Result<Self::Compressed, CodecError> {
        serde_json::to_vec(&self).map_err(|e| CodecError::Compress(e.to_string()))
    }
}

impl Decompress for ContractClass {
    fn decompress<B: AsRef<[u8]>>(bytes: B) -> Result<Self, CodecError> {
        serde_json::from_slice(bytes.as_ref()).map_err(|e| CodecError::Decode(e.to_string()))
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
