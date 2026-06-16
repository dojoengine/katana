use std::net::IpAddr;

use katana_chain_spec::ChainSpec;
use katana_node_config::build_info::BuildInfo;
use katana_node_config::rpc::RpcModulesList;
use katana_primitives::block::{BlockHashOrNumber, GasPrices};
use katana_primitives::{ContractAddress, Felt};
use serde::{Deserialize, Serialize};

/// Identity and build information for a running Katana node.
///
/// On full nodes, `chain_id` and `chain_kind` reflect the *configured* chain (via
/// `config.network`), not the chain observed from the upstream sync source. Operators
/// who point a sync source at the wrong network will still see their configured
/// identity reported here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeInfo {
    /// Semver-ish version string, e.g. `"1.0.0-alpha.19-dev"`.
    /// Does not embed the git SHA (see `git_sha`).
    pub version: String,
    /// Git commit SHA of the build, e.g. `"77d4800"`. `"unknown"` for non-CLI builds.
    pub git_sha: String,
    /// Build timestamp in ISO 8601. `"unknown"` for non-CLI builds.
    pub build_timestamp: String,
    /// Names of enabled compile-time features. Currently only `"native"` is reported;
    /// detection for additional features (tee, grpc, cartridge) is not wired up. Empty
    /// for non-CLI builds.
    pub features: Vec<String>,
    /// Chain identifier as a raw Felt (hex-encoded, same wire format as
    /// `starknet_chainId`).
    pub chain_id: Felt,
    /// Role of this node: sequencer (producing blocks) or full node (following a chain).
    pub chain_kind: ChainKind,
    /// `true` if the node is running a development chain (`ChainSpec::Dev`). Orthogonal
    /// to `chain_kind`: a sequencer can be dev or production (rollup); a full node is
    /// never dev.
    pub dev: bool,
}

impl NodeInfo {
    /// Construct from a build-info snapshot and a chain spec. Both node implementations
    /// call this at registration time so the wire format stays consistent.
    pub fn from_parts(build_info: &BuildInfo, chain_spec: &ChainSpec) -> Self {
        Self {
            version: build_info.version.clone(),
            git_sha: build_info.git_sha.clone(),
            build_timestamp: build_info.build_timestamp.clone(),
            features: build_info.features.clone(),
            chain_id: chain_spec.id().id(),
            chain_kind: ChainKind::from(chain_spec),
            dev: matches!(chain_spec, ChainSpec::Dev(_)),
        }
    }
}

/// Role of the node: sequencer (producing blocks) or full node (following a chain).
///
/// Serialized as PascalCase (`"Sequencer"`, `"FullNode"`) rather than camelCase,
/// matching how enum tags are conventionally written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChainKind {
    Sequencer,
    FullNode,
}

impl From<&ChainSpec> for ChainKind {
    fn from(spec: &ChainSpec) -> Self {
        match spec {
            // Dev and Rollup are both sequencer-mode chain specs; the dev/prod distinction
            // is captured separately on `NodeInfo::dev`.
            ChainSpec::Dev(_) | ChainSpec::Rollup(_) => Self::Sequencer,
            ChainSpec::FullNode(_) => Self::FullNode,
        }
    }
}

/// The runtime configuration of a sequencer node, as returned by `node_getConfig`.
///
/// This is a serializable *mirror* of the node's internal `Config`, not the internal type
/// itself: several internal fields (`http::HeaderValue`, `Duration`, `url::Url`, the full
/// `ChainSpec`) don't serialize cleanly, so they are projected onto wire-friendly forms
/// (strings, milliseconds, `chain_id`). The assembly from the internal `Config` lives in the
/// sequencer crate (the dependency direction forbids `rpc-types` from importing it), mirroring
/// how [`NodeInfo`] is built at registration time.
///
/// Secrets (paymaster API key and deployer private key) are exposed verbatim — this is intended
/// for local development sequencers where introspecting the full config is the point.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeConfig {
    /// Chain identifier as a raw Felt (same wire format as `starknet_chainId`). The full chain
    /// spec / genesis is intentionally not dumped here; chain identity is covered by
    /// `node_getInfo`.
    pub chain_id: Felt,
    /// Database options.
    pub db: DbConfigDump,
    /// Forking options, if the node was started in forking mode.
    pub forking: Option<ForkingConfigDump>,
    /// JSON-RPC server options.
    pub rpc: RpcConfigDump,
    /// Feeder gateway options, if enabled.
    pub gateway: Option<GatewayConfigDump>,
    /// Metrics server options, if enabled.
    pub metrics: Option<MetricsConfigDump>,
    /// Transaction execution options.
    pub execution: ExecutionConfigDump,
    /// L1<>L2 messaging options, if enabled.
    pub messaging: Option<MessagingConfigDump>,
    /// Block production options.
    pub sequencing: SequencingConfigDump,
    /// Development-mode options.
    pub dev: DevConfigDump,
    /// Cartridge paymaster options, if enabled. Includes secrets verbatim.
    pub paymaster: Option<PaymasterConfigDump>,
    /// TEE attestation options, if enabled.
    pub tee: Option<TeeConfigDump>,
    /// gRPC server options. `None` when the `grpc` feature is disabled or it is not configured.
    pub grpc: Option<GrpcConfigDump>,
}

