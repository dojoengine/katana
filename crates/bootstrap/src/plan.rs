//! Resolved, ready-to-execute representation of a bootstrap operation.
//!
//! A [`BootstrapPlan`] is what the executor consumes. It is produced from either a
//! parsed [`Manifest`](crate::manifest::Manifest) or from the interactive prompt
//! session, so the executor never has to know which front-end produced the work.
//!
//! Resolution does the I/O up front: file-loaded classes are read from disk and their
//! hashes computed once, so the executor can be a thin RPC submitter.

use std::collections::HashMap;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use katana_primitives::class::{ClassHash, CompiledClassHash, ContractClass};
use katana_primitives::Felt;

use crate::embedded::{self, EmbeddedClass};
use crate::manifest::{ClassEntry, ContractEntry, Manifest};

/// A fully-resolved declare step.
#[derive(Debug, Clone)]
pub struct DeclareStep {
    /// Local alias used to cross-reference this declaration from deploy steps.
    pub name: String,
    /// The Sierra class to declare. Wrapped in [`Arc`] because the same class may be
    /// referenced by multiple deploys and the underlying payload is large.
    pub class: Arc<ContractClass>,
    pub class_hash: ClassHash,
    pub casm_hash: CompiledClassHash,
    /// Set when the source is an embedded class — used purely for nicer summary output.
    pub source: ClassSource,
}

#[derive(Debug, Clone)]
pub enum ClassSource {
    Embedded(&'static str),
    File(PathBuf),
}

/// A fully-resolved deploy step.
#[derive(Debug, Clone)]
pub struct DeployStep {
    pub label: Option<String>,
    /// The class hash this deploy targets. Already resolved against [`DeclareStep`]s
    /// or the embedded registry, so the executor doesn't need a lookup table.
    pub class_hash: ClassHash,
    /// Local alias of the resolved class — for summary output.
    pub class_name: String,
    pub salt: Felt,
    pub unique: bool,
    pub calldata: Vec<Felt>,
}

/// The executable plan handed to the executor.
#[derive(Debug, Clone, Default)]
pub struct BootstrapPlan {
    pub declares: Vec<DeclareStep>,
    pub deploys: Vec<DeployStep>,
}

impl BootstrapPlan {
    /// Resolve a parsed manifest into an executable plan.
    ///
    /// This reads any file-referenced classes from disk and computes their hashes,
    /// failing fast on bad inputs so the executor can stay simple.
    pub fn from_manifest(manifest: &Manifest) -> Result<Self> {
        // Map from local class alias -> (hash, casm_hash). Used for resolving deploy refs
        // that point at classes declared earlier in the same manifest.
        let mut local_aliases: HashMap<String, (ClassHash, String)> = HashMap::new();
        let mut declares = Vec::with_capacity(manifest.classes.len());

        for entry in &manifest.classes {
            let declare = resolve_class_entry(entry)?;
            local_aliases.insert(declare.name.clone(), (declare.class_hash, declare.name.clone()));
            declares.push(declare);
        }

        let mut deploys = Vec::with_capacity(manifest.contracts.len());
        for (idx, entry) in manifest.contracts.iter().enumerate() {
            deploys.push(resolve_contract_entry(idx, entry, &local_aliases)?);
        }

        Ok(Self { declares, deploys })
    }
}

fn resolve_class_entry(entry: &ClassEntry) -> Result<DeclareStep> {
    if let Some(name) = entry.embedded.as_deref() {
        let embedded =
            embedded::require(name).with_context(|| format!("class `{}`", entry.name))?;
        return Ok(declare_from_embedded(&entry.name, embedded));
    }

    let path = entry.path.as_ref().expect("validate() guarantees one source is set");
    declare_from_file(&entry.name, path)
}

fn declare_from_embedded(local_name: &str, embedded: &'static EmbeddedClass) -> DeclareStep {
    DeclareStep {
        name: local_name.to_string(),
        class: Arc::new(embedded.class()),
        class_hash: embedded.class_hash,
        casm_hash: embedded.casm_hash,
        source: ClassSource::Embedded(embedded.name),
    }
}

fn declare_from_file(local_name: &str, path: &std::path::Path) -> Result<DeclareStep> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("class `{local_name}`: failed to read {}", path.display()))?;
    let class = ContractClass::from_str(&raw)
        .with_context(|| format!("class `{local_name}`: invalid sierra json {}", path.display()))?;

    if class.is_legacy() {
        return Err(anyhow!(
            "class `{local_name}`: legacy (Cairo 0) classes are not supported by bootstrap; use a \
             Sierra class"
        ));
    }

    let class_hash = class
        .class_hash()
        .with_context(|| format!("class `{local_name}`: failed to compute class hash"))?;

    let compiled = class
        .clone()
        .compile()
        .with_context(|| format!("class `{local_name}`: failed to compile to casm"))?;
    let casm_hash = compiled
        .class_hash()
        .with_context(|| format!("class `{local_name}`: failed to compute casm hash"))?;

    Ok(DeclareStep {
        name: local_name.to_string(),
        class: Arc::new(class),
        class_hash,
        casm_hash,
        source: ClassSource::File(path.to_path_buf()),
    })
}

fn resolve_contract_entry(
    idx: usize,
    entry: &ContractEntry,
    local: &HashMap<String, (ClassHash, String)>,
) -> Result<DeployStep> {
    let (class_hash, class_name) = if let Some((hash, name)) = local.get(&entry.class) {
        (*hash, name.clone())
    } else if let Some(embedded) = embedded::get(&entry.class) {
        (embedded.class_hash, embedded.name.to_string())
    } else {
        return Err(anyhow!(
            "contract #{idx}: unknown class `{}` (validate() should have caught this)",
            entry.class
        ));
    };

    Ok(DeployStep {
        label: entry.label.clone(),
        class_hash,
        class_name,
        salt: entry.salt.unwrap_or(Felt::ZERO),
        unique: entry.unique,
        calldata: entry.calldata.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_embedded_only_manifest() {
        let manifest: Manifest = toml::from_str(
            r#"
            schema = 1
            [[class]]
            name = "acc"
            embedded = "dev_account"

            [[contract]]
            class = "acc"
            label = "alice"
            salt = "0x1"
            calldata = ["0x42"]
            "#,
        )
        .unwrap();
        manifest.validate().unwrap();

        let plan = BootstrapPlan::from_manifest(&manifest).unwrap();
        assert_eq!(plan.declares.len(), 1);
        assert_eq!(plan.deploys.len(), 1);
        assert_eq!(plan.deploys[0].class_hash, plan.declares[0].class_hash);
        assert_eq!(plan.deploys[0].salt, Felt::from(1u32));
        assert_eq!(plan.deploys[0].label.as_deref(), Some("alice"));
    }

    #[test]
    fn resolves_deploy_referencing_embedded_directly() {
        let manifest: Manifest = toml::from_str(
            r#"
            schema = 1
            [[contract]]
            class = "dev_account"
            calldata = ["0x1"]
            "#,
        )
        .unwrap();
        manifest.validate().unwrap();

        let plan = BootstrapPlan::from_manifest(&manifest).unwrap();
        assert!(plan.declares.is_empty());
        assert_eq!(plan.deploys.len(), 1);
        assert_eq!(plan.deploys[0].class_name, "dev_account");
    }
}
