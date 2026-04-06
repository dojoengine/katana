//! Interactive wizard for `katana bootstrap` (when no manifest/flags are provided).
//!
//! Walks the user through:
//!   1. picking classes to declare (embedded or from disk),
//!   2. picking contracts to deploy (referencing those classes),
//!   3. reviewing the resulting plan,
//!   4. (optionally) saving the equivalent manifest to disk for replay.
//!
//! All prompts are powered by `inquire` and follow the existing
//! `bin/katana/src/cli/init/prompt.rs` style.

use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

use anyhow::Result;
use inquire::{Confirm, CustomType, Select, Text};
use katana_primitives::class::ContractClass;
use katana_primitives::Felt;

use super::embedded::{self, EmbeddedClass};
use super::manifest::{ClassEntry, ContractEntry, Manifest};
use super::plan::{BootstrapPlan, ClassSource, DeclareStep, DeployStep};

/// Run the interactive session and return both the plan to execute and the
/// equivalent manifest (so we can offer to write it after success).
pub fn run() -> Result<(BootstrapPlan, Manifest)> {
    println!("Bootstrap wizard. Walks you through declaring classes and deploying contracts.\n");

    let declares = declare_loop()?;
    let deploys = deploy_loop(&declares)?;

    let plan = BootstrapPlan { declares: declares.clone(), deploys: deploys.clone() };
    let manifest = build_manifest(&declares, &deploys);

    println!("\nPlan:");
    for d in &plan.declares {
        println!("  declare  {}  ({:#x})", d.name, d.class_hash);
    }
    for d in &plan.deploys {
        let label = d.label.as_deref().unwrap_or("-");
        println!("  deploy   {} as {}  (class {})", d.class_name, label, d.class_name);
    }

    if !Confirm::new("Execute this plan?").with_default(true).prompt()? {
        anyhow::bail!("aborted by user");
    }

    Ok((plan, manifest))
}

/// After execution, optionally persist the equivalent manifest. Done as a separate
/// step so the user only sees this prompt once we know the bootstrap actually worked.
pub fn maybe_save_manifest(manifest: &Manifest) -> Result<()> {
    if !Confirm::new("Save manifest for replay?").with_default(false).prompt()? {
        return Ok(());
    }

    let raw = Text::new("Manifest path").with_default("./bootstrap.toml").prompt()?;
    let path = PathBuf::from(raw);

    let serialized = toml::to_string_pretty(manifest)?;
    std::fs::write(&path, serialized)?;
    println!("Wrote {}", path.display());
    Ok(())
}

// ---------- declare loop -------------------------------------------------------------

fn declare_loop() -> Result<Vec<DeclareStep>> {
    let mut declares: Vec<DeclareStep> = Vec::new();
    let mut next_id = 1usize;

    loop {
        let action = Select::new(
            "Add a class to declare?",
            vec![DeclareAction::Embedded, DeclareAction::FromFile, DeclareAction::Done],
        )
        .prompt()?;

        match action {
            DeclareAction::Done => break,
            DeclareAction::Embedded => {
                let entry = pick_embedded()?;
                let local = unique_name(&entry.name, &declares);
                declares.push(DeclareStep {
                    name: local,
                    class: Arc::new(entry.class()),
                    class_hash: entry.class_hash,
                    casm_hash: entry.casm_hash,
                    source: ClassSource::Embedded(entry.name),
                });
            }
            DeclareAction::FromFile => {
                let raw = Text::new("Sierra class JSON path")
                    .with_validator(|s: &str| {
                        if std::path::Path::new(s).is_file() {
                            Ok(inquire::validator::Validation::Valid)
                        } else {
                            Ok(inquire::validator::Validation::Invalid(
                                "file does not exist".into(),
                            ))
                        }
                    })
                    .prompt()?;
                let path = PathBuf::from(raw);
                let local = Text::new("Local alias for this class")
                    .with_default(&format!("class_{next_id}"))
                    .prompt()?;
                next_id += 1;
                let step = load_file_class(&local, &path)?;
                declares.push(step);
            }
        }
    }

    Ok(declares)
}

#[derive(Debug, Clone)]
enum DeclareAction {
    Embedded,
    FromFile,
    Done,
}

impl std::fmt::Display for DeclareAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DeclareAction::Embedded => write!(f, "Pick a built-in class"),
            DeclareAction::FromFile => write!(f, "Load a Sierra class from disk"),
            DeclareAction::Done => write!(f, "Done"),
        }
    }
}

fn pick_embedded() -> Result<&'static EmbeddedClass> {
    let options: Vec<EmbeddedPick> =
        embedded::REGISTRY.iter().map(|c| EmbeddedPick(c)).collect();
    let picked = Select::new("Built-in class", options).prompt()?;
    Ok(picked.0)
}

