//! Felt type conversions.

use katana_primitives::Felt;
use tonic::Status;

use crate::protos::types::Felt as ProtoFelt;

/// Convert from Katana Felt to proto Felt.
impl From<Felt> for ProtoFelt {
    fn from(felt: Felt) -> Self {
        ProtoFelt { value: felt.to_bytes_be().to_vec() }
    }
}

/// Convert from proto Felt to Katana Felt.
impl TryFrom<&ProtoFelt> for Felt {
    type Error = Status;

    fn try_from(proto: &ProtoFelt) -> Result<Self, Self::Error> {
        if proto.value.len() > 32 {
            return Err(Status::invalid_argument("Felt value exceeds 32 bytes"));
        }

        // Pad the value to 32 bytes if necessary (big-endian)
        let mut bytes = [0u8; 32];
        let offset = 32 - proto.value.len();
        bytes[offset..].copy_from_slice(&proto.value);

        // from_bytes_be returns Felt directly (doesn't fail for valid 32-byte input)
        Ok(Felt::from_bytes_be(&bytes))
    }
}

impl TryFrom<ProtoFelt> for Felt {
    type Error = Status;

    fn try_from(proto: ProtoFelt) -> Result<Self, Self::Error> {
        Felt::try_from(&proto)
    }
}

/// Extension trait for converting vectors of Felts.
pub trait FeltVecExt {
    fn to_proto_felts(&self) -> Vec<ProtoFelt>;
}

impl FeltVecExt for [Felt] {
    fn to_proto_felts(&self) -> Vec<ProtoFelt> {
        self.iter().copied().map(ProtoFelt::from).collect()
    }
}

/// Extension trait for converting vectors of proto Felts.
pub trait ProtoFeltVecExt {
    fn to_felts(&self) -> Result<Vec<Felt>, Status>;
}

impl ProtoFeltVecExt for [ProtoFelt] {
    fn to_felts(&self) -> Result<Vec<Felt>, Status> {
        self.iter().map(Felt::try_from).collect()
    }
}
