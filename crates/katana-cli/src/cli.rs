use std::{process::exit, sync::Arc};

use clap::Parser;
use env_logger::Env;
use katana_core::sequencer::KatanaSequencer;
use katana_rpc::KatanaRpc;
use log::error;
use tokio::sync::RwLock;
use yansi::Paint;

mod config;

use config::Cli;

#[tokio::main]
async fn main() {
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();

    let config = Cli::parse();
    let rpc_config = config.rpc_config();
    let starknet_config = config.starknet_config();

    let sequencer = Arc::new(RwLock::new(KatanaSequencer::new(starknet_config)));
    sequencer.write().await.start();

    let predeployed_accounts = if config.hide_predeployed_accounts {
        None
    } else {
        Some(
            sequencer
                .read()
                .await
                .starknet
                .predeployed_accounts
                .display(),
        )
    };

    match KatanaRpc::new(sequencer.clone(), rpc_config).run().await {
        Ok((addr, server_handle)) => {
            print_intro(
                predeployed_accounts,
                config.seed,
                format!(
                    "🚀 JSON-RPC server started: {}",
                    Paint::red(format!("http://{addr}"))
                ),
            );

            server_handle.stopped().await;
        }
        Err(err) => {
            error! {"{}", err};
            exit(1);
        }
    };
}

fn print_intro(accounts: Option<String>, seed: Option<String>, address: String) {
    println!(
        "{}",
        Paint::red(
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

    if let Some(accounts) = accounts {
        println!(
            r"        
PREFUNDED ACCOUNTS
==================
{accounts}
    "
        );
    }

    if let Some(seed) = seed {
        println!(
            r"
ACCOUNTS SEED
=============
{seed}
    "
        );
    }

    println!("\n{address}\n\n");
}
