//! Assembly of the serializable [`NodeConfig`] dump returned by the `node_getConfig` RPC.
//!
//! The wire types live in `katana-rpc-types` (the dependency direction forbids that crate from
//! importing the node [`Config`]), so the projection from the internal config onto its
//! wire-friendly mirror happens here — mirroring how `NodeInfo` is assembled at registration
//! time. Fields that don't serialize cleanly are converted: URLs and CORS header values become
//! strings, `Duration`s become milliseconds, and the chain spec is reduced to its `chain_id`.

use katana_chain_spec::ChainSpec;
use katana_messaging::SettlementChainConfig;
use katana_rpc_types::node::{
    CartridgeApiConfigDump, DbConfigDump, DevConfigDump, ExecutionConfigDump, FixedGasPricesDump,
    ForkingConfigDump, GatewayConfigDump, GrpcConfigDump, MessagingConfigDump, MetricsConfigDump,
    NodeConfig, PaymasterConfigDump, RpcConfigDump, SequencingConfigDump, SettlementChainDump,
    TeeConfigDump, VrfConfigDump,
};

use super::Config;

/// Builds the serializable [`NodeConfig`] dump from the node's internal [`Config`].
pub fn node_config_dump(config: &Config, chain_spec: &ChainSpec) -> NodeConfig {
    NodeConfig {
        chain_id: chain_spec.id().id(),
        db: DbConfigDump {
            dir: config.db.dir.as_ref().map(|p| p.display().to_string()),
            migrate: config.db.migrate,
        },
        forking: config
            .forking
            .as_ref()
            .map(|f| ForkingConfigDump { url: f.url.to_string(), block: f.block }),
        rpc: RpcConfigDump {
            addr: config.rpc.addr,
            port: config.rpc.port,
            explorer: explorer_flag(config),
            apis: config.rpc.apis.clone(),
            cors_origins: config
                .rpc
                .cors_origins
                .iter()
                .map(|h| String::from_utf8_lossy(h.as_bytes()).into_owned())
                .collect(),
            max_connections: config.rpc.max_connections,
            max_concurrent_estimate_fee_requests: config.rpc.max_concurrent_estimate_fee_requests,
            max_request_body_size: config.rpc.max_request_body_size,
            max_response_body_size: config.rpc.max_response_body_size,
            timeout_millis: config.rpc.timeout.map(|d| d.as_millis() as u64),
            max_proof_keys: config.rpc.max_proof_keys,
            max_event_page_size: config.rpc.max_event_page_size,
            max_call_gas: config.rpc.max_call_gas,
        },
        gateway: config.gateway.as_ref().map(|g| GatewayConfigDump {
            addr: g.addr,
            port: g.port,
            timeout_millis: g.timeout.map(|d| d.as_millis() as u64),
        }),
        metrics: config.metrics.as_ref().map(|m| MetricsConfigDump { addr: m.addr, port: m.port }),
        execution: ExecutionConfigDump {
            invocation_max_steps: config.execution.invocation_max_steps,
            validation_max_steps: config.execution.validation_max_steps,
            max_recursion_depth: config.execution.max_recursion_depth as u64,
            compile_native: compile_native_flag(config),
        },
        messaging: config.messaging.as_ref().map(|m| MessagingConfigDump {
            settlement: match &m.settlement {
                SettlementChainConfig::Ethereum { rpc_url, contract_address } => {
                    SettlementChainDump::Ethereum {
                        rpc_url: rpc_url.to_string(),
                        contract_address: contract_address.to_string(),
                    }
                }
                SettlementChainConfig::Starknet { rpc_url, contract_address } => {
                    SettlementChainDump::Starknet {
                        rpc_url: rpc_url.to_string(),
                        contract_address: contract_address.to_string(),
                    }
                }
            },
            interval: m.interval,
            from_block: m.from_block,
            confirmation_depth: m.confirmation_depth,
        }),
        sequencing: SequencingConfigDump {
            block_time: config.sequencing.block_time,
            no_mining: config.sequencing.no_mining,
            block_cairo_steps_limit: config.sequencing.block_cairo_steps_limit,
            no_state_trie: config.sequencing.no_state_trie,
        },
        dev: DevConfigDump {
            fee: config.dev.fee,
            account_validation: config.dev.account_validation,
            fixed_gas_prices: config.dev.fixed_gas_prices.as_ref().map(|g| FixedGasPricesDump {
                l2_gas_prices: g.l2_gas_prices.clone(),
                l1_gas_prices: g.l1_gas_prices.clone(),
                l1_data_gas_prices: g.l1_data_gas_prices.clone(),
            }),
        },
        paymaster: config.paymaster.as_ref().map(|p| PaymasterConfigDump {
            url: p.url.to_string(),
            api_key: p.api_key.clone(),
            cartridge_api: p.cartridge_api.as_ref().map(|c| CartridgeApiConfigDump {
                cartridge_api_url: c.cartridge_api_url.to_string(),
                controller_deployer_address: c.controller_deployer_address,
                controller_deployer_private_key: c.controller_deployer_private_key,
                vrf: c
                    .vrf
                    .as_ref()
                    .map(|v| VrfConfigDump { url: v.url.to_string(), vrf_account: v.vrf_account }),
            }),
        }),
        tee: config.tee.as_ref().map(|t| TeeConfigDump {
            provider_type: t.provider_type.to_string(),
            fork_block_number: t.fork_block_number,
        }),
        grpc: grpc_dump(config),
    }
}

#[cfg(feature = "explorer")]
fn explorer_flag(config: &Config) -> Option<bool> {
    Some(config.rpc.explorer)
}

#[cfg(not(feature = "explorer"))]
fn explorer_flag(_config: &Config) -> Option<bool> {
    None
}

#[cfg(feature = "native")]
fn compile_native_flag(config: &Config) -> Option<bool> {
    Some(config.execution.compile_native)
}

#[cfg(not(feature = "native"))]
fn compile_native_flag(_config: &Config) -> Option<bool> {
    None
}

#[cfg(feature = "grpc")]
fn grpc_dump(config: &Config) -> Option<GrpcConfigDump> {
    config.grpc.as_ref().map(|g| GrpcConfigDump {
        addr: g.addr,
        port: g.port,
        timeout_millis: g.timeout.map(|d| d.as_millis() as u64),
    })
}

#[cfg(not(feature = "grpc"))]
fn grpc_dump(_config: &Config) -> Option<GrpcConfigDump> {
    None
}
