//! TOML manifest schema for `katana bootstrap`.
//!
//! A manifest describes a sequence of class declarations and contract deployments to
//! perform against a running katana node. The schema is intentionally minimal: in v1
//! it only covers declares and deploys (no storage writes / balance prefunding).
//!
//! See `examples/bootstrap.toml` for the canonical shape.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use katana_primitives::Felt;
use serde::{Deserialize, Serialize};

/// Top-level manifest. Currently only `schema = 1` is accepted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    /// Manifest schema version. Bump when introducing breaking changes.
    pub schema: u32,

    /// Classes to declare, in order. Each entry must have a unique `name`.
    #[serde(default, rename = "class")]
    pub classes: Vec<ClassEntry>,

    /// Contracts to deploy, in order. Each entry references a class by `name` (either
    /// a class declared earlier in this manifest or an embedded class registered in
    /// [`crate::cli::bootstrap::embedded`]).
    #[serde(default, rename = "contract")]
    pub contracts: Vec<ContractEntry>,
}

/// One class to declare.
///
/// Exactly one of `embedded` or `path` must be set:
/// - `embedded = "dev_account"` selects a class compiled into the binary;
/// - `path = "./build/foo.json"` loads a Sierra class from disk. The CASM hash is re-derived at
///   runtime by recompiling the Sierra program.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassEntry {
    /// Local alias used by [`ContractEntry::class`] to reference this declaration.
    pub name: String,
    /// Name of an embedded class in [`crate::cli::bootstrap::embedded::REGISTRY`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedded: Option<String>,
    /// Path to a Sierra class JSON on disk.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
}

/// One contract to deploy via the Universal Deployer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractEntry {
    /// The local class alias to deploy. Must match either a `[[class]]` `name` from this
    /// manifest or an embedded class name.
    pub class: String,
    /// Optional human-readable label, surfaced in the summary output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Salt used by the UDC. Defaults to `0x0`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub salt: Option<Felt>,
    /// `unique` flag passed to the UDC. Defaults to `false`.
    #[serde(default)]
    pub unique: bool,
    /// Constructor calldata as raw felts.
    #[serde(default)]
    pub calldata: Vec<Felt>,
}

impl Manifest {
    /// Load and validate a manifest from disk.
    pub fn load(path: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read manifest at {}", path.display()))?;
        let manifest: Manifest = toml::from_str(&raw)
            .with_context(|| format!("failed to parse manifest at {}", path.display()))?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// Run structural validation: schema version, unique class names, exclusive
    /// `embedded`/`path`, and that every contract references a known class.
    pub fn validate(&self) -> Result<()> {
        if self.schema != 1 {
            return Err(anyhow!("unsupported manifest schema {} (expected 1)", self.schema));
        }

        let mut seen = std::collections::HashSet::new();
        for class in &self.classes {
            if !seen.insert(class.name.as_str()) {
                return Err(anyhow!("duplicate class name `{}`", class.name));
            }
            match (&class.embedded, &class.path) {
                (Some(_), Some(_)) => {
                    return Err(anyhow!(
                        "class `{}`: `embedded` and `path` are mutually exclusive",
                        class.name
                    ));
                }
                (None, None) => {
                    return Err(anyhow!(
                        "class `{}`: must set either `embedded` or `path`",
                        class.name
                    ));
                }
                _ => {}
            }
        }

        for (idx, contract) in self.contracts.iter().enumerate() {
            let known_local = self.classes.iter().any(|c| c.name == contract.class);
            let known_embedded = crate::embedded::get(&contract.class).is_some();
            if !known_local && !known_embedded {
                return Err(anyhow!(
                    "contract #{idx}: references unknown class `{}` (not declared in this \
                     manifest and not an embedded class)",
                    contract.class
                ));
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Result<Manifest> {
        let m: Manifest = toml::from_str(s)?;
        m.validate()?;
        Ok(m)
    }

    #[test]
    fn parses_minimal_manifest() {
        let manifest = parse(
            r#"
            schema = 1

            [[class]]
            name = "dev_account"
            embedded = "dev_account"

            [[contract]]
            class = "dev_account"
            label = "alice"
            calldata = ["0x123"]
            "#,
        )
        .unwrap();

        assert_eq!(manifest.classes.len(), 1);
        assert_eq!(manifest.contracts.len(), 1);
        assert_eq!(manifest.contracts[0].label.as_deref(), Some("alice"));
    }

    #[test]
    fn rejects_unknown_schema() {
        let err = parse("schema = 99").unwrap_err().to_string();
        assert!(err.contains("schema"));
    }

    #[test]
    fn rejects_embedded_and_path_together() {
        let err = parse(
            r#"
            schema = 1
            [[class]]
            name = "x"
            embedded = "dev_account"
            path = "./foo.json"
            "#,
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("mutually exclusive"));
    }

    #[test]
    fn rejects_class_without_source() {
        let err = parse(
            r#"
            schema = 1
            [[class]]
            name = "x"
            "#,
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("must set either"));
    }

    #[test]
    fn rejects_duplicate_class_names() {
        let err = parse(
            r#"
            schema = 1
            [[class]]
            name = "x"
            embedded = "dev_account"
            [[class]]
            name = "x"
            embedded = "dev_account"
            "#,
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("duplicate"));
    }

    #[test]
    fn rejects_contract_with_unknown_class() {
        let err = parse(
            r#"
            schema = 1
            [[contract]]
            class = "ghost"
            "#,
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("unknown class"));
    }

    #[test]
    fn accepts_contract_referencing_embedded_class_directly() {
        // No [[class]] declared but the contract references an embedded one — fine.
        parse(
            r#"
            schema = 1
            [[contract]]
            class = "dev_account"
            calldata = ["0x1"]
            "#,
        )
        .unwrap();
    }
}
