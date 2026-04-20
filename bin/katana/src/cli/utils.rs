use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::Result;
use cairo_lang_starknet_classes::abi;
use clap::{Args, Subcommand};
use katana_primitives::class::{ContractClass, MaybeInvalidSierraContractAbi};
use katana_primitives::utils::get_selector_from_name;
use katana_primitives::Felt;

#[derive(Debug, Args)]
#[cfg_attr(test, derive(PartialEq))]
pub struct UtilsArgs {
    #[command(subcommand)]
    command: UtilsCommands,
}

#[derive(Debug, Subcommand)]
#[cfg_attr(test, derive(PartialEq))]
enum UtilsCommands {
    #[command(about = "Get the selector for a contract entrypoint from its name")]
    Selector(SelectorArgs),

    #[command(about = "Display a contract class's entry points with their human-readable names")]
    ClassAbi(ClassAbiArgs),
}

#[derive(Debug, Args)]
#[cfg_attr(test, derive(PartialEq))]
struct SelectorArgs {
    /// The human-readable function name (e.g. "transfer").
    name: String,
}

#[derive(Debug, Args)]
#[cfg_attr(test, derive(PartialEq))]
struct ClassAbiArgs {
    /// Path to a contract class JSON artifact (.contract_class.json).
    path: PathBuf,
}

impl UtilsArgs {
    pub fn execute(self) -> Result<()> {
        match self.command {
            UtilsCommands::Selector(args) => {
                let selector = get_selector_from_name(&args.name);
                println!("{selector:#x}");
                Ok(())
            }
            UtilsCommands::ClassAbi(args) => {
                let json = std::fs::read_to_string(&args.path)?;
                let class: ContractClass = json.parse()?;
                display_class_entry_points(&class);
                Ok(())
            }
        }
    }
}

fn display_class_entry_points(class: &ContractClass) {
    match class {
        ContractClass::Class(sierra) => {
            // Build a selector → name lookup from the ABI.
            let name_map = sierra.abi.as_ref().map(build_selector_name_map).unwrap_or_default();

            let eps = &sierra.entry_points_by_type;

            print_entry_points("EXTERNAL", &eps.external, &name_map);
            print_entry_points("L1_HANDLER", &eps.l1_handler, &name_map);
            print_entry_points("CONSTRUCTOR", &eps.constructor, &name_map);
        }

        ContractClass::Legacy(_legacy) => {
            println!("Legacy (Cairo 0) classes are not yet supported by this command.");
        }
    }
}

/// Builds a mapping from selector (Felt) to human-readable function name by walking the ABI.
fn build_selector_name_map(abi: &MaybeInvalidSierraContractAbi) -> HashMap<Felt, String> {
    let MaybeInvalidSierraContractAbi::Valid(contract_abi) = abi else {
        return HashMap::new();
    };

    // Collect all interfaces so we can resolve Impl items.
    let mut interfaces: HashMap<String, &[abi::Item]> = HashMap::new();
    for item in contract_abi.clone() {
        if let abi::Item::Interface(ref iface) = item {
            interfaces.insert(iface.name.clone(), &[]);
        }
    }

    // Two-pass: first collect interface references, then collect names.
    // We need owned data since we can't hold references into the iterator.
    let items: Vec<abi::Item> = contract_abi.clone().into_iter().collect();

    let mut interface_map: HashMap<String, Vec<String>> = HashMap::new();
    for item in &items {
        if let abi::Item::Interface(iface) = item {
            let names: Vec<String> = iface
                .items
                .iter()
                .filter_map(|item| match item {
                    abi::Item::Function(f) => Some(f.name.clone()),
                    _ => None,
                })
                .collect();
            interface_map.insert(iface.name.clone(), names);
        }
    }

    let mut map = HashMap::new();

    for item in &items {
        match item {
            abi::Item::Function(f) => {
                let selector = get_selector_from_name(&f.name);
                map.insert(selector, f.name.clone());
            }
            abi::Item::Constructor(c) => {
                let selector = get_selector_from_name(&c.name);
                map.insert(selector, c.name.clone());
            }
            abi::Item::L1Handler(h) => {
                let selector = get_selector_from_name(&h.name);
                map.insert(selector, h.name.clone());
            }
            abi::Item::Impl(imp) => {
                // Resolve the interface's functions as entry points.
                if let Some(names) = interface_map.get(&imp.interface_name) {
                    for name in names {
                        let selector = get_selector_from_name(name);
                        map.insert(selector, name.clone());
                    }
                }
            }
            _ => {}
        }
    }

    map
}

fn print_entry_points(
    label: &str,
    entry_points: &[cairo_lang_starknet_classes::contract_class::ContractEntryPoint],
    name_map: &HashMap<Felt, String>,
) {
    if entry_points.is_empty() {
        return;
    }

    println!("{label}:");
    for ep in entry_points {
        let selector: Felt = ep.selector.clone().into();
        let name = name_map.get(&selector).map(|s| s.as_str()).unwrap_or("<unknown>");
        println!("  {name:<40} {selector:#x}");
    }
    println!();
}
