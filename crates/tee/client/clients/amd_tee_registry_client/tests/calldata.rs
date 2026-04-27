use amd_tee_registry_client::StarknetCalldata;
use num_bigint::BigUint;

#[test]
fn test_hex_string_conversion() {
    let calldata = StarknetCalldata::from_values(vec![
        BigUint::from(255u32),
        BigUint::from(4095u32),
        BigUint::from(0u32),
    ]);

    let hex_strings = calldata.to_hex_strings();
    assert_eq!(hex_strings[0], "0xff");
    assert_eq!(hex_strings[1], "0xfff");
    assert_eq!(hex_strings[2], "0x0");
}

#[test]
fn test_hex_file_content() {
    // to_hex_file_content skips the first element (length prefix)
    // So we include a length prefix (2) followed by the actual values
    let calldata = StarknetCalldata::from_values(vec![
        BigUint::from(2u32), // length prefix
        BigUint::from(1u32),
        BigUint::from(2u32),
    ]);

    let content = calldata.to_hex_file_content();
    assert_eq!(content, "0x1\n0x2\n");
}

#[test]
fn test_decimal_strings() {
    let calldata =
        StarknetCalldata::from_values(vec![BigUint::from(255u32), BigUint::from(1000u32)]);

    let decimal_strings = calldata.to_decimal_strings();
    assert_eq!(decimal_strings[0], "255");
    assert_eq!(decimal_strings[1], "1000");
}
