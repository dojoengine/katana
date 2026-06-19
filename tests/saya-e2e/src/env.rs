use starknet::core::types::Felt;

/// RPC URL of the settlement (L2) chain.
pub fn settlement_rpc_url() -> String {
    std::env::var("SETTLEMENT_RPC_URL").unwrap_or_else(|_| "http://localhost:5050".to_string())
}

/// RPC URL of the rollup (L3) chain.
pub fn l3_rpc_url() -> String {
    std::env::var("L3_RPC_URL").unwrap_or_else(|_| "http://localhost:5051".to_string())
}

/// L2 messaging contract (`sn_msg`).
pub fn sn_msg_address() -> Felt {
    Felt::from_hex(&std::env::var("SN_MSG_ADDRESS").unwrap_or_else(|_| {
        "0x042ba247c6b41acc8fa299571a1c9ad3c0c19511fe09255ea9ff9287c250144a".to_string()
    }))
    .expect("invalid SN_MSG_ADDRESS")
}

/// L3 messaging contract (`appc_msg_sn`).
pub fn appc_msg_sn_address() -> Felt {
    Felt::from_hex(&std::env::var("APPC_MSG_SN_ADDRESS").unwrap_or_else(|_| {
        "0x00be8c1b5ddc2edacb375bc8734b8a96d618f8213df8bd531e60fa338c0aa429".to_string()
    }))
    .expect("invalid APPC_MSG_SN_ADDRESS")
}

/// L2 katana0 account address.
pub fn l2_account_address() -> Felt {
    Felt::from_hex("0x127fd5f1fe78a71f8bcd1fec63e3fe2f0486b6ecd5c86a0466c3a21fa5cfcec")
        .expect("invalid L2 account address")
}

/// L2 katana0 private key.
pub fn l2_private_key() -> Felt {
    Felt::from_hex("0xc5b2fcab997346f3ea1c00b002ecf6f382c5f9c9659a3894eb783c5320f912")
        .expect("invalid L2 private key")
}

/// L3 prefunded account address.
pub fn appc_account_address() -> Felt {
    Felt::from_hex("0x1f401c745d3dba9b9da11921d1fb006c96f571e9039a0ece3f3b0dc14f04c3d")
        .expect("invalid L3 account address")
}

/// L3 prefunded account private key.
pub fn appc_private_key() -> Felt {
    Felt::from_hex("0x7230b49615d175307d580c33d6fda61fc7b9aec91df0f5c1a5ebe3b8cbfee02")
        .expect("invalid L3 private key")
}

/// Address of the deployed piltover core contract.
/// Defaults to the address used in `compose.yml`.
pub fn piltover_address() -> Felt {
    let addr = std::env::var("PILTOVER_ADDRESS").unwrap_or_else(|_| {
        "0x387f8d08b19b4f0b3809a3411eab4a27f804ad672a37faeab77bd99af2b9896".to_string()
    });
    Felt::from_hex(&addr).expect("invalid PILTOVER_ADDRESS")
}

/// Chain ID used when configuring the core contract program info.
pub fn chain_id() -> String {
    std::env::var("CHAIN_ID").unwrap_or_else(|_| "custom".to_string())
}

/// Fact registry address configured for the core contract.
pub fn fact_registry_address() -> Felt {
    let addr = std::env::var("FACT_REGISTRY_ADDRESS").unwrap_or_else(|_| {
        "0x3eb0d510d1238120bf7f9d176faafe0c7066797a86be985855952f87769d3bd".to_string()
    });
    Felt::from_hex(&addr).expect("invalid FACT_REGISTRY_ADDRESS")
}

/// Fee token address used to build the Starknet OS config hash.
pub fn fee_token_address() -> Felt {
    let addr = std::env::var("FEE_TOKEN_ADDRESS").unwrap_or_else(|_| {
        "0x2e7442625bab778683501c0eadbc1ea17b3535da040a12ac7d281066e915eea".to_string()
    });
    Felt::from_hex(&addr).expect("invalid FEE_TOKEN_ADDRESS")
}