/// Database options. Mirror of the internal `DbConfig`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DbConfigDump {
    /// Path to the database directory, if persisted to disk.
    pub dir: Option<String>,
}

/// Forking options. Mirror of the internal `ForkingConfig`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForkingConfigDump {
    /// The JSON-RPC URL of the network being forked from.
    pub url: String,
    /// The block forked from. `None` means the latest block at startup.
    pub block: Option<BlockHashOrNumber>,
}

/// L1<>L2 messaging options. Mirror of the internal `MessagingConfig`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessagingConfigDump {
    /// The settlement chain being watched for messages.
    #[serde(flatten)]
    pub settlement: SettlementChainDump,
    /// Poll interval, in seconds, for new settlement-chain blocks.
    pub interval: u64,
    /// The settlement-chain block from which message gathering starts on first run.
    pub from_block: u64,
    /// Number of confirmations to wait before a settlement-chain block is considered safe.
    pub confirmation_depth: u64,
}

/// The settlement chain a messaging service watches. Mirror of the internal
/// `SettlementChainConfig`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "chain", rename_all = "camelCase")]
pub enum SettlementChainDump {
    /// An Ethereum settlement chain.
    #[serde(rename = "ethereum")]
    Ethereum {
        /// The RPC URL of the settlement chain.
        rpc_url: String,
        /// The messaging contract address on the settlement chain.
        contract_address: String,
    },
    /// A Starknet settlement chain.
    #[serde(rename = "starknet")]
    Starknet {
        /// The RPC URL of the settlement chain.
        rpc_url: String,
        /// The messaging contract address on the settlement chain.
        contract_address: String,
    },
}

/// JSON-RPC server options. Mirror of the internal `RpcConfig`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcConfigDump {
    /// The IP address the RPC server is bound to.
    pub addr: IpAddr,
    /// The port the RPC server listens on.
    pub port: u16,
    /// Whether the built-in explorer is served. `None` when the `explorer` feature is disabled.
    pub explorer: Option<bool>,
    /// The set of enabled RPC modules.
    pub apis: RpcModulesList,
    /// Allowed CORS origins (header values rendered as strings).
    pub cors_origins: Vec<String>,
    /// Maximum number of concurrent connections, if capped.
    pub max_connections: Option<u32>,
    /// Maximum number of concurrent `estimateFee` requests, if capped.
    pub max_concurrent_estimate_fee_requests: Option<u32>,
    /// Maximum request body size in bytes, if capped.
    pub max_request_body_size: Option<u32>,
    /// Maximum response body size in bytes, if capped.
    pub max_response_body_size: Option<u32>,
    /// Per-request timeout in milliseconds, if set.
    pub timeout_millis: Option<u64>,
    /// Maximum number of keys for `starknet_getStorageProof`, if capped.
    pub max_proof_keys: Option<u64>,
    /// Maximum page size for `starknet_getEvents`, if capped.
    pub max_event_page_size: Option<u64>,
    /// Maximum gas for `starknet_call`, if capped.
    pub max_call_gas: Option<u64>,
}

/// Feeder gateway options. Mirror of the internal `GatewayConfig`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayConfigDump {
    /// The IP address the gateway server is bound to.
    pub addr: IpAddr,
    /// The port the gateway server listens on.
    pub port: u16,
    /// Response timeout in milliseconds. `None` means wait indefinitely.
    pub timeout_millis: Option<u64>,
}

/// Metrics server options. Mirror of the internal `MetricsConfig`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricsConfigDump {
    /// The IP address the metrics server is bound to.
    pub addr: IpAddr,
    /// The port the metrics server listens on.
    pub port: u16,
}

