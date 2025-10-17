use anyhow::{anyhow, Context, Result};
use clap::{Args, Subcommand};
use katana_primitives::block::BlockHashOrNumber;
use katana_primitives::contract::ContractAddress;
use katana_primitives::Felt;
use katana_provider::api::state::{StateFactoryProvider, StateProvider, StateRootProvider};
use katana_provider::providers::db::DbProvider;

use super::open_db_ro;

#[derive(Debug, Args)]
#[cfg_attr(test, derive(PartialEq, Eq, Clone))]
pub struct TrieArgs {
    /// Path to the database directory.
    #[arg(short, long)]
    pub path: String,

    /// Block number to inspect. Defaults to the latest state when omitted.
    #[arg(short, long)]
    pub block: Option<u64>,

    #[command(subcommand)]
    pub command: TrieCommand,
}

#[derive(Debug, Subcommand)]
#[cfg_attr(test, derive(PartialEq, Eq, Clone))]
pub enum TrieCommand {
    /// Inspect the root of the classes trie.
    Classes,
    /// Inspect the root of the contracts trie.
    Contracts,
    /// Inspect the root of a contract's storage trie.
    Storage(StorageArgs),
}

#[derive(Debug, Args, Clone, PartialEq, Eq)]
pub struct StorageArgs {
    /// Address of the contract whose storage trie root should be inspected.
    #[arg(value_name = "CONTRACT_ADDRESS")]
    pub address: ContractAddress,
}

impl TrieArgs {
    pub fn execute(self) -> Result<()> {
        let root = self.root()?;
        println!("{root:#x}");
        Ok(())
    }

    fn root(&self) -> Result<Felt> {
        let state_provider = self.state_provider()?;

        let root = match &self.command {
            TrieCommand::Classes => state_provider.classes_root()?,
            TrieCommand::Contracts => state_provider.contracts_root()?,
            TrieCommand::Storage(storage) => {
                let address = storage.address;
                state_provider
                    .storage_root(address)?
                    .ok_or_else(|| anyhow!("storage trie not found for contract {address:#x}"))?
            }
        };

        Ok(root)
    }

    fn state_provider(&self) -> Result<Box<dyn StateProvider>> {
        let provider = DbProvider::new(open_db_ro(&self.path)?);

        match self.block {
            Some(block) => provider
                .historical(BlockHashOrNumber::Num(block))
                .with_context(|| format!("failed to get state at block {block}"))?
                .ok_or_else(|| anyhow!("block {block} not found")),

            None => provider.latest().context("failed to get latest state"),
        }
    }
}
