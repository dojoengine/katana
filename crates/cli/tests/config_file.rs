use std::path::PathBuf;

use assert_matches::assert_matches;
use clap::Parser;
use katana_cli::sequencer::SequencerNodeArgs;
use katana_gas_price_oracle::{
    DEFAULT_ETH_L1_DATA_GAS_PRICE, DEFAULT_STRK_L1_DATA_GAS_PRICE, DEFAULT_STRK_L1_GAS_PRICE,
};
use katana_sequencer_node::config::execution::{
    DEFAULT_INVOCATION_MAX_STEPS, DEFAULT_VALIDATION_MAX_STEPS,
};

/// Write TOML content to a temp file and return its path.
fn write_config(content: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let path = std::env::temp_dir().join(format!("katana-test-config-{pid}-{id}.toml"));
    std::fs::write(&path, content).unwrap();
    path
}

/// Helper: parse args, merge config file, produce final Config.
fn parse_and_config(args: &[&str]) -> katana_sequencer_node::config::Config {
    SequencerNodeArgs::parse_from(args).with_config_file().unwrap().config().unwrap()
}

/// Baseline: all config file values are used when no CLI args are provided.
/// Exercises every merge path in the "file wins" direction:
/// - Option fields (block_time)
/// - Bool fields (no_mining)
/// - Whole-struct replace (gpo, including l2 prices)
/// - Field-level merge (starknet.env, dev options)
#[test]
fn config_file_only() {
    let config_path = write_config(
        r#"
block_time = 5000
no_mining = false

[gpo]
l1_eth_gas_price = "100"
l1_strk_gas_price = "200"
l1_eth_data_gas_price = "10"
l1_strk_data_gas_price = "20"
l2_eth_gas_price = "55"
l2_strk_gas_price = "66"

[dev]
dev = true
no_fee = true
total_accounts = 15
seed = "42"

[starknet.env]
invoke_max_steps = 5000
validate_max_steps = 3000
"#,
    );
    let path_str = config_path.to_string_lossy().to_string();
    let config = parse_and_config(&["katana", "--config", &path_str]);

    // Sequencing
    assert_eq!(config.sequencing.block_time, Some(5000));
    assert!(!config.sequencing.no_mining);

    // Execution steps from file (EnvironmentOptions field-level merge)
    assert_eq!(config.execution.invocation_max_steps, 5000);
    assert_eq!(config.execution.validation_max_steps, 3000);

    // Dev options from file
    assert!(!config.dev.fee); // no_fee = true => fee = false

    // GPO whole-struct replace (all 6 price fields)
    assert_matches!(&config.dev.fixed_gas_prices, Some(prices) => {
        assert_eq!(prices.l1_gas_prices.eth.get(), 100);
        assert_eq!(prices.l1_gas_prices.strk.get(), 200);
        assert_eq!(prices.l1_data_gas_prices.eth.get(), 10);
        assert_eq!(prices.l1_data_gas_prices.strk.get(), 20);
        assert_eq!(prices.l2_gas_prices.eth.get(), 55);
        assert_eq!(prices.l2_gas_prices.strk.get(), 66);
    });
}

/// CLI args take precedence over config file for fields that use field-level merge
/// (block_time, invoke/validate_max_steps, dev.accounts) while file values are
/// used for fields CLI didn't set (dev.no_fee).
#[test]
fn cli_args_override_config_file() {
    let config_path = write_config(
        r#"
block_time = 5000

[dev]
dev = true
no_fee = true
total_accounts = 20

[starknet.env]
invoke_max_steps = 9000
validate_max_steps = 8000
"#,
    );
    let path_str = config_path.to_string_lossy().to_string();

    let config = parse_and_config(&[
        "katana",
        "--config",
        &path_str,
        "--block-time",
        "1000",
        "--invoke-max-steps",
        "200",
        "--validate-max-steps",
        "100",
        "--dev",
        "--dev.accounts",
        "5",
    ]);

    // CLI wins for these fields
    assert_eq!(config.sequencing.block_time, Some(1000));
    assert_eq!(config.execution.invocation_max_steps, 200);
    assert_eq!(config.execution.validation_max_steps, 100);

    // File's no_fee=true wins because CLI no_fee defaults to false
    assert!(!config.dev.fee);
}

