//! `katana bootstrap` — clap entry point.
//!
//! Two operating modes:
//!
//! - **Programmatic** — the `declare` and `deploy` subcommands, each optionally combined with a
//!   TOML manifest. Used in scripts and CI.
//! - **Interactive** — a guided wizard, entered when no subcommand is given.
//!
//! Both modes feed into the same [`crate::plan::BootstrapPlan`] -> [`crate::executor::execute`]
//! pipeline, so the only difference between them is *how* the plan is constructed.

use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use clap::{Args, Subcommand};
use katana_primitives::class::ContractClass;
use katana_primitives::{ContractAddress, Felt};
use url::Url;

use crate::executor::{self, ExecutorConfig};
use crate::manifest::Manifest;
use crate::plan::{BootstrapPlan, ClassSource, DeclareStep, DeployStep};
use crate::tui::{self, SignerDefaults};
use crate::{embedded, report};

#[derive(Debug, Args, PartialEq, Eq)]
pub struct BootstrapArgs {
    /// Katana RPC endpoint URL.
    #[arg(long, default_value = "http://localhost:5050", global = true)]
    rpc_url: Url,

    /// Address of the account used to sign declare/deploy transactions.
    #[arg(long, global = true)]
    account: Option<ContractAddress>,

    /// Private key of the signing account.
    #[arg(long, global = true)]
    private_key: Option<Felt>,

    /// Path to a bootstrap manifest TOML file.
    #[arg(long, global = true)]
    manifest: Option<PathBuf>,

    /// Emit a machine-readable JSON report instead of the human-readable tables.
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Option<Commands>,
}

/// The `bootstrap` subcommands. When omitted, the interactive wizard is launched.
#[derive(Debug, Subcommand, PartialEq, Eq)]
enum Commands {
    /// Declare one or more classes.
    Declare(DeclareArgs),
    /// Deploy one or more contracts.
    Deploy(DeployArgs),
}

#[derive(Debug, Args, PartialEq, Eq)]
struct DeclareArgs {
    /// Class to declare: either an embedded class name (e.g. `dev_account`) or a path to a
    /// Sierra class JSON. Repeat the argument to declare several classes. Combined with any
    /// classes declared by the `--manifest`.
    ///
    /// `long_help` lists the embedded class names from the registry; the short `-h` help
    /// keeps the doc-comment summary above.
    #[arg(value_name = "NAME_OR_PATH", long_help = declare_long_help())]
    classes: Vec<String>,
}

/// Full `--help` text for the `declare` positional: the summary plus the live list of
/// embedded class names sourced from [`embedded::REGISTRY`].
fn declare_long_help() -> String {
    format!(
        "Class to declare: either an embedded class name or a path to a Sierra class \
         JSON.\nRepeat the argument to declare several classes. Combined with any classes \
         declared by the --manifest.\n\nEmbedded classes:\n{}",
        embedded::help_listing()
    )
}

#[derive(Debug, Args, PartialEq, Eq)]
struct DeployArgs {
    /// Contract to deploy. Format:
    /// `<class>[:label=<L>][,salt=0x..][,calldata=0x..,0x..][,unique]`. `<class>` must
    /// reference an embedded class name or a class declared by the `--manifest`. Repeat the
    /// argument to deploy several contracts.
    #[arg(value_name = "SPEC")]
    specs: Vec<String>,
}

impl BootstrapArgs {
    pub async fn execute(self) -> Result<()> {
        // No subcommand → interactive wizard. The TUI collects --account / --private-key in
        // its Settings tab if they weren't passed on the CLI, so we don't validate them here.
        let Some(command) = &self.command else {
            let initial =
                if let Some(path) = &self.manifest { Some(Manifest::load(path)?) } else { None };
            let defaults = SignerDefaults {
                rpc_url: Some(self.rpc_url.to_string()),
                account: self.account,
                private_key: self.private_key,
            };
            tui::run(initial, defaults).await?;
            return Ok(());
        };

        let cfg = self.executor_config()?;
        let plan = match command {
            Commands::Declare(args) => self.build_declare_plan(&args.classes)?,
            Commands::Deploy(args) => self.build_deploy_plan(&args.specs)?,
        };
        let report = executor::execute(&plan, &cfg).await?;
        if self.json {
            report::print_json(&report);
        } else {
            report::print(&report);
        }
        Ok(())
    }

    fn executor_config(&self) -> Result<ExecutorConfig> {
        let account = self.account.ok_or_else(|| anyhow!("--account is required"))?;
        let private_key = self.private_key.ok_or_else(|| anyhow!("--private-key is required"))?;
        Ok(ExecutorConfig { rpc_url: self.rpc_url.clone(), account_address: account, private_key })
    }

