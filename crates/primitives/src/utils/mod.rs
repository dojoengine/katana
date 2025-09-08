use starknet_types_core::felt::NonZeroFelt;
use starknet_types_core::hash::{Pedersen, StarkHash};

use crate::{Felt, U256};

pub mod class;
pub mod transaction;

/// Split a [U256] into its high and low 128-bit parts in represented as [FieldElement]s.
/// The first element in the returned tuple is the low part, and the second element is the high
/// part.
pub fn split_u256(value: U256) -> (Felt, Felt) {
    let low_u128: u128 = (value & U256::from(u128::MAX)).to();
    let high_u128: u128 = U256::from(value >> 128).to();
    (Felt::from(low_u128), Felt::from(high_u128))
}

/// This function can be used to calculate target addresses from `DEPLOY_ACCOUNT` transactions or
/// invoking the `deploy` syscall. The `deployer_address` parameter should be set to `0` for
/// `DEPLOY_ACCOUNT` transactions, and in other cases, to the address of the contract where the
/// `deploy` syscall is invoked.
///
/// Implementation reference: https://github.com/starkware-libs/cairo-lang/blob/v0.14.0/src/starkware/starknet/core/os/contract_address/contract_address.cairo
pub fn get_contract_address(
    salt: Felt,
    class_hash: Felt,
    constructor_calldata: &[Felt],
    deployer_address: Felt,
) -> Felt {
    // Cairo string of 'STARKNET_CONTRACT_ADDRESS'
    const CONTRACT_ADDRESS_PREFIX: Felt = Felt::from_raw([
        533439743893157637,
        8635008616843941496,
        17289941567720117366,
        3829237882463328880,
    ]);

    let address = Pedersen::hash_array(&[
        CONTRACT_ADDRESS_PREFIX,
        deployer_address,
        salt,
        class_hash,
        Pedersen::hash_array(constructor_calldata),
    ]);

    normalize_address(address)
}

/// Valid storage addresses should satisfy `address + offset < 2**251` where `offset <
/// 256` and `address < ADDR_BOUND`.
///
/// 2 ** 251 - 256
const ADDR_BOUND: NonZeroFelt = NonZeroFelt::from_raw([
    576459263475590224,
    18446744073709255680,
    160989183,
    18446743986131443745,
]);

/// Computes addr % [`ADDR_BOUND`] so that the result will form a valid storage item address in the
/// storage tree.
///
/// Implementation reference: https://github.com/starkware-libs/cairo-lang/blob/6d99011f6ef2a3dc178f7c8db4f0ddc6e836f303/src/starkware/starknet/common/storage.cairo#L12-L54
pub fn normalize_address(address: Felt) -> Felt {
    address.mod_floor(&ADDR_BOUND)
}

/// A variant of eth-keccak that computes a value that fits in a Starknet [`Felt`](crate::Felt).
///
/// Implementation reference: https://github.com/starkware-libs/cairo-lang/blob/66355d7d99f1962ff9ccba8d0dbacbce3bd79bf8/src/starkware/starknet/public/abi.py#L38-L43
pub fn starknet_keccak(data: &[u8]) -> Felt {
    use alloy_primitives::Keccak256;

    let mut hasher = Keccak256::new();
    hasher.update(data);
    let hash = hasher.finalize();

    // Safety: The output of the hash function is guaranteed to be 32 bytes long.
    let mut hash = unsafe { *(hash[..].as_ptr() as *const [u8; 32]) };
    // Mask the first 6 bits to ensure the result is less than 2**250 - 1
    hash[0] &= 0b00000011;

    Felt::from_bytes_be(&hash)
}

/// Error type for non-ASCII storage variable names.
#[derive(Debug, thiserror::Error)]
#[error("storage variable name `{name}` is not a valid ASCII string")]
pub struct NonAsciiNameError {
    /// The name of the storage variable that caused the error.
    pub name: String,
}

/// Returns the storage address of a Starkbet storage variable given its name and arguments.
///
/// Implementation reference: https://github.com/starkware-libs/cairo-lang/blob/66355d7d99f1962ff9ccba8d0dbacbce3bd79bf8/src/starkware/starknet/public/abi.py#L75
pub fn get_storage_var_address(var_name: &str, args: &[Felt]) -> Result<Felt, NonAsciiNameError> {
    if var_name.is_ascii() {
        let mut res = starknet_keccak(var_name.as_bytes());

        for arg in args {
            res = Pedersen::hash(&res, arg);
        }

        Ok(normalize_address(res))
    } else {
        Err(NonAsciiNameError { name: var_name.to_string() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_u256() {
        // Given
        let value = U256::MAX;

        // When
        let (low, high) = split_u256(value);

        // Then
        assert_eq!(low, Felt::from(u128::MAX));
        assert_eq!(high, Felt::from(u128::MAX));
    }
}
