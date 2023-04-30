use std::{process::exit, sync::Arc};

use clap::Parser;
use env_logger::Env;
use katana_core::{sequencer::KatanaSequencer, starknet::Config};
use katana_rpc::{config::RpcConfig, KatanaRpc};
use log::error;
use yansi::Paint;

#[tokio::main]
async fn main() {
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();

    let config = Cli::parse();
    let rpc_config = config.get_rpc_config();
    let sequencer = Arc::new(KatanaSequencer::new(Config));
    let predeployed_accounts = sequencer
        .starknet
        .read()
        .unwrap()
        .predeployed_accounts
        .display();

    match KatanaRpc::new(sequencer, rpc_config).run().await {
        Ok((addr, server_handle)) => {
            print_title();

            println!("{predeployed_accounts}");

            println!(
                "\n\n🚀 JSON-RPC server started: {}\n",
                Paint::red(format!("http://{addr}"))
            );

            server_handle.stopped().await;
        }
        Err(err) => {
            error! {"{}", err};
            exit(1);
        }
    };
}

#[derive(Parser, Debug)]
struct Cli {
    #[arg(short, long)]
    #[arg(default_value = "5050")]
    #[arg(help = "Port number to listen on.")]
    port: u16,
}

impl Cli {
    fn get_rpc_config(&self) -> RpcConfig {
        RpcConfig { port: self.port }
    }
}

fn print_title() {
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
}
