use std::path::PathBuf;

use anyhow::{Context, Result};
use katana_chain_spec::rollup::ChainConfigDir;
use katana_primitives::block::{BlockHash, BlockHashOrNumber, BlockNumber};
use katana_primitives::genesis::json::GenesisJson;
use katana_primitives::genesis::Genesis;
#[cfg(feature = "server")]
use katana_rpc::cors::HeaderValue;
use serde::{Deserialize, Deserializer, Serializer};

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

pub fn parse_chain_config_dir(value: &str) -> Result<ChainConfigDir> {
    let path = PathBuf::from(shellexpand::full(value)?.into_owned());
    Ok(ChainConfigDir(path))
}

#[cfg(feature = "server")]
pub fn serialize_cors_origins<S>(
    values: &[HeaderValue],
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let strings: Vec<String> = values.iter().map(|v| v.to_str().unwrap().to_string()).collect();
    strings.serialize(serializer)
}

#[cfg(feature = "server")]
pub fn deserialize_cors_origins<'de, D>(deserializer: D) -> Result<Vec<HeaderValue>, D::Error>
where
    D: Deserializer<'de>,
{
    let strings: Vec<String> = Vec::deserialize(deserializer)?;
    strings
        .into_iter()
        .map(|s| HeaderValue::from_str(&s).map_err(serde::de::Error::custom))
        .collect()
}