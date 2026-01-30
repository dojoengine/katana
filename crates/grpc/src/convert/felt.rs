//! Felt type conversions.

use katana_primitives::Felt;
use tonic::Status;

use crate::protos::types::Felt as ProtoFelt;

/// Converts a proto Felt to a Katana Felt.
pub fn from_proto_felt(proto: &ProtoFelt) -> Result<Felt, Status> {
    if proto.value.len() > 32 {
        return Err(Status::invalid_argument("Felt value exceeds 32 bytes"));
    }

    // Pad the value to 32 bytes if necessary (big-endian)
    let mut bytes = [0u8; 32];
    let offset = 32 - proto.value.len();
    bytes[offset..].copy_from_slice(&proto.value);

    Felt::from_bytes_be(&bytes)
        .map_err(|e| Status::invalid_argument(format!("Invalid Felt value: {e}")))
}

/// Converts a proto Felt option to a Katana Felt option.
pub fn from_proto_felt_opt(proto: Option<&ProtoFelt>) -> Result<Option<Felt>, Status> {
    proto.map(from_proto_felt).transpose()
}

/// Converts a Katana Felt to a proto Felt.
pub fn to_proto_felt(felt: Felt) -> ProtoFelt {
    ProtoFelt { value: felt.to_bytes_be().to_vec() }
}

/// Converts a Katana Felt option to a proto Felt option.
pub fn to_proto_felt_opt(felt: Option<Felt>) -> Option<ProtoFelt> {
    felt.map(to_proto_felt)
}

/// Converts a vector of proto Felts to Katana Felts.
pub fn from_proto_felts(protos: &[ProtoFelt]) -> Result<Vec<Felt>, Status> {
    protos.iter().map(from_proto_felt).collect()
}

/// Converts a vector of Katana Felts to proto Felts.
pub fn to_proto_felts(felts: &[Felt]) -> Vec<ProtoFelt> {
    felts.iter().copied().map(to_proto_felt).collect()
}
