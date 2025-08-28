use console::Style;
use katana_chain_spec::ChainSpec;
use katana_cli_args::NodeArgs;
use katana_primitives::class::ClassHash;
use katana_primitives::contract::ContractAddress;
use katana_primitives::genesis::allocation::GenesisAccountAlloc;
use katana_primitives::genesis::constant::{
    DEFAULT_LEGACY_ERC20_CLASS_HASH, DEFAULT_LEGACY_UDC_CLASS_HASH, DEFAULT_UDC_ADDRESS,
};
use katana_tracing::LogFormat;
use tracing::info;

use crate::exec::LOG_TARGET;

pub fn print_intro(args: &NodeArgs, chain: &ChainSpec) {
    let mut accounts = chain.genesis().accounts().peekable();
    let account_class_hash = accounts.peek().map(|e| e.1.class_hash());
    let seed = &args.development.seed;

    if args.logging.log_format == LogFormat::Json {
        info!(
            target: LOG_TARGET,
            "{}",
            serde_json::json!({
                "accounts": accounts.map(|a| serde_json::json!(a)).collect::<Vec<_>>(),
                "seed": format!("{}", seed),
            })
        )
    } else {
        println!(
            "{}",
            Style::new().red().apply_to(
                r"


██╗  ██╗ █████╗ ████████╗ █████╗ ███╗   ██╗ █████╗
██║ ██╔╝██╔══██╗╚══██╔══╝██╔══██╗████╗  ██║██╔══██╗
█████╔╝ ███████║   ██║   ███████║██╔██╗ ██║███████║
██╔═██╗ ██╔══██║   ██║   ██╔══██║██║╚██╗██║██╔══██║
██║  ██╗██║  ██║   ██║   ██║  ██║██║ ╚████║██║  ██║
╚═╝  ╚═╝╚═╝  ╚═╝   ╚═╝   ╚═╝  ╚═╝╚═╝  ╚═══╝╚═╝  ╚═╝
"
            )
        );

        print_genesis_contracts(chain, account_class_hash);
        print_genesis_accounts(accounts);

        println!(
            r"

ACCOUNTS SEED
=============
{seed}
    "
        );
    }
}

fn print_genesis_contracts(chain: &ChainSpec, account_class_hash: Option<ClassHash>) {
    match chain {
        ChainSpec::Dev(_) | ChainSpec::Rollup(_) => {
            println!(
                r"

PREFUNDED ACCOUNTS
=================="
            );

            if let Some(hash) = account_class_hash {
                println!("Class hash: {:#x}\n", hash);
            }
        }
    }
}

fn print_genesis_accounts(
    accounts: impl Iterator<Item = (ContractAddress, GenesisAccountAlloc)>,
) {
    for (addr, account) in accounts {
        if let Some(pk) = account.private_key() {
            println!("| Account address |  {addr:#x}");
            println!("| Private key     |  {pk:#x}");
            println!("| Public key      |  {:#x}", account.public_key());
        } else {
            println!("| Account address |  {addr:#x}");
            println!("| Public key      |  {:#x}", account.public_key());
        }

        println!();
    }

    println!(
        r"
PREFUNDED CONTRACTS
===================

| Contract        | Udc
| Address         | {DEFAULT_UDC_ADDRESS:#x}
| Class hash      | {DEFAULT_LEGACY_UDC_CLASS_HASH:#x}

| Contract        | ERC20 Mock
| Class hash      | {DEFAULT_LEGACY_ERC20_CLASS_HASH:#x}",
    );
}