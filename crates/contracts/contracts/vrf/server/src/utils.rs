use num::BigInt;
use starknet_crypto::Felt;
use std::str::FromStr;

pub fn format<T: std::fmt::Display>(v: T) -> String {
    let int = BigInt::from_str(&format!("{v}")).unwrap();
    format!("0x{}", int.to_str_radix(16))
}

pub fn format_felt<T: std::fmt::Display>(v: T) -> Felt {
    let hex = format(v);
    Felt::from_hex_unchecked(&hex)
}