    /// Build a plan declaring `classes` on top of any manifest plan. Each class is either an
    /// embedded class name or a Sierra path.
    fn build_declare_plan(&self, classes: &[String]) -> Result<BootstrapPlan> {
        let mut plan = match &self.manifest {
            Some(path) => BootstrapPlan::from_manifest(&Manifest::load(path)?)?,
            None => BootstrapPlan::default(),
        };

        for spec in classes {
            let step = resolve_declare(spec)?;
            if plan.declares.iter().any(|d| d.name == step.name) {
                return Err(anyhow!(
                    "duplicate class alias `{}` between manifest and `declare` arguments",
                    step.name
                ));
            }
            plan.declares.push(step);
        }

        Ok(plan)
    }

    /// Build a plan deploying `specs` on top of any manifest plan. Each spec resolves against
    /// the manifest's declared classes plus the embedded registry.
    fn build_deploy_plan(&self, specs: &[String]) -> Result<BootstrapPlan> {
        let mut plan = match &self.manifest {
            Some(path) => BootstrapPlan::from_manifest(&Manifest::load(path)?)?,
            None => BootstrapPlan::default(),
        };

        for spec in specs {
            let step = parse_deploy(spec, &plan.declares)?;
            plan.deploys.push(step);
        }

        Ok(plan)
    }
}

fn resolve_declare(spec: &str) -> Result<DeclareStep> {
    if let Some(entry) = embedded::get(spec) {
        return Ok(DeclareStep {
            name: entry.name.to_string(),
            class: Arc::new(entry.class()),
            class_hash: entry.class_hash,
            casm_hash: entry.casm_hash,
            source: ClassSource::Embedded(entry.name),
        });
    }

    let path = PathBuf::from(spec);
    if !path.is_file() {
        return Err(anyhow!(
            "declare `{spec}`: not a known embedded class and not a readable file"
        ));
    }
    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let class = ContractClass::from_str(&raw)
        .with_context(|| format!("invalid sierra json {}", path.display()))?;
    if class.is_legacy() {
        return Err(anyhow!("declare `{spec}`: legacy classes are not supported"));
    }
    let class_hash = class.class_hash()?;
    let casm_hash = class.clone().compile()?.class_hash()?;

    // Use the file stem as the local alias for cross-referencing in --deploy.
    let alias = path
        .file_stem()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("{class_hash:#x}"));

    Ok(DeclareStep {
        name: alias,
        class: Arc::new(class),
        class_hash,
        casm_hash,
        source: ClassSource::File(path),
    })
}