/// Transaction execution options. Mirror of the internal `ExecutionConfig`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionConfigDump {
    /// Maximum Cairo steps for a transaction's `__execute__`.
    pub invocation_max_steps: u32,
    /// Maximum Cairo steps for a transaction's `__validate__`.
    pub validation_max_steps: u32,
    /// Maximum contract call recursion depth.
    pub max_recursion_depth: u64,
    /// Whether Cairo-native compilation is enabled. `None` when the `native` feature is disabled.
    pub compile_native: Option<bool>,
}

/// Block production options. Mirror of the internal `SequencingConfig`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SequencingConfigDump {
    /// Interval block time in milliseconds. `None` means instant mining.
    pub block_time: Option<u64>,
    /// Whether automatic block production is disabled (manual mining only).
    pub no_mining: bool,
    /// Maximum accumulated Cairo steps per block, if capped.
    pub block_cairo_steps_limit: Option<u64>,
    /// Whether state-trie computation is disabled during block production.
    pub no_state_trie: bool,
}

/// Development-mode options. Mirror of the internal `DevConfig`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DevConfigDump {
    /// Whether transaction fees are charged.
    pub fee: bool,
    /// Whether account validation is performed.
    pub account_validation: bool,
    /// Fixed L1 gas prices, if overridden for development.
    pub fixed_gas_prices: Option<FixedGasPricesDump>,
}

/// Fixed development gas prices. Mirror of the internal `FixedL1GasPriceConfig`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FixedGasPricesDump {
    /// L2 gas prices.
    pub l2_gas_prices: GasPrices,
    /// L1 gas prices.
    pub l1_gas_prices: GasPrices,
    /// L1 data gas prices.
    pub l1_data_gas_prices: GasPrices,
}

/// Cartridge paymaster options. Mirror of the internal `PaymasterConfig`. Secrets verbatim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaymasterConfigDump {
    /// Paymaster service URL.
    pub url: String,
    /// Optional API key for authentication (verbatim).
    pub api_key: Option<String>,
    /// Cartridge-specific paymaster integration options.
    pub cartridge_api: Option<CartridgeApiConfigDump>,
}

/// Cartridge paymaster integration options. Mirror of the internal `CartridgeApiConfig`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CartridgeApiConfigDump {
    /// Cartridge API URL.
    pub cartridge_api_url: String,
    /// The paymaster account address used for deploying controllers.
    pub controller_deployer_address: ContractAddress,
    /// The paymaster account private key used for deploying controllers (verbatim).
    pub controller_deployer_private_key: Felt,
    /// VRF service options.
    pub vrf: Option<VrfConfigDump>,
}

/// VRF service options. Mirror of the internal `VrfConfig`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VrfConfigDump {
    /// The VRF service URL.
    pub url: String,
    /// The address of the VRF account contract.
    pub vrf_account: ContractAddress,
}

/// TEE attestation options. Mirror of the internal `TeeConfig`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeeConfigDump {
    /// The TEE provider type used for attestation (e.g. `"sev-snp"`).
    pub provider_type: String,
    /// The block number Katana forked from, included in TEE report data.
    pub fork_block_number: Option<u64>,
}

/// gRPC server options. Mirror of the internal `GrpcConfig`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GrpcConfigDump {
    /// The IP address the gRPC server is bound to.
    pub addr: IpAddr,
    /// The port the gRPC server listens on.
    pub port: u16,
    /// Request timeout in milliseconds, if set.
    pub timeout_millis: Option<u64>,
}

#[cfg(test)]
mod tests {
    use katana_chain_spec::ChainSpec;
    use katana_node_config::build_info::BuildInfo;
    use serde_json::json;

    use super::{ChainKind, NodeInfo};

    #[test]
    fn chain_kind_from_chain_spec_dev_is_sequencer() {
        assert_eq!(ChainKind::from(&ChainSpec::dev()), ChainKind::Sequencer);
    }

    #[test]
    fn chain_kind_from_chain_spec_full_node() {
        assert_eq!(ChainKind::from(&ChainSpec::mainnet()), ChainKind::FullNode);
        assert_eq!(ChainKind::from(&ChainSpec::sepolia()), ChainKind::FullNode);
    }

    #[test]
    fn node_info_dev_flag_is_derived_from_chain_spec() {
        let bi = BuildInfo::default();
        assert!(NodeInfo::from_parts(&bi, &ChainSpec::dev()).dev);
        assert!(!NodeInfo::from_parts(&bi, &ChainSpec::mainnet()).dev);
        assert!(!NodeInfo::from_parts(&bi, &ChainSpec::sepolia()).dev);
    }

    #[test]
    fn chain_kind_serializes_as_pascal_case() {
        assert_eq!(serde_json::to_value(ChainKind::Sequencer).unwrap(), json!("Sequencer"));
        assert_eq!(serde_json::to_value(ChainKind::FullNode).unwrap(), json!("FullNode"));
    }

