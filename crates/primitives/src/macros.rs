/// Creates an [Ethereum Address][crate::eth::Address] from a literal hex string.
///
/// This macro supports both short and full-length (20-bytes) hex strings, and automatically pads
/// the hex string to 40 characters with leading zeros.
///
/// # Examples
/// ```
/// use katana_primitives::eth_address;
///
/// let addr1 = eth_address!("0x1337"); // Pads to 20 bytes
/// let addr2 = eth_address!("0xd8da6bf26964af9d7eed9e03e53415d37aa96045"); // Full 20 bytes
/// let addr3 = eth_address!("1337"); // Also works without 0x prefix
/// ```
#[macro_export]
macro_rules! eth_address {
    ($hex:literal) => {{
        // Runtime execution
        let hex_str = $hex;
        let hex_str = if hex_str.starts_with("0x") || hex_str.starts_with("0X") {
            &hex_str[2..]
        } else {
            hex_str
        };

        // Pad to 40 characters (20 bytes) with leading zeros
        let padded_hex =
            if hex_str.len() < 40 { format!("{:0>40}", hex_str) } else { hex_str.to_string() };

        $crate::_private::alloy_primitives::Address::from_str(&padded_hex).unwrap()
    }};
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use alloy_primitives::Address;

    #[test]
    fn test_eth_address_macro() {
        // Test with short hex
        let addr1 = eth_address!("0x1337");
        let expected1 = Address::from_str("0x0000000000000000000000000000000000001337").unwrap();
        assert_eq!(addr1, expected1);

        // Test without 0x prefix
        let addr2 = eth_address!("1337");
        assert_eq!(addr2, expected1);

        // Test with full 20 bytes
        let addr3 = eth_address!("0xd8da6bf26964af9d7eed9e03e53415d37aa96045");
        let expected3 = Address::from_str("0xd8da6bf26964af9d7eed9e03e53415d37aa96045").unwrap();
        assert_eq!(addr3, expected3);

        // Test single digit
        let addr4 = eth_address!("0x1");
        let expected4 = Address::from_str("0x0000000000000000000000000000000000000001").unwrap();
        assert_eq!(addr4, expected4);
    }
}
