use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use cainome::rs::abigen_legacy;
use katana_primitives::genesis::constant::DEFAULT_ETH_FEE_TOKEN_ADDRESS;
use katana_utils::TestNode;
use starknet::core::types::Felt;
use starknet::macros::felt;
use tokio::sync::Mutex;

mod common;

abigen_legacy!(Erc20Contract, "contracts/build/legacy/erc20.json", derives(Clone));

#[tokio::test(flavor = "multi_thread")]
async fn test_estimate_fee_rate_limiting() -> Result<()> {
    let mut config = katana_utils::node::test_config();

    let max_concurrent_estimate_fee_requests = 2;
    config.rpc.max_concurrent_estimate_fee_requests = Some(max_concurrent_estimate_fee_requests);

    let sequencer = TestNode::new_with_config(config).await;

    let _provider = sequencer.starknet_provider();
    let account = Arc::new(sequencer.account());

    let _contract = Erc20Contract::new(DEFAULT_ETH_FEE_TOKEN_ADDRESS.into(), &account);

    let recipient = felt!("0x1");
    let amount = Uint256 { low: felt!("0x1"), high: Felt::ZERO };

    const REQUEST_COUNT: usize = 10;
    let start_time = Arc::new(Mutex::new(None));
    let completion_times = Arc::new(Mutex::new(Vec::with_capacity(REQUEST_COUNT)));

    let mut handles = Vec::with_capacity(REQUEST_COUNT);

    for i in 0..REQUEST_COUNT {
        let account_clone = account.clone();
        let recipient_clone = recipient;
        let amount_clone = amount.clone();
        let start_time = start_time.clone();
        let completion_times = completion_times.clone();

        let handle = tokio::spawn(async move {
            if i == 0 {
                let mut start = start_time.lock().await;
                *start = Some(Instant::now());
            }

            // Create contract instance inside the task with the cloned account
            let contract_instance =
                Erc20Contract::new(DEFAULT_ETH_FEE_TOKEN_ADDRESS.into(), &account_clone);
            let result =
                contract_instance.transfer(&recipient_clone, &amount_clone).estimate_fee().await;

            let now = Instant::now();
            let mut times = completion_times.lock().await;
            times.push((i, now));

            result
        });

        handles.push(handle);

        if i < REQUEST_COUNT - 1 {
            tokio::time::sleep(tokio::time::Duration::from_millis(5)).await;
        }
    }

    for handle in handles {
        let _ = handle.await?;
    }

    let _start_time = start_time.lock().await.unwrap();
    let completion_times = completion_times.lock().await;

    let mut sorted_times = completion_times.clone();
    sorted_times.sort_by_key(|&(_, time)| time);

    println!("Completion times (task_id, time):");
    for (i, time) in &sorted_times {
        println!("Task {}: {:?}", i, time);
    }

    let mut time_diffs = Vec::new();
    for i in 1..sorted_times.len() {
        let prev_time = sorted_times[i - 1].1;
        let curr_time = sorted_times[i].1;
        let diff = curr_time.duration_since(prev_time).as_millis();
        time_diffs.push(diff);
        println!("Time diff between {} and {}: {} ms", i - 1, i, diff);
    }

    let mut batches = Vec::new();
    let mut current_batch = vec![sorted_times[0].0];

    for i in 1..sorted_times.len() {
        let prev_time = sorted_times[i - 1].1;
        let curr_time = sorted_times[i].1;
        let diff = curr_time.duration_since(prev_time).as_millis();

        if diff < 20 {
            current_batch.push(sorted_times[i].0);
        } else {
            batches.push(current_batch);
            current_batch = vec![sorted_times[i].0];
        }
    }

    if !current_batch.is_empty() {
        batches.push(current_batch);
    }

    println!("Task batches by completion time: {:?}", batches);

    let first_batch_size = batches[0].len();
    println!("First batch size: {}", first_batch_size);

    assert!(
        first_batch_size <= max_concurrent_estimate_fee_requests as usize,
        "First batch of requests ({}) exceeded the configured limit ({})",
        first_batch_size,
        max_concurrent_estimate_fee_requests
    );

    assert!(
        batches.len() > 1,
        "No evidence of rate limiting found - all requests completed in a single batch"
    );

    Ok(())
}
