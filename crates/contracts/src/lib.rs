pub mod contracts {
    use katana_contracts_macro::contract;

    contract!(LegacyERC20, "crates/contracts/build/legacy/erc20.json");
    contract!(GenesisAccount, "crates/contracts/build/legacy/account.json");
    contract!(UniversalDeployer, "crates/contracts/build/legacy/universal_deployer.json");
    contract!(Account, "crates/contracts/build/katana_account_Account.contract_class.json");
}
