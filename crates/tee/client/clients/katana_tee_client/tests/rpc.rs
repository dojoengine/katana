use katana_tee_client::KatanaRpcClient;

#[test]
fn test_client_from_env() {
    std::env::set_var("KATANA_RPC_URL", "http://test:1234");
    let client = KatanaRpcClient::from_env();
    assert_eq!(client.url(), "http://test:1234");
    std::env::remove_var("KATANA_RPC_URL");
}

#[test]
fn test_client_default() {
    std::env::remove_var("KATANA_RPC_URL");
    let client = KatanaRpcClient::from_env();
    assert_eq!(client.url(), "http://localhost:5050");
}
