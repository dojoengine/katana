use ark_ff::PrimeField;
use num_bigint::BigInt;
use stark_vrf::ScalarField;
use starknet::core::types::Felt;
use std::str::FromStr;

pub fn format<T: std::fmt::Display>(v: T) -> String {
    let int = BigInt::from_str(&format!("{v}")).unwrap();
    format!("0x{}", int.to_str_radix(16))
}

pub fn format_felt<T: std::fmt::Display>(v: T) -> Felt {
    let hex = format(v);
    Felt::from_hex_unchecked(&hex)
}

pub fn parse_felt(value: &str) -> Result<Felt, String> {
    if value.starts_with("0x") || value.starts_with("0X") {
        Felt::from_hex(value).map_err(|e| e.to_string())
    } else {
        Felt::from_dec_str(value).map_err(|e| e.to_string())
    }
}

pub fn felt_to_scalar(value: Felt) -> ScalarField {
    let bytes = value.to_bytes_be();
    ScalarField::from_be_bytes_mod_order(&bytes)
}