/// Partial config: only some fields set in file, unset fields stay at defaults.
/// Tests that a partial gpo section deserializes correctly with serde defaults
/// for unset gas price fields.
#[test]
fn partial_config_file() {
    let config_path = write_config(
        r#"
block_time = 3000

[gpo]
l1_eth_gas_price = "42"
"#,
    );
    let path_str = config_path.to_string_lossy().to_string();
    let config = parse_and_config(&["katana", "--config", &path_str]);

    // Values from file
    assert_eq!(config.sequencing.block_time, Some(3000));
    assert_matches!(&config.dev.fixed_gas_prices, Some(prices) => {
        assert_eq!(prices.l1_gas_prices.eth.get(), 42);
        // Other gas prices should be defaults
        assert_eq!(prices.l1_gas_prices.strk, DEFAULT_STRK_L1_GAS_PRICE);
        assert_eq!(prices.l1_data_gas_prices.eth, DEFAULT_ETH_L1_DATA_GAS_PRICE);
        assert_eq!(prices.l1_data_gas_prices.strk, DEFAULT_STRK_L1_DATA_GAS_PRICE);
    });

    // Defaults for everything else
    assert!(!config.sequencing.no_mining);
    assert_eq!(config.execution.invocation_max_steps, DEFAULT_INVOCATION_MAX_STEPS);
    assert_eq!(config.execution.validation_max_steps, DEFAULT_VALIDATION_MAX_STEPS);
    assert!(config.dev.fee);
    assert!(config.dev.account_validation);
}

/// Tests the no_mining bool merge: `if !self.no_mining { self.no_mining = file_value }`.
/// File sets true, CLI doesn't set it => file's true propagates.
#[test]
fn no_mining_from_file() {
    let config_path = write_config(
        r#"
no_mining = true
"#,
    );
    let path_str = config_path.to_string_lossy().to_string();
    let config = parse_and_config(&["katana", "--config", &path_str]);

    assert!(config.sequencing.no_mining);
}

/// Tests the no_mining bool merge in the other direction: CLI --no-mining
/// sets self.no_mining=true, so the file value is never consulted.
#[test]
fn no_mining_cli_overrides_file() {
    let config_path = write_config(
        r#"
no_mining = false
"#,
    );
    let path_str = config_path.to_string_lossy().to_string();
    let config = parse_and_config(&["katana", "--config", &path_str, "--no-mining"]);

    assert!(config.sequencing.no_mining);
}

/// Tests DbOptions field-level merge: CLI overrides dir while file's
/// migrate=true is preserved (since CLI migrate defaults to false).
#[test]
fn db_options_merge() {
    let config_path = write_config(
        r#"
dir = "/from/file"
migrate = true
"#,
    );
    let path_str = config_path.to_string_lossy().to_string();

    let config = parse_and_config(&["katana", "--config", &path_str, "--data-dir", "/from/cli"]);

    assert_eq!(config.db.dir, Some(PathBuf::from("/from/cli")));
    assert!(config.db.migrate);
}

/// Tests DevOptions field-level merge: CLI overrides seed+accounts,
/// file provides no_fee and no_account_validation.
#[test]
fn dev_options_merge() {
    let config_path = write_config(
        r#"
[dev]
dev = true
seed = "999"
total_accounts = 25
no_fee = true
no_account_validation = true
"#,
    );
    let path_str = config_path.to_string_lossy().to_string();

    let config = parse_and_config(&[
        "katana",
        "--config",
        &path_str,
        "--dev",
        "--dev.seed",
        "123",
        "--dev.accounts",
        "5",
    ]);

    // File's bool flags win because CLI defaults are false
    assert!(!config.dev.fee); // no_fee = true from file
    assert!(!config.dev.account_validation); // no_account_validation = true from file
}

/// Edge case: empty config file should not break anything; all values
/// should remain at their defaults.
#[test]
fn empty_config_file_uses_defaults() {
    let config_path = write_config("");
    let path_str = config_path.to_string_lossy().to_string();
    let config = parse_and_config(&["katana", "--config", &path_str]);

    assert_eq!(config.sequencing.block_time, None);
    assert!(!config.sequencing.no_mining);
    assert_eq!(config.execution.invocation_max_steps, DEFAULT_INVOCATION_MAX_STEPS);
    assert_eq!(config.execution.validation_max_steps, DEFAULT_VALIDATION_MAX_STEPS);
    assert!(config.dev.fee);
    assert!(config.dev.account_validation);
    assert!(config.dev.fixed_gas_prices.is_none());
    assert_eq!(config.db.dir, None);
}

/// Tests the custom `deserialize_gas_price` deserializer handles hex strings
/// (0x-prefix) correctly in the config file.
#[test]
fn gpo_hex_values_from_file() {
    let config_path = write_config(
        r#"
[gpo]
l1_eth_gas_price = "0xff"
l1_strk_gas_price = "0x10"
"#,
    );
    let path_str = config_path.to_string_lossy().to_string();
    let config = parse_and_config(&["katana", "--config", &path_str]);

    assert_matches!(&config.dev.fixed_gas_prices, Some(prices) => {
        assert_eq!(prices.l1_gas_prices.eth.get(), 255);
        assert_eq!(prices.l1_gas_prices.strk.get(), 16);
        assert_eq!(prices.l1_data_gas_prices.eth, DEFAULT_ETH_L1_DATA_GAS_PRICE);
        assert_eq!(prices.l1_data_gas_prices.strk, DEFAULT_STRK_L1_DATA_GAS_PRICE);
    });
}
