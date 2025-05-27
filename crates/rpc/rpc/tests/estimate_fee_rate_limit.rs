use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use jsonrpsee::http_client::HttpClientBuilder;
use katana_primitives::genesis::constant::DEFAULT_ETH_FEE_TOKEN_ADDRESS;
use katana_rpc::starknet::StarknetApiConfig;
use katana_utils::TestNode;
use starknet::accounts::ConnectedAccount;
use starknet::core::types::{BlockId, BlockTag, Felt};
use tokio::sync::Mutex;

mod common;

abigen_legacy!(Erc20Contract, "crates/rpc/rpc/tests/test_data/erc20.json", derives(Clone));

#[tokio::test(flavor = "multi_thread")]
async fn test_estimate_fee_rate_limiting() -> Result<()> {
    let mut config = katana_utils::node::test_config();

    let max_concurrent_estimate_fee_requests = 2;

    let sequencer = TestNode::new_with_custom_rpc(config, |rpc_server| {
        let cfg = StarknetApiConfig {
            max_event_page_size: None,
            max_proof_keys: None,
            max_concurrent_estimate_fee_requests: Some(max_concurrent_estimate_fee_requests),
            #[cfg(feature = "cartridge")]
            paymaster: None,
        };

        rpc_server.starknet_api_config(cfg)
    })
    .await;

    let provider = sequencer.starknet_provider();
    let account = Arc::new(sequencer.account());

    let contract = Erc20Contract::new(DEFAULT_ETH_FEE_TOKEN_ADDRESS.into(), &account);

    let recipient = Felt::ONE;
    let amount = Uint256 { low: Felt::ONE, high: Felt::ZERO };

    const REQUEST_COUNT: usize = 10;
    let start_time = Arc::new(Mutex::new(None));
    let completion_times = Arc::new(Mutex::new(Vec::with_capacity(REQUEST_COUNT)));

    let mut handles = Vec::with_capacity(REQUEST_COUNT);

    for i in 0..REQUEST_COUNT {
        let contract = contract.clone();
        let start_time = start_time.clone();
        let completion_times = completion_times.clone();

        let handle = tokio::spawn(async move {
            if i == 0 {
                let mut start = start_time.lock().await;
                *start = Some(Instant::now());
            }

            let result = contract.transfer(&recipient, &amount).estimate_fee().await;

            let now = Instant::now();
            let mut times = completion_times.lock().await;
            times.push((i, now));

            result
        });

        handles.push(handle);
    }

    for handle in handles {
        let _ = handle.await?;
    }

    let start_time = start_time.lock().await.unwrap();
    let completion_times = completion_times.lock().await;

    let mut sorted_times = completion_times.clone();
    sorted_times.sort_by_key(|&(_, time)| time);

    let mut time_diffs = Vec::new();
    for i in 1..sorted_times.len() {
        let prev_time = sorted_times[i - 1].1;
        let curr_time = sorted_times[i].1;
        let diff = curr_time.duration_since(prev_time).as_millis();
        time_diffs.push(diff);
    }

    let first_batch_count = time_diffs.iter().filter(|&&diff| diff < 5).count() + 1; // +1 for the first request

    assert!(
        first_batch_count <= max_concurrent_estimate_fee_requests + 1,
        "First batch of requests ({}) exceeded the configured limit ({})",
        first_batch_count,
        max_concurrent_estimate_fee_requests
    );

    let delayed_requests = time_diffs.iter().filter(|&&diff| diff >= 5).count();
    assert!(
        delayed_requests > 0,
        "No evidence of rate limiting found - all requests completed too quickly"
    );

    Ok(())
}
