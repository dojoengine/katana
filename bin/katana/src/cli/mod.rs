use std::future::Future;

use anyhow::{Context, Result};
use clap::{Args, CommandFactory, Parser, Subcommand};
use clap_complete::Shell;
use katana_cli::{NodeCli, SequencerNodeArgs};
use tokio::runtime::Runtime;

mod config;
pub mod db;
mod init;
mod stage;
mod version;

#[cfg(feature = "client")]
mod rpc;

use version::{generate_long, generate_short};

#[derive(Debug, Parser)]
#[cfg_attr(test, derive(PartialEq))]
#[command(name = "katana", author, version = generate_short(), long_version = generate_long() ,about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    commands: Option<Commands>,

    #[command(flatten)]
    node: SequencerNodeArgs,
}

impl Cli {
    pub fn run(self) -> Result<()> {
        if let Some(cmd) = self.commands {
            return match cmd {
                Commands::Db(args) => args.execute(),
                Commands::Config(args) => args.execute(),
                Commands::Stage(args) => args.execute(),
                Commands::Completions(args) => args.execute(),
                Commands::Init(args) => execute_async(args.execute())?,
                #[cfg(feature = "client")]
                Commands::Rpc(args) => execute_async(args.execute())?,
                Commands::Node(args) => execute_async(args.execute())?,
            };
        }

        execute_async(self.node.with_config_file()?.execute())?
    }
}

#[derive(Debug, Subcommand)]
#[cfg_attr(test, derive(PartialEq))]
pub enum Commands {
    #[command(about = "Initialize chain")]
    Init(Box<init::InitCommand>),

    #[command(about = "Chain configuration utilities")]
    Config(config::ConfigArgs),

    #[command(about = "Database utilities")]
    Db(db::DbArgs),

    #[command(about = "Syncing stage utilities")]
    Stage(stage::StageArgs),

    #[command(about = "Generate shell completion file for specified shell")]
    Completions(CompletionsArgs),

    #[cfg(feature = "client")]
    #[command(about = "RPC client for interacting with Katana")]
    Rpc(rpc::RpcArgs),

    #[command(hide = true)]
    #[command(about = "Run and manage Katana nodes")]
    Node(NodeCli),
}

#[derive(Debug, Args)]
#[cfg_attr(test, derive(PartialEq))]
pub struct CompletionsArgs {
    pub shell: Shell,
}

impl CompletionsArgs {
    fn execute(self) -> Result<()> {
        let mut command = Cli::command();
        let name = command.get_name().to_string();
        clap_complete::generate(self.shell, &mut command, name, &mut std::io::stdout());
        Ok(())
    }
}

pub fn execute_async<F: Future>(future: F) -> Result<F::Output> {
    Ok(build_tokio_runtime().context("Failed to build tokio runtime")?.block_on(future))
}

fn build_tokio_runtime() -> std::io::Result<Runtime> {
    tokio::runtime::Builder::new_multi_thread().enable_all().build()
}

#[cfg(test)]
mod tests {
    use katana_cli::NodeSubcommand;

    use super::*;

    #[test]
    fn default_command_is_sequencer() {
        let cli_no_subcommand = Cli::parse_from(["katana"]);
        let cli_explicit_sequencer = Cli::parse_from(["katana", "node", "sequencer"]);

        assert!(cli_no_subcommand.commands.is_none());
        assert!(matches!(cli_explicit_sequencer.commands, Some(Commands::Node(_))));

        let config_default = cli_no_subcommand.node.config().unwrap();
        let config_explicit =
            cli_explicit_sequencer.node.with_config_file().unwrap().config().unwrap();

        assert_eq!(config_default.chain.id(), config_explicit.chain.id());
        assert_eq!(config_default.dev.fee, config_explicit.dev.fee);
        assert_eq!(config_default.dev.account_validation, config_explicit.dev.account_validation);
        assert_eq!(config_default.sequencing.block_time, config_explicit.sequencing.block_time);
        assert_eq!(config_default.sequencing.no_mining, config_explicit.sequencing.no_mining);
    }

    #[test]
    fn default_command_with_flags() {
        let args_default = ["katana", "--dev", "--dev.no-fee", "--block-time", "1000"];
        let args_explicit =
            ["katana", "node", "sequencer", "--dev", "--dev.no-fee", "--block-time", "1000"];

        let cli_default = Cli::parse_from(args_default);
        let cli_explicit = Cli::parse_from(args_explicit);

        assert!(cli_default.commands.is_none());
        assert!(matches!(cli_explicit.commands, Some(Commands::Node(_))));

        let Commands::Node(NodeCli { command: NodeSubcommand::Sequencer(explicit_node_args) }) =
            &cli_explicit.commands.unwrap()
        else {
            panic!("Expected Node command");
        };

        similar_asserts::assert_eq!(&cli_default.node, explicit_node_args.as_ref());

        let config_default = cli_default.node.config().unwrap();
        let config_explicit = explicit_node_args.config().unwrap();

        assert!(!config_default.dev.fee);
        assert!(!config_explicit.dev.fee);
        assert_eq!(config_default.sequencing.block_time, Some(1000));
        assert_eq!(config_explicit.sequencing.block_time, Some(1000));

        // assert that the rest of the configurations is equal
        similar_asserts::assert_eq!(config_default, config_explicit);
    }
}