/// Parse a `deploy` spec.
///
/// Grammar (informal):
/// ```text
/// SPEC := CLASS [: KV (, KV)*]
/// KV   := label=<str>
///       | salt=<felt>
///       | calldata=<felt>(,<felt>)*    -- consumes the rest of the spec
///       | unique
/// ```
///
/// Because `calldata` is comma-separated and the top-level KV separator is also `,`,
/// `calldata` must be the last KV in the spec. This is documented in the CLI help.
fn parse_deploy(spec: &str, declares: &[DeclareStep]) -> Result<DeployStep> {
    let (class, rest) = match spec.split_once(':') {
        Some((c, r)) => (c.trim(), Some(r)),
        None => (spec.trim(), None),
    };
    if class.is_empty() {
        return Err(anyhow!("deploy `{spec}`: missing class reference"));
    }

    // Resolve the class against the manifest's declare list, then the embedded registry.
    let (class_hash, class_name) = if let Some(d) = declares.iter().find(|d| d.name == class) {
        (d.class_hash, d.name.clone())
    } else if let Some(e) = embedded::get(class) {
        (e.class_hash, e.name.to_string())
    } else {
        return Err(anyhow!(
            "deploy `{spec}`: unknown class `{class}` (not in --manifest and not an embedded \
             class)"
        ));
    };

    let mut label = None;
    let mut salt = Felt::ZERO;
    let mut unique = false;
    let mut calldata: Vec<Felt> = Vec::new();

    if let Some(rest) = rest {
        let mut remaining = rest;
        while !remaining.is_empty() {
            // `calldata=...` swallows the rest of the spec.
            if let Some(stripped) = remaining.strip_prefix("calldata=") {
                calldata = stripped
                    .split(',')
                    .filter(|s| !s.is_empty())
                    .map(|s| {
                        Felt::from_str(s.trim())
                            .map_err(|e| anyhow!("deploy `{spec}`: invalid felt `{s}`: {e}"))
                    })
                    .collect::<Result<Vec<_>>>()?;
                break;
            }

            let (head, tail) = match remaining.split_once(',') {
                Some((h, t)) => (h, t),
                None => (remaining, ""),
            };
            let head = head.trim();
            if head == "unique" {
                unique = true;
            } else if let Some(v) = head.strip_prefix("label=") {
                label = Some(v.to_string());
            } else if let Some(v) = head.strip_prefix("salt=") {
                salt = Felt::from_str(v.trim())
                    .map_err(|e| anyhow!("deploy `{spec}`: invalid salt: {e}"))?;
            } else if !head.is_empty() {
                return Err(anyhow!("deploy `{spec}`: unknown key `{head}`"));
            }
            remaining = tail;
        }
    }

    Ok(DeployStep { label, class_hash, class_name, salt, unique, calldata })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_deploy_minimal() {
        let s = parse_deploy("dev_account", &[]).unwrap();
        assert_eq!(s.class_name, "dev_account");
        assert!(s.label.is_none());
        assert_eq!(s.salt, Felt::ZERO);
        assert!(!s.unique);
        assert!(s.calldata.is_empty());
    }

    #[test]
    fn parse_deploy_with_all_kvs() {
        let s = parse_deploy("dev_account:label=alice,salt=0x7,unique,calldata=0x1,0x2,0x3", &[])
            .unwrap();
        assert_eq!(s.label.as_deref(), Some("alice"));
        assert_eq!(s.salt, Felt::from(7u32));
        assert!(s.unique);
        assert_eq!(s.calldata, vec![Felt::from(1u32), Felt::from(2u32), Felt::from(3u32)]);
    }

    #[test]
    fn parse_deploy_unknown_class_errors() {
        let err = parse_deploy("ghost", &[]).unwrap_err().to_string();
        assert!(err.contains("unknown class"));
    }

    #[test]
    fn parse_deploy_unknown_key_errors() {
        let err = parse_deploy("dev_account:foo=bar", &[]).unwrap_err().to_string();
        assert!(err.contains("unknown key"));
    }

    /// Thin `Parser` wrapper so we can exercise clap's parsing of the `Args`-derived
    /// `BootstrapArgs` exactly as the `katana bootstrap ...` subcommand mounts it.
    #[derive(Debug, clap::Parser)]
    struct TestCli {
        #[command(flatten)]
        args: BootstrapArgs,
    }

    fn parse(argv: &[&str]) -> BootstrapArgs {
        use clap::Parser;
        TestCli::try_parse_from(argv).unwrap().args
    }

    #[test]
    fn no_subcommand_is_interactive() {
        let args = parse(&["bootstrap"]);
        assert_eq!(args.command, None);
    }

    #[test]
    fn declare_subcommand_collects_classes() {
        let args = parse(&["bootstrap", "declare", "dev_account", "udc"]);
        match args.command {
            Some(Commands::Declare(d)) => assert_eq!(d.classes, vec!["dev_account", "udc"]),
            other => panic!("expected declare subcommand, got {other:?}"),
        }
    }

    #[test]
    fn deploy_subcommand_collects_specs() {
        let args = parse(&["bootstrap", "deploy", "dev_account:label=alice"]);
        match args.command {
            Some(Commands::Deploy(d)) => assert_eq!(d.specs, vec!["dev_account:label=alice"]),
            other => panic!("expected deploy subcommand, got {other:?}"),
        }
    }

    /// The `declare --help` text lists the embedded class names sourced from the registry,
    /// so they're discoverable without reading the source.
    #[test]
    fn declare_help_lists_embedded_classes() {
        use clap::CommandFactory;
        let mut cmd = TestCli::command();
        let declare = cmd.find_subcommand_mut("declare").expect("declare subcommand exists");
        let help = declare.render_long_help().to_string();
        for name in embedded::REGISTRY.iter().map(|c| c.name) {
            assert!(help.contains(name), "declare --help should list `{name}`:\n{help}");
        }
    }

    /// Global signer/connection flags resolve whether they appear before or after the
    /// subcommand — that's the point of `global = true`.
    #[test]
    fn global_flags_resolve_after_subcommand() {
        let args = parse(&[
            "bootstrap",
            "declare",
            "dev_account",
            "--account",
            "0x1",
            "--private-key",
            "0x2",
        ]);
        assert_eq!(args.account, Some(ContractAddress::from(Felt::ONE)));
        assert_eq!(args.private_key, Some(Felt::from(2u32)));
    }
}
