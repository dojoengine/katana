//! Registry of classes embedded into the katana binary at compile time.
//!
//! Bootstrap is agnostic over which classes/contracts are installed: callers can either
//! reference one of these embedded classes by name, or supply their own Sierra artifact
//! via the manifest. Adding a new embedded class is a single entry in [`REGISTRY`].

use anyhow::{anyhow, Result};
use katana_contracts::contracts;
use katana_primitives::class::{ClassHash, CompiledClassHash, ContractClass};

/// A Sierra class compiled into the katana binary, exposed to bootstrap by name.
#[derive(Debug)]
pub struct EmbeddedClass {
    /// The CLI/manifest-visible identifier (e.g. `dev_account`).
    pub name: &'static str,
    /// Short description shown in the interactive picker.
    pub description: &'static str,
    /// Loader producing the underlying [`ContractClass`]. Wrapped in a function so the
    /// (relatively heavy) Sierra payload is only materialised when actually needed.
    load: fn() -> ContractClass,
    /// Pre-computed Sierra class hash.
    pub class_hash: ClassHash,
    /// Pre-computed CASM hash needed for v3 declares.
    pub casm_hash: CompiledClassHash,
}

impl EmbeddedClass {
    /// Materialise the contract class. Always returns a Sierra class — legacy classes are
    /// not registered here because they cannot be declared via `declare_v3`.
    pub fn class(&self) -> ContractClass {
        (self.load)()
    }
}

/// All classes embedded in the binary, in display order.
///
/// Future additions (`oz_account`, `controller_account`, `erc20`, `erc721`) require adding
/// the cairo source under `crates/contracts/contracts/`, registering it in
/// `crates/contracts/src/lib.rs`, and then appending an entry here.
pub const REGISTRY: &[EmbeddedClass] = &[
    EmbeddedClass {
        name: "dev_account",
        description: "Default katana dev account (Cairo 1)",
        load: || contracts::Account::CLASS.clone(),
        class_hash: contracts::Account::HASH,
        casm_hash: contracts::Account::CASM_HASH,
    },
    EmbeddedClass {
        name: "udc",
        description: "OpenZeppelin Universal Deployer Contract (mainnet class)",
        load: || contracts::OpenZeppelinUniversalDeployer::CLASS.clone(),
        class_hash: contracts::OpenZeppelinUniversalDeployer::HASH,
        casm_hash: contracts::OpenZeppelinUniversalDeployer::CASM_HASH,
    },
];

/// Look up an embedded class by name.
pub fn get(name: &str) -> Option<&'static EmbeddedClass> {
    REGISTRY.iter().find(|c| c.name == name)
}

/// Look up an embedded class, returning a friendly error listing alternatives on miss.
pub fn require(name: &str) -> Result<&'static EmbeddedClass> {
    get(name).ok_or_else(|| {
        let known: Vec<&str> = REGISTRY.iter().map(|c| c.name).collect();
        anyhow!("unknown embedded class `{name}` (known: {})", known.join(", "))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_is_non_empty() {
        assert!(!REGISTRY.is_empty());
    }

    #[test]
    fn dev_account_is_registered_and_sierra() {
        let entry = get("dev_account").expect("dev_account must be registered");
        let class = entry.class();
        assert!(class.as_sierra().is_some(), "embedded classes must be Sierra");
        assert_eq!(class.class_hash().unwrap(), entry.class_hash);
    }

    /// The `udc` entry must resolve to the OpenZeppelin Universal Deployer Sierra class
    /// currently deployed on Starknet mainnet. Pinning the hash here guards against
    /// accidental swaps of the load function or the underlying vendored artifact.
    #[test]
    fn udc_is_registered_with_mainnet_hash() {
        use katana_primitives::Felt;
        const MAINNET_UDC: Felt = Felt::from_hex_unchecked(
            "0x01b2df6d8861670d4a8ca4670433b2418d78169c2947f46dc614e69f333745c8",
        );
        let entry = get("udc").expect("udc must be registered");
        assert_eq!(entry.class_hash, MAINNET_UDC);
        let class = entry.class();
        assert!(class.as_sierra().is_some(), "embedded classes must be Sierra");
        assert_eq!(class.class_hash().unwrap(), entry.class_hash);
    }

    #[test]
    fn unknown_class_is_rejected() {
        assert!(get("not_a_real_class").is_none());
        let err = match require("not_a_real_class") {
            Ok(_) => panic!("expected error"),
            Err(e) => e.to_string(),
        };
        assert!(err.contains("dev_account"));
    }
}
