use katana_contracts_macro::contract;
use katana_primitives::felt;

#[test]
fn contract_macro() {
    contract!(AccountContract, "crates/contracts/build/katana_account_Account.contract_class.json");

    assert_eq!(
        felt!("0x07dc7899aa655b0aae51eadff6d801a58e97dd99cf4666ee59e704249e51adf2"),
        AccountContract::HASH
    );
    assert_eq!(
        felt!("0x01b97e0ef7f5c2f2b7483cda252a3accc7f917773fb69d4bd290f92770069aec"),
        AccountContract::CASM_HASH
    );
}
