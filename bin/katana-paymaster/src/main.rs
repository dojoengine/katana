use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::{Parser, ValueEnum};
use paymaster_prices::mock::MockPriceOracle;
use paymaster_prices::TokenPrice;
use paymaster_relayer::lock::mock::MockLockLayer;
use paymaster_relayer::lock::{LockLayerConfiguration, RelayerLock};
use paymaster_relayer::rebalancing::OptionalRebalancingConfiguration;
use paymaster_relayer::RelayersConfiguration;
use paymaster_rpc::server::PaymasterServer;
use paymaster_rpc::{Configuration, RPCConfiguration};
use paymaster_sponsoring::{Configuration as SponsoringConfiguration, SelfConfiguration};
use paymaster_starknet::constants::Token;
use paymaster_starknet::{ChainID, Configuration as StarknetConfiguration, StarknetAccountConfiguration};
use starknet::core::types::Felt;
use tokio::signal;
use tracing::{info, warn};

const DEFAULT_API_KEY: &str = "paymaster_katana";

#[derive(ValueEnum, Clone, Copy, Debug)]
enum SupportedToken {
    Eth,
    Strk,
}

#[derive(Parser, Debug)]
#[command(version, about = "Katana Avnu paymaster sidecar", long_about = None)]
struct Args {
    /// RPC port to bind the paymaster server on.
    #[arg(long, default_value_t = 8081)]
    port: u16,

    /// Starknet JSON-RPC URL for the Katana node.
    #[arg(long, value_name = "URL")]
    rpc_url: String,

    /// Chain id identifier (SN_SEPOLIA or SN_MAIN).
    #[arg(long, value_name = "CHAIN_ID")]
    chain_id: String,

    /// Forwarder contract address.
    #[arg(long, value_name = "ADDRESS", value_parser = parse_felt)]
    forwarder: Felt,

    /// Paymaster account address.
    #[arg(long, value_name = "ADDRESS", value_parser = parse_felt)]
    account_address: Felt,

    /// Paymaster account private key.
    #[arg(long, value_name = "KEY", value_parser = parse_felt)]
    account_private_key: Felt,

    /// API key used for sponsored transactions (must start with `paymaster_`).
    #[arg(long, value_name = "KEY")]
    api_key: Option<String>,

    /// Supported tokens (defaults to ETH + STRK).
    #[arg(long = "supported-token", value_enum)]
    supported_tokens: Vec<SupportedToken>,
}

#[derive(Debug, Clone)]
struct MockPriceOracleImpl;

#[async_trait::async_trait]
impl MockPriceOracle for MockPriceOracleImpl {
    fn new() -> Self {
        Self
    }

    async fn fetch_token(&self, address: Felt) -> Result<TokenPrice, paymaster_prices::Error> {
        Ok(TokenPrice { address, price_in_strk: Felt::from(1_000_000_000_000_000_000u128), decimals: 18 })
    }
}

#[derive(Debug)]
struct MockLockingLayer {
    relayer_address: Felt,
}

impl MockLockingLayer {
    fn new(relayer_address: Felt) -> Self {
        Self { relayer_address }
    }
}

#[async_trait::async_trait]
impl MockLockLayer for MockLockingLayer {
    fn new() -> Self {
        Self { relayer_address: Felt::ZERO }
    }

    async fn count_enabled_relayers(&self) -> usize {
        1
    }

    async fn set_enabled_relayers(&self, _relayers: &HashSet<Felt>) {}

    async fn lock_relayer(&self) -> Result<RelayerLock, paymaster_relayer::lock::Error> {
        Ok(RelayerLock::new(self.relayer_address, None, Duration::from_secs(30)))
    }

    async fn release_relayer(
        &self,
        _lock: RelayerLock,
    ) -> Result<(), paymaster_relayer::lock::Error> {
        Ok(())
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    let args = Args::parse();
    let chain_id = ChainID::from_string(&args.chain_id)
        .with_context(|| format!("unsupported chain id: {}", args.chain_id))?;

    let mut api_key = args.api_key.unwrap_or_else(|| DEFAULT_API_KEY.to_string());
    if !api_key.starts_with("paymaster_") {
        warn!(%api_key, "API key must start with 'paymaster_'; falling back to default key");
        api_key = DEFAULT_API_KEY.to_string();
    }

    let supported_tokens = resolve_supported_tokens(&args.supported_tokens);
    let account = StarknetAccountConfiguration {
        address: args.account_address,
        private_key: args.account_private_key,
    };

    let relayers = RelayersConfiguration {
        private_key: args.account_private_key,
        addresses: vec![args.account_address],
        min_relayer_balance: Felt::ZERO,
        lock: LockLayerConfiguration::Mock {
            retry_timeout: Duration::from_secs(5),
            lock_layer: Arc::new(MockLockingLayer::new(args.account_address)),
        },
        rebalancing: OptionalRebalancingConfiguration::initialize(None),
    };

    let configuration = Configuration {
        rpc: RPCConfiguration { port: args.port as u64 },
        supported_tokens,
        forwarder: args.forwarder,
        gas_tank: account,
        estimate_account: account,
        relayers,
        max_fee_multiplier: 3.0,
        provider_fee_overhead: 0.1,
        starknet: StarknetConfiguration {
            chain_id,
            endpoint: args.rpc_url.clone(),
            timeout: 30,
            fallbacks: vec![],
        },
        price: paymaster_prices::Configuration::Mock(Arc::new(MockPriceOracleImpl)),
        sponsoring: SponsoringConfiguration::SelfSponsoring(SelfConfiguration {
            api_key,
            sponsor_metadata: vec![],
        }),
    };

    let server = PaymasterServer::new(&configuration);
    let handle = server.start().await.context("failed to start paymaster server")?;
    info!(port = args.port, "Paymaster sidecar started");

    signal::ctrl_c().await.context("failed to install ctrl-c handler")?;
    handle.stop().context("failed to stop paymaster server")?;
    handle.stopped().await;

    Ok(())
}

fn resolve_supported_tokens(tokens: &[SupportedToken]) -> HashSet<Felt> {
    if tokens.is_empty() {
        return HashSet::from([Token::ETH_ADDRESS, Token::STRK_ADDRESS]);
    }

    tokens
        .iter()
        .map(|token| match token {
            SupportedToken::Eth => Token::ETH_ADDRESS,
            SupportedToken::Strk => Token::STRK_ADDRESS,
        })
        .collect()
}

fn parse_felt(value: &str) -> std::result::Result<Felt, String> {
    if value.starts_with("0x") || value.starts_with("0X") {
        Felt::from_hex(value).map_err(|e| e.to_string())
    } else {
        Felt::from_dec_str(value).map_err(|e| e.to_string())
    }
}
