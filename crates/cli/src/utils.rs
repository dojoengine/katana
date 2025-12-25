use std::path::PathBuf;

use anyhow::{anyhow, Context, Result};
use console::Style;
use katana_chain_spec::rollup::ChainConfigDir;
use katana_chain_spec::ChainSpec;
use katana_genesis::allocation::GenesisAccountAlloc;
use katana_genesis::constant::{
    DEFAULT_LEGACY_ERC20_CLASS_HASH, DEFAULT_LEGACY_UDC_CLASS_HASH, DEFAULT_UDC_ADDRESS,
};
use katana_genesis::json::GenesisJson;
use katana_genesis::Genesis;
use katana_primitives::block::{BlockHash, BlockHashOrNumber, BlockNumber};
use katana_primitives::cairo::ShortString;
use katana_primitives::chain::ChainId;
use katana_primitives::class::ClassHash;
use katana_primitives::contract::ContractAddress;
use katana_rpc_server::cors::HeaderValue;
use katana_tracing::LogFormat;
use serde::{Deserialize, Deserializer, Serializer};
use tracing::info;

use crate::args::LOG_TARGET;
use crate::SequencerNodeArgs;

pub fn parse_seed(seed: &str) -> [u8; 32] {
    let seed = seed.as_bytes();

    if seed.len() >= 32 {
        unsafe { *(seed[..32].as_ptr() as *const [u8; 32]) }
    } else {
        let mut actual_seed = [0u8; 32];
        seed.iter().enumerate().for_each(|(i, b)| actual_seed[i] = *b);
        actual_seed
    }
}

/// Used as clap value parser for [Genesis].
pub fn parse_genesis(value: &str) -> Result<Genesis> {
    let path = PathBuf::from(shellexpand::full(value)?.into_owned());
    let genesis = Genesis::try_from(GenesisJson::load(path)?)?;
    Ok(genesis)
}

/// If the value starts with `0x`, it is parsed as a [`BlockHash`], otherwise as a [`BlockNumber`].
pub fn parse_block_hash_or_number(value: &str) -> Result<BlockHashOrNumber> {
    if value.starts_with("0x") {
        Ok(BlockHashOrNumber::Hash(BlockHash::from_hex(value)?))
    } else {
        let num = value.parse::<BlockNumber>().context("could not parse block number")?;
        Ok(BlockHashOrNumber::Num(num))
    }
}