struct EmbeddedPick(&'static EmbeddedClass);

impl std::fmt::Display for EmbeddedPick {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} — {}", self.0.name, self.0.description)
    }
}

fn unique_name(base: &str, existing: &[DeclareStep]) -> String {
    if !existing.iter().any(|d| d.name == base) {
        return base.to_string();
    }
    let mut i = 2;
    loop {
        let candidate = format!("{base}_{i}");
        if !existing.iter().any(|d| d.name == candidate) {
            return candidate;
        }
        i += 1;
    }
}

fn load_file_class(name: &str, path: &std::path::Path) -> Result<DeclareStep> {
    let raw = std::fs::read_to_string(path)?;
    let class = ContractClass::from_str(&raw)?;
    if class.is_legacy() {
        anyhow::bail!("legacy (Cairo 0) classes are not supported");
    }
    let class_hash = class.class_hash()?;
    let casm_hash = class.clone().compile()?.class_hash()?;
    println!("  computed class hash: {class_hash:#x}");
    Ok(DeclareStep {
        name: name.to_string(),
        class: Arc::new(class),
        class_hash,
        casm_hash,
        source: ClassSource::File(path.to_path_buf()),
    })
}

// ---------- deploy loop --------------------------------------------------------------

fn deploy_loop(declares: &[DeclareStep]) -> Result<Vec<DeployStep>> {
    let mut deploys = Vec::new();

    loop {
        if !Confirm::new("Add a contract to deploy?").with_default(true).prompt()? {
            break;
        }

        let class_pick = pick_class_for_deploy(declares)?;
        let label = Text::new("Label (optional)")
            .with_default("")
            .prompt()?;
        let label = if label.is_empty() { None } else { Some(label) };

        let salt = CustomType::<Felt>::new("Salt")
            .with_default(Felt::ZERO)
            .with_formatter(&|v: Felt| format!("{v:#x}"))
            .prompt()?;

        let unique =
            Confirm::new("UDC unique?").with_default(false).prompt()?;

        let calldata = prompt_calldata()?;

        deploys.push(DeployStep {
            label,
            class_hash: class_pick.class_hash,
            class_name: class_pick.name,
            salt,
            unique,
            calldata,
        });
    }

    Ok(deploys)
}

#[derive(Clone)]
struct ClassPick {
    name: String,
    class_hash: katana_primitives::class::ClassHash,
}

impl std::fmt::Display for ClassPick {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({:#x})", self.name, self.class_hash)
    }
}

fn pick_class_for_deploy(declares: &[DeclareStep]) -> Result<ClassPick> {
    let mut options: Vec<ClassPick> = declares
        .iter()
        .map(|d| ClassPick { name: d.name.clone(), class_hash: d.class_hash })
        .collect();
    // Also offer any embedded class directly, even if the user didn't add it as a
    // declare step — bootstrap will skip the declare if the node already has it.
    for entry in embedded::REGISTRY {
        if !options.iter().any(|o| o.name == entry.name) {
            options.push(ClassPick { name: entry.name.to_string(), class_hash: entry.class_hash });
        }
    }
    Ok(Select::new("Class to deploy", options).prompt()?)
}

fn prompt_calldata() -> Result<Vec<Felt>> {
    let raw = Text::new("Constructor calldata (comma-separated felts, blank for none)")
        .with_default("")
        .prompt()?;
    if raw.trim().is_empty() {
        return Ok(Vec::new());
    }
    raw.split(',')
        .map(|s| Felt::from_str(s.trim()).map_err(|e| anyhow::anyhow!("invalid felt `{s}`: {e}")))
        .collect()
}

// ---------- manifest serialization ---------------------------------------------------

fn build_manifest(declares: &[DeclareStep], deploys: &[DeployStep]) -> Manifest {
    let classes = declares
        .iter()
        .map(|d| match &d.source {
            ClassSource::Embedded(name) => ClassEntry {
                name: d.name.clone(),
                embedded: Some((*name).to_string()),
                path: None,
            },
            ClassSource::File(path) => ClassEntry {
                name: d.name.clone(),
                embedded: None,
                path: Some(path.clone()),
            },
        })
        .collect();

    let contracts = deploys
        .iter()
        .map(|d| ContractEntry {
            class: d.class_name.clone(),
            label: d.label.clone(),
            salt: if d.salt == Felt::ZERO { None } else { Some(d.salt) },
            unique: d.unique,
            calldata: d.calldata.clone(),
        })
        .collect();

    Manifest { schema: 1, classes, contracts }
}
