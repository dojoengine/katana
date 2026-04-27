use saya_e2e::{
    compose_up, env, get_facts_registry, get_program_info, provider, wait_for_settlement,
    ComposeGuard,
};
use starknet::core::crypto::compute_hash_on_elements;
use starknet::core::types::Felt;
use starknet::core::utils::cairo_short_string_to_felt;
use starknet::macros::{felt, short_string};
use std::time::Duration;

const BOOTLOADER_PROGRAM_HASH: Felt =
    felt!("0x5ab580b04e3532b6b18f81cfa654a05e29dd8e2352d88df1e765a84072db07");
const SNOS_PROGRAM_HASH: Felt =
    felt!("0x10e5341a417427d140af8f5def7d2cc687d84591ff8ec241623c590b5ca8c80");
const LAYOUT_BRIDGE_PROGRAM_HASH: Felt =
    felt!("0x43c5c4cc37c4614d2cf3a833379052c3a38cd18d688b617e2c720e8f941cb8");

#[tokio::test]
async fn test_program_info_and_fact_registry() {
    compose_up();
    let _guard = ComposeGuard;

    let provider = provider(&env::settlement_rpc_url());
    let piltover_address = env::piltover_address();

    let program_info = get_program_info(&provider, piltover_address)
        .await
        .expect("failed to query piltover get_program_info");

    let facts_registry = get_facts_registry(&provider, piltover_address)
        .await
        .expect("failed to query piltover get_facts_registry");

    let chain_id = cairo_short_string_to_felt(&env::chain_id()).expect("invalid chain id for test");
    let snos_config_hash = compute_hash_on_elements(&[
        short_string!("StarknetOsConfig3"),
        chain_id,
        env::fee_token_address(),
    ]);

    assert_eq!(
        program_info.bootloader_program_hash,
        BOOTLOADER_PROGRAM_HASH
    );
    assert_eq!(program_info.snos_program_hash, SNOS_PROGRAM_HASH);
    assert_eq!(
        program_info.layout_bridge_program_hash,
        LAYOUT_BRIDGE_PROGRAM_HASH
    );
    assert_eq!(program_info.snos_config_hash, snos_config_hash);
    assert_eq!(facts_registry, env::fact_registry_address());
}

#[tokio::test]
async fn test_settlement_advances_after_genesis() {
    compose_up();
    let _guard = ComposeGuard;

    let provider = provider(&env::settlement_rpc_url());
    let piltover_address = env::piltover_address();

    let state = wait_for_settlement(
        &provider,
        piltover_address,
        0,
        Duration::from_secs(120),
        Duration::from_secs(2),
    )
    .await
    .expect("settlement did not advance past genesis within timeout");

    let _settled: u64 = state
        .block_number
        .try_into()
        .expect("failed to convert settled block number");
}