pub fn print_intro(args: &SequencerNodeArgs, chain: &ChainSpec) {
    let mut accounts = chain.genesis().accounts().peekable();
    let account_class_hash = accounts.peek().map(|e| e.1.class_hash());
    let seed = &args.development.seed;

    if args.logging.stdout.stdout_format == LogFormat::Json {
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
        ChainSpec::Dev(cs) => {
            println!(
                r"
PREDEPLOYED CONTRACTS
==================

| Contract        | ETH Fee Token
| Address         | {}
| Class Hash      | {:#064x}

| Contract        | STRK Fee Token
| Address         | {}
| Class Hash      | {:#064x}",
                cs.fee_contracts.eth,
                DEFAULT_LEGACY_ERC20_CLASS_HASH,
                cs.fee_contracts.strk,
                DEFAULT_LEGACY_ERC20_CLASS_HASH
            );
        }

        ChainSpec::Rollup(cs) => {
            println!(
                r"
PREDEPLOYED CONTRACTS
==================

| Contract        | STRK Fee Token
| Address         | {}
| Class Hash      | {:#064x}",
                cs.fee_contracts.strk, DEFAULT_LEGACY_ERC20_CLASS_HASH,
            );
        }

        ChainSpec::FullNode(..) => {}
    }

    println!(
        r"
| Contract        | Universal Deployer
| Address         | {}
| Class Hash      | {:#064x}",
        DEFAULT_UDC_ADDRESS, DEFAULT_LEGACY_UDC_CLASS_HASH
    );

    if let Some(hash) = account_class_hash {
        println!(
            r"
| Contract        | Account Contract
| Class Hash      | {hash:#064x}"
        )
    }
}

fn print_genesis_accounts<'a, Accounts>(accounts: Accounts)
where
    Accounts: Iterator<Item = (&'a ContractAddress, &'a GenesisAccountAlloc)>,
{
    println!(
        r"

PREFUNDED ACCOUNTS
=================="
    );

    for (addr, account) in accounts {
        if let Some(pk) = account.private_key() {
            println!(
                r"
| Account address |  {addr}
| Private key     |  {pk:#x}
| Public key      |  {:#x}",
                account.public_key()
            )
        } else {
            println!(
                r"
| Account address |  {addr}
| Public key      |  {:#x}",
                account.public_key()
            )
        }
    }
}

pub fn serialize_cors_origins<S>(values: &[HeaderValue], serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let string = values
        .iter()
        .map(|v| v.to_str())
        .collect::<Result<Vec<_>, _>>()
        .map_err(serde::ser::Error::custom)?
        .join(",");

    serializer.serialize_str(&string)
}

pub fn deserialize_cors_origins<'de, D>(deserializer: D) -> Result<Vec<HeaderValue>, D::Error>
where
    D: Deserializer<'de>,
{
    String::deserialize(deserializer)?
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(HeaderValue::from_str)
        .collect::<Result<Vec<HeaderValue>, _>>()
        .map_err(serde::de::Error::custom)
}

// Chain IDs can be arbitrary ASCII strings, making them indistinguishable from filesystem paths.
// To handle this ambiguity, we first try parsing single-component inputs as paths, then as chain
// IDs. Multi-component inputs are always treated as paths.
pub fn parse_chain_config_dir(value: &str) -> Result<ChainConfigDir> {
    let path = PathBuf::from(value);

    if path.components().count() == 1 {
        if path.exists() {
            Ok(ChainConfigDir::open(path)?)
        } else if let Ok(id) = ChainId::parse(value) {
            Ok(ChainConfigDir::open_local(&id)?)
        } else {
            Err(anyhow!("Invalid path or chain id"))
        }
    } else {
        let path = PathBuf::from(shellexpand::tilde(value).as_ref());
        Ok(ChainConfigDir::open(path)?)
    }
}

/// A clap value parser for [`ShortString`] that ensures the string is non-empty.
///
/// This is the `ShortString` equivalent of clap's `NonEmptyStringValueParser`.
#[derive(Clone, Debug)]
pub struct ShortStringValueParser;

impl clap::builder::TypedValueParser for ShortStringValueParser {
    type Value = ShortString;

    fn parse_ref(
        &self,
        cmd: &clap::Command,
        arg: Option<&clap::Arg>,
        value: &std::ffi::OsStr,
    ) -> Result<Self::Value, clap::Error> {
        use core::str::FromStr;

        use clap::error::{ContextKind, ContextValue, ErrorKind};

        let value =
            value.to_str().ok_or_else(|| clap::Error::new(ErrorKind::InvalidUtf8).with_cmd(cmd))?;

        if value.is_empty() {
            let mut err = clap::Error::new(ErrorKind::InvalidValue).with_cmd(cmd);
            if let Some(arg) = arg {
                err.insert(ContextKind::InvalidArg, ContextValue::String(arg.to_string()));
            }
            err.insert(ContextKind::InvalidValue, ContextValue::String(value.to_string()));
            return Err(err);
        }

        ShortString::from_str(value).map_err(|e| {
            let mut err = clap::Error::new(ErrorKind::InvalidValue).with_cmd(cmd);
            if let Some(arg) = arg {
                err.insert(ContextKind::InvalidArg, ContextValue::String(arg.to_string()));
            }
            err.insert(ContextKind::InvalidValue, ContextValue::String(value.to_string()));
            err.insert(ContextKind::Custom, ContextValue::String(e.to_string()));
            err
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_genesis_file() {
        let path = "./test-data/genesis.json";
        parse_genesis(path).unwrap();
    }
}
