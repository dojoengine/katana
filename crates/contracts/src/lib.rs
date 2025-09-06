pub use katana_contracts_macro::contract;

pub mod contracts {
    use katana_contracts_macro::contract;

    contract!(LegacyERC20, "{CARGO_MANIFEST_DIR}/build/legacy/erc20.json");
    contract!(GenesisAccount, "{CARGO_MANIFEST_DIR}/build/legacy/account.json");
    contract!(UniversalDeployer, "{CARGO_MANIFEST_DIR}/build/legacy/universal_deployer.json");
    contract!(Account, "{CARGO_MANIFEST_DIR}/build/katana_account_Account.contract_class.json");
}
