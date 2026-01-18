use katana_tee_client::TeeQuoteResponse;

const EXAMPLE_JSON: &str = r#"{
    "jsonrpc": "2.0",
    "id": 1,
    "result": {
        "quote": "0x0500000000000000",
        "stateRoot": "0x5da4151adf86566185d11904106b9c682225978d65129a170704dad449f642d",
        "blockHash": "0x54d29b665f69e69f551fe33159ae4be707968c5b953ca9946701fd8633cb5bd",
        "blockNumber": 0
    }
}"#;

#[test]
fn test_parse_json_rpc_response() {
    let response = TeeQuoteResponse::from_json_str(EXAMPLE_JSON).unwrap();
    assert_eq!(response.quote, "0x0500000000000000");
    assert_eq!(response.block_number, 0);
}

#[test]
fn test_quote_bytes() {
    let response = TeeQuoteResponse::from_json_str(EXAMPLE_JSON).unwrap();
    let bytes = response.quote_bytes().unwrap();
    assert_eq!(bytes, vec![0x05, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);
}

#[test]
fn test_parse_direct() {
    let direct_json = r#"{
        "quote": "0xabcd",
        "stateRoot": "0x123",
        "blockHash": "0x456",
        "blockNumber": 42
    }"#;
    let response = TeeQuoteResponse::from_json_str(direct_json).unwrap();
    assert_eq!(response.block_number, 42);
}
