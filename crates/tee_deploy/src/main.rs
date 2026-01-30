//! Declare and deploy AMDTeeRegistry + KatanaTee on Starknet.
//!
//! Build contracts first:
//!   cd contracts/amd_tee_registry && scarb build
//!   cd contracts/katana_tee && scarb build
//!
//! Then run from repo root:
//!   cargo run -p tee_deploy -- init --private-key 0x... --address 0x... --provider-url https://...

use clap::Parser;
use tee_deploy::init::{run_init, InitArgs};
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(name = "tee_deploy")]
#[command(about = "Declare and deploy AMDTeeRegistry + KatanaTee on Starknet")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(clap::Subcommand, Debug)]
enum Command {
    /// Declare and deploy AMDTeeRegistry and KatanaTee
    Init(InitArgs),
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("tee_deploy=info".parse()?))
        .init();

    let cli = Cli::parse();
    match cli.command {
        Command::Init(args) => {
            run_init(args).await?;
        }
    }
    Ok(())
}
