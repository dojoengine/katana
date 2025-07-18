use crate::contract;

contract!(Erc20, "crates/contracts/build/legacy/erc20.json", crate);
contract!(GenesisAccount, "crates/contracts/build/legacy/account.json", crate);
contract!(UniversalDeployer, "crates/contracts/build/legacy/universal_deployer.json", crate);
contract!(Account, "crates/contracts/build/katana_account_Account.contract_class.json", crate);
