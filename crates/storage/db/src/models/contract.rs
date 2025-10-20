use katana_primitives::class::ClassHash;
use katana_primitives::contract::{ContractAddress, Nonce};
use serde::{Deserialize, Serialize};

use super::list::BlockList;
use crate::codecs::{Compress, Decode, Decompress, Encode};
use crate::error::CodecError;

#[derive(Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ContractInfoChangeList {
    pub class_change_list: BlockList,
    pub nonce_change_list: BlockList,
}

/// The type of event that triggered the class change.
#[derive(Debug, Default, PartialEq, Eq)]
#[cfg_attr(test, derive(::arbitrary::Arbitrary))]
pub enum ContractClassChangeType {
    /// The contract was deployed with the given class hash.
    #[default]
    Deployed,
    /// The class change is made using the `replace_class` syscall.
    Replaced,
}

#[derive(Debug, Default, PartialEq, Eq)]
#[cfg_attr(test, derive(::arbitrary::Arbitrary))]
pub struct ContractClassChange {
    /// The type of class change.
    pub r#type: ContractClassChangeType,
    /// The address of the contract whose class has changed.
    pub contract_address: ContractAddress,
    /// The updated class hash of `contract_address`.
    pub class_hash: ClassHash,
}

impl ContractClassChange {
    /// Creates a new `ContractClassChange` instance representing a deployed contract.
    pub fn deployed(contract_address: ContractAddress, class_hash: ClassHash) -> Self {
        Self { r#type: ContractClassChangeType::Deployed, contract_address, class_hash }
    }

    /// Creates a new `ContractClassChange` instance representing a replaced class
    pub fn replaced(contract_address: ContractAddress, new_class_hash: ClassHash) -> Self {
        Self {
            r#type: ContractClassChangeType::Replaced,
            contract_address,
            class_hash: new_class_hash,
        }
    }

    fn decompress_current(bytes: &[u8]) -> Result<Self, CodecError> {
        let r#type = ContractClassChangeType::decompress(&bytes[0..1])?;
        let contract_address = ContractAddress::decode(&bytes[1..33])?;
        let class_hash = ClassHash::decompress(&bytes[33..])?;
        Ok(Self { r#type, contract_address, class_hash })
    }

    /// Backward compatibility purposes.
    ///
    /// The old format doesn't distinguish between a class change that happen because of a contract
    /// deployment or a class replacement via the `replace_class` system call. Thus, we are
    /// unable to determine the type of class change and have to just assume it's a deployment.
    #[cold]
    fn decompress_legacy(bytes: &[u8]) -> Result<Self, CodecError> {
        let contract_address = ContractAddress::decode(&bytes[0..32])?;
        let class_hash = ClassHash::decompress(&bytes[32..])?;
        Ok(Self { r#type: ContractClassChangeType::Deployed, contract_address, class_hash })
    }
}

impl Compress for ContractClassChangeType {
    type Compressed = Vec<u8>;
    fn compress(self) -> Result<Self::Compressed, CodecError> {
        let byte: u8 = match self {
            ContractClassChangeType::Deployed => 0,
            ContractClassChangeType::Replaced => 1,
        };
        Ok(vec![byte])
    }
}

impl Decompress for ContractClassChangeType {
    fn decompress<B: AsRef<[u8]>>(bytes: B) -> Result<Self, CodecError> {
        let bytes = bytes.as_ref();
        if bytes.is_empty() {
            return Err(CodecError::Decompress("can't decompress empty bytes".to_string()));
        }

        match bytes[0] {
            0 => Ok(ContractClassChangeType::Deployed),
            1 => Ok(ContractClassChangeType::Replaced),
            _ => Err(CodecError::Decompress("unknown ContractClassChangeType variant".to_string())),
        }
    }
}

impl Compress for ContractClassChange {
    type Compressed = Vec<u8>;

    fn compress(self) -> Result<Self::Compressed, CodecError> {
        let mut buf = Vec::new();
        buf.extend(self.r#type.compress()?);
        buf.extend(self.contract_address.encode());
        buf.extend(self.class_hash.compress()?);
        Ok(buf)
    }
}

impl Decompress for ContractClassChange {
    fn decompress<B: AsRef<[u8]>>(bytes: B) -> Result<Self, crate::error::CodecError> {
        let bytes = bytes.as_ref();

        if let Ok(result) = Self::decompress_current(bytes) {
            Ok(result)
        } else {
            ContractClassChange::decompress_legacy(bytes)
        }
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
#[cfg_attr(test, derive(::arbitrary::Arbitrary))]
pub struct ContractNonceChange {
    pub contract_address: ContractAddress,
    /// The updated nonce value of `contract_address`.
    pub nonce: Nonce,
}

impl Compress for ContractNonceChange {
    type Compressed = Vec<u8>;
    fn compress(self) -> Result<Self::Compressed, CodecError> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&self.contract_address.encode());
        buf.extend_from_slice(&self.nonce.compress()?);
        Ok(buf)
    }
}

impl Decompress for ContractNonceChange {
    fn decompress<B: AsRef<[u8]>>(bytes: B) -> Result<Self, crate::error::CodecError> {
        let bytes = bytes.as_ref();
        let contract_address = ContractAddress::decode(&bytes[0..32])?;
        let nonce = Nonce::decompress(&bytes[32..])?;
        Ok(Self { contract_address, nonce })
    }
}