    #[test]
    fn chain_kind_deserializes_pascal_case() {
        assert_eq!(
            serde_json::from_value::<ChainKind>(json!("Sequencer")).unwrap(),
            ChainKind::Sequencer
        );
        assert_eq!(
            serde_json::from_value::<ChainKind>(json!("FullNode")).unwrap(),
            ChainKind::FullNode,
        );
        // Pin the casing: lowercase must fail.
        assert!(serde_json::from_value::<ChainKind>(json!("sequencer")).is_err());
        assert!(serde_json::from_value::<ChainKind>(json!("fullNode")).is_err());
    }

    #[test]
    fn node_info_round_trips_through_serde() {
        let info = NodeInfo {
            version: "1.2.3-dev".into(),
            git_sha: "abcdef0".into(),
            build_timestamp: "2026-04-21T00:00:00Z".into(),
            features: vec!["native".into(), "tee".into()],
            chain_id: ChainSpec::dev().id().id(),
            chain_kind: ChainKind::Sequencer,
            dev: true,
        };

        let json = serde_json::to_value(&info).unwrap();

        // Field names are camelCase; chain_kind value is PascalCase.
        assert_eq!(json["version"], json!("1.2.3-dev"));
        assert_eq!(json["gitSha"], json!("abcdef0"));
        assert_eq!(json["buildTimestamp"], json!("2026-04-21T00:00:00Z"));
        assert_eq!(json["features"], json!(["native", "tee"]));
        assert_eq!(json["chainKind"], json!("Sequencer"));
        assert_eq!(json["dev"], json!(true));
        // chain_id is a raw hex Felt string (same shape as starknet_chainId).
        assert!(
            json["chainId"].is_string(),
            "chainId must serialize as a hex string, not an enum object — got {}",
            json["chainId"]
        );

        let roundtrip: NodeInfo = serde_json::from_value(json).unwrap();
        assert_eq!(roundtrip, info);
    }

    #[test]
    fn node_config_serializes_camel_case_and_round_trips() {
        use katana_node_config::rpc::{RpcModuleKind, RpcModulesList};

        use super::{
            DbConfigDump, DevConfigDump, ExecutionConfigDump, NodeConfig, RpcConfigDump,
            SequencingConfigDump,
        };

        let mut apis = RpcModulesList::new();
        apis.add(RpcModuleKind::Starknet);
        apis.add(RpcModuleKind::Node);

        let config = NodeConfig {
            chain_id: ChainSpec::dev().id().id(),
            db: DbConfigDump { dir: None },
            forking: None,
            rpc: RpcConfigDump {
                addr: "127.0.0.1".parse().unwrap(),
                port: 5050,
                explorer: None,
                apis,
                cors_origins: vec!["*".to_string()],
                max_connections: None,
                max_concurrent_estimate_fee_requests: None,
                max_request_body_size: None,
                max_response_body_size: None,
                timeout_millis: Some(30_000),
                max_proof_keys: Some(100),
                max_event_page_size: Some(1024),
                max_call_gas: Some(1_000_000_000),
            },
            gateway: None,
            metrics: None,
            execution: ExecutionConfigDump {
                invocation_max_steps: 10_000_000,
                validation_max_steps: 1_000_000,
                max_recursion_depth: 1000,
                compile_native: None,
            },
            messaging: None,
            sequencing: SequencingConfigDump {
                block_time: Some(5000),
                no_mining: false,
                block_cairo_steps_limit: None,
                no_state_trie: false,
            },
            dev: DevConfigDump { fee: true, account_validation: true, fixed_gas_prices: None },
            paymaster: None,
            tee: None,
            grpc: None,
        };

        let json = serde_json::to_value(&config).unwrap();

        // Field names are camelCase, and the durations/limits round-trip as numbers.
        assert!(json["chainId"].is_string(), "chainId must be a hex string");
        assert_eq!(json["sequencing"]["blockTime"], json!(5000));
        assert_eq!(json["rpc"]["timeoutMillis"], json!(30_000));
        assert_eq!(json["rpc"]["maxCallGas"], json!(1_000_000_000u64));
        assert_eq!(json["rpc"]["corsOrigins"], json!(["*"]));
        assert_eq!(json["execution"]["maxRecursionDepth"], json!(1000));
        assert_eq!(json["dev"]["accountValidation"], json!(true));

        let roundtrip: NodeConfig = serde_json::from_value(json).unwrap();
        assert_eq!(roundtrip, config);
    }
}
