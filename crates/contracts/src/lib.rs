pub use katana_contracts_macro::contract;

pub mod contracts {
    use katana_contracts_macro::contract;

    contract!(LegacyERC20, "{CARGO_MANIFEST_DIR}/build/legacy/erc20.json");
    contract!(GenesisAccount, "{CARGO_MANIFEST_DIR}/build/legacy/account.json");
    contract!(UniversalDeployer, "{CARGO_MANIFEST_DIR}/build/legacy/universal_deployer.json");
    contract!(Account, "{CARGO_MANIFEST_DIR}/build/katana_account_Account.contract_class.json");
}

pub mod vrf {
    use katana_contracts_macro::contract;

    contract!(
        CartridgeVrfProvider,
        "{CARGO_MANIFEST_DIR}/build/cartridge_vrf_VrfProvider.contract_class.json"
    );
    contract!(
        CartridgeVrfConsumer,
        "{CARGO_MANIFEST_DIR}/build/cartridge_vrf_VrfConsumer.contract_class.json"
    );
    contract!(
        CartridgeVrfAccount,
        "{CARGO_MANIFEST_DIR}/build/cartridge_vrf_VrfAccount.contract_class.json"
    );
}

pub mod avnu {
    use katana_contracts_macro::contract;

    contract!(AvnuForwarder, "{CARGO_MANIFEST_DIR}/build/avnu_Forwarder.contract_class.json");
}

#[rustfmt::skip]
pub mod controller;

#[cfg(test)]
mod tests {
    use super::*;

    /// Asserts that decompressing the embedded contract class still yields a class whose
    /// computed hash matches the hash baked in at macro expansion time.
    #[test]
    fn embedded_compressed_classes_round_trip() {
        use contracts::*;
        assert_eq!(LegacyERC20::CLASS.class_hash().unwrap(), LegacyERC20::HASH);
        assert_eq!(GenesisAccount::CLASS.class_hash().unwrap(), GenesisAccount::HASH);
        assert_eq!(UniversalDeployer::CLASS.class_hash().unwrap(), UniversalDeployer::HASH);
        assert_eq!(Account::CLASS.class_hash().unwrap(), Account::HASH);
        assert_eq!(
            controller::ControllerLatest::CLASS.class_hash().unwrap(),
            controller::ControllerLatest::HASH
        );
    }
}
