//! `katana bootstrap` — clap entry point.
//!
//! Two operating modes:
//!
//! - **Programmatic** — driven by CLI flags and/or a TOML manifest. Used in scripts and CI.
//! - **Interactive** — a guided wizard, entered when `--interactive` is passed or when no
//!   actionable inputs are present.
//!
//! Both modes feed into the same [`crate::plan::BootstrapPlan`] -> [`crate::executor::execute`]
//! pipeline, so the only difference between them is *how* the plan is constructed.

use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use clap::Args;
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
    /// Force interactive wizard even if other flags are present.
    #[arg(long)]
    interactive: bool,

    /// Katana RPC endpoint URL.
    #[arg(long, default_value = "http://localhost:5050")]
    rpc_url: Url,

    /// Address of the account used to sign declare/deploy transactions.
    #[arg(long)]
    account: Option<ContractAddress>,

    /// Private key of the signing account.
    #[arg(long)]
    private_key: Option<Felt>,

    /// Path to a bootstrap manifest TOML file.
    #[arg(long)]
    manifest: Option<PathBuf>,

    /// Declare a class. Argument is either an embedded class name (e.g. `dev_account`) or
    /// a path to a Sierra class JSON. May be repeated.
    #[arg(long = "declare", value_name = "NAME_OR_PATH")]
    declares: Vec<String>,

    /// Deploy a contract. Format: `<class>[:label=<L>][,salt=0x..][,calldata=0x..,0x..][,unique]`.
    /// `<class>` must reference either a `--declare` alias used in this invocation, an
    /// embedded class name, or a class declared by the `--manifest`. May be repeated.
    #[arg(long = "deploy", value_name = "SPEC")]
    deploys: Vec<String>,
}

impl BootstrapArgs {
    pub async fn execute(self) -> Result<()> {
        // Decide mode. If --interactive is set, or no actionable inputs are present,
        // run the TUI. Otherwise build a plan from the flags/manifest.
        let no_inputs =
            self.manifest.is_none() && self.declares.is_empty() && self.deploys.is_empty();

        if self.interactive || no_inputs {
            // The TUI collects --account / --private-key in its Settings tab if they
            // weren't passed on the CLI, so we don't validate them here.
            let initial =
                if let Some(path) = &self.manifest { Some(Manifest::load(path)?) } else { None };
            let defaults = SignerDefaults {
                rpc_url: Some(self.rpc_url.to_string()),
                account: self.account,
                private_key: self.private_key,
            };
            tui::run(initial, defaults).await?;
            return Ok(());
        }

        let cfg = self.executor_config()?;
        let plan = self.build_programmatic_plan()?;
        let report = executor::execute(&plan, &cfg).await?;
        report::print(&report);
        Ok(())
    }

    fn executor_config(&self) -> Result<ExecutorConfig> {
        let account = self.account.ok_or_else(|| anyhow!("--account is required"))?;
        let private_key = self.private_key.ok_or_else(|| anyhow!("--private-key is required"))?;
        Ok(ExecutorConfig { rpc_url: self.rpc_url.clone(), account_address: account, private_key })
    }

    fn build_programmatic_plan(&self) -> Result<BootstrapPlan> {
        // Start from the manifest (if any) so its declares are visible to flag-driven deploys.
        let mut plan = if let Some(path) = &self.manifest {
            let manifest = Manifest::load(path)?;
            BootstrapPlan::from_manifest(&manifest)?
        } else {
            BootstrapPlan::default()
        };

        // Append --declare flags. Each is either an embedded class name or a Sierra path.
        for spec in &self.declares {
            let step = resolve_declare_flag(spec)?;
            if plan.declares.iter().any(|d| d.name == step.name) {
                return Err(anyhow!(
                    "duplicate class alias `{}` between manifest and --declare flags",
                    step.name
                ));
            }
            plan.declares.push(step);
        }

        // Append --deploy flags. Resolves against the now-fully-populated declare list
        // plus the embedded registry.
        for spec in &self.deploys {
            let step = parse_deploy_flag(spec, &plan.declares)?;
            plan.deploys.push(step);
        }

        Ok(plan)
    }
}

fn resolve_declare_flag(spec: &str) -> Result<DeclareStep> {
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
            "--declare `{spec}`: not a known embedded class and not a readable file"
        ));
    }
    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let class = ContractClass::from_str(&raw)
        .with_context(|| format!("invalid sierra json {}", path.display()))?;
    if class.is_legacy() {
        return Err(anyhow!("--declare `{spec}`: legacy classes are not supported"));
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

/// Parse a `--deploy` spec.
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
fn parse_deploy_flag(spec: &str, declares: &[DeclareStep]) -> Result<DeployStep> {
    let (class, rest) = match spec.split_once(':') {
        Some((c, r)) => (c.trim(), Some(r)),
        None => (spec.trim(), None),
    };
    if class.is_empty() {
        return Err(anyhow!("--deploy `{spec}`: missing class reference"));
    }

    // Resolve the class against this invocation's declare list, then the embedded registry.
    let (class_hash, class_name) = if let Some(d) = declares.iter().find(|d| d.name == class) {
        (d.class_hash, d.name.clone())
    } else if let Some(e) = embedded::get(class) {
        (e.class_hash, e.name.to_string())
    } else {
        return Err(anyhow!(
            "--deploy `{spec}`: unknown class `{class}` (not in --declare/--manifest and not an \
             embedded class)"
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
                            .map_err(|e| anyhow!("--deploy `{spec}`: invalid felt `{s}`: {e}"))
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
                    .map_err(|e| anyhow!("--deploy `{spec}`: invalid salt: {e}"))?;
            } else if !head.is_empty() {
                return Err(anyhow!("--deploy `{spec}`: unknown key `{head}`"));
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
        let s = parse_deploy_flag("dev_account", &[]).unwrap();
        assert_eq!(s.class_name, "dev_account");
        assert!(s.label.is_none());
        assert_eq!(s.salt, Felt::ZERO);
        assert!(!s.unique);
        assert!(s.calldata.is_empty());
    }

    #[test]
    fn parse_deploy_with_all_kvs() {
        let s =
            parse_deploy_flag("dev_account:label=alice,salt=0x7,unique,calldata=0x1,0x2,0x3", &[])
                .unwrap();
        assert_eq!(s.label.as_deref(), Some("alice"));
        assert_eq!(s.salt, Felt::from(7u32));
        assert!(s.unique);
        assert_eq!(s.calldata, vec![Felt::from(1u32), Felt::from(2u32), Felt::from(3u32)]);
    }

    #[test]
    fn parse_deploy_unknown_class_errors() {
        let err = parse_deploy_flag("ghost", &[]).unwrap_err().to_string();
        assert!(err.contains("unknown class"));
    }

    #[test]
    fn parse_deploy_unknown_key_errors() {
        let err = parse_deploy_flag("dev_account:foo=bar", &[]).unwrap_err().to_string();
        assert!(err.contains("unknown key"));
    }
}
