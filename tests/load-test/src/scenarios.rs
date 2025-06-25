use anyhow::Result;
use goose::prelude::*;
use rand::Rng;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

use crate::client::{Account, StarknetClient};
use crate::transactions::TransactionType;
use crate::{BurstArgs, GlobalArgs, RampUpArgs};

/// Global state shared across Goose users
#[derive(Clone)]
pub struct UserState {
    pub client: StarknetClient,
    pub account: Account,
    pub nonce: Arc<Mutex<starknet::core::types::Felt>>,
}

impl UserState {
    pub async fn new(global_args: &GlobalArgs) -> Result<Self> {
        let client = StarknetClient::new(&global_args.rpc_url)?;
        
        let account = if let (Some(private_key), Some(address)) = 
            (&global_args.private_key, &global_args.account_address) {
            Account::new(private_key, address)?
        } else {
            Account::default_test_account()
        };

        // Get current nonce
        let current_nonce = client.get_nonce(account.address).await?;
        let nonce = Arc::new(Mutex::new(current_nonce));

        Ok(Self {
            client,
            account,
            nonce,
        })
    }

    pub async fn get_and_increment_nonce(&self) -> starknet::core::types::Felt {
        let mut nonce = self.nonce.lock().await;
        let current = *nonce;
        *nonce = current + starknet::core::types::Felt::ONE;
        current
    }
}

/// Main transaction sending scenario
pub async fn send_transaction(user: &mut GooseUser) -> TransactionResult {
    let state = user.get_session_data::<UserState>()
        .ok_or_else(|| GooseError::from("No user state found"))?;

    let tx_type = TransactionType::random();
    let nonce = state.get_and_increment_nonce().await;

    debug!("Sending transaction: {:?} with nonce: {:#x}", tx_type, nonce);

    let transaction = tx_type.build_transaction(&state.account, nonce)
        .map_err(|e| GooseError::from(format!("Failed to build transaction: {}", e)))?;

    let start_time = std::time::Instant::now();
    
    match state.client.add_invoke_transaction(transaction).await {
        Ok(tx_hash) => {
            let elapsed = start_time.elapsed();
            debug!("Transaction submitted: {:#x} in {:?}", tx_hash, elapsed);
            
            // Store tx_hash in user session for status checking
            user.set_session_data(tx_hash);
            
            user.set_success(&TransactionRequest {
                elapsed: elapsed.as_millis() as u64,
                final_url: state.client.rpc_url.clone(),
                name: format!("send_{:?}", tx_type).to_lowercase(),
                redirected: false,
                response_time: elapsed.as_millis() as u64,
                status_code: 200,
                success: true,
                update: false,
                user: user.weighted_users_index,
                ..Default::default()
            })
        }
        Err(e) => {
            let elapsed = start_time.elapsed();
            warn!("Transaction failed: {}", e);
            
            user.set_failure(&format!("Transaction failed: {}", e), &mut TransactionRequest {
                elapsed: elapsed.as_millis() as u64,
                final_url: state.client.rpc_url.clone(),
                name: format!("send_{:?}", tx_type).to_lowercase(),
                redirected: false,
                response_time: elapsed.as_millis() as u64,
                status_code: 500,
                success: false,
                update: false,
                user: user.weighted_users_index,
                ..Default::default()
            })
        }
    }
}

/// Check transaction status scenario
pub async fn check_transaction_status(user: &mut GooseUser) -> TransactionResult {
    let state = user.get_session_data::<UserState>()
        .ok_or_else(|| GooseError::from("No user state found"))?;

    // Get the last transaction hash from session
    let tx_hash = match user.get_session_data::<starknet::core::types::Felt>() {
        Some(hash) => hash,
        None => {
            // If no transaction hash, skip this request
            return user.set_success(&TransactionRequest {
                name: "check_tx_status_skipped".to_string(),
                ..Default::default()
            });
        }
    };

    let start_time = std::time::Instant::now();
    
    match state.client.get_transaction_status(*tx_hash).await {
        Ok(status) => {
            let elapsed = start_time.elapsed();
            debug!("Transaction {:#x} status: {}", tx_hash, status);
            
            user.set_success(&TransactionRequest {
                elapsed: elapsed.as_millis() as u64,
                final_url: state.client.rpc_url.clone(),
                name: "check_tx_status".to_string(),
                redirected: false,
                response_time: elapsed.as_millis() as u64,
                status_code: 200,
                success: true,
                update: false,
                user: user.weighted_users_index,
                ..Default::default()
            })
        }
        Err(e) => {
            let elapsed = start_time.elapsed();
            warn!("Failed to check transaction status: {}", e);
            
            user.set_failure(&format!("Status check failed: {}", e), &mut TransactionRequest {
                elapsed: elapsed.as_millis() as u64,
                final_url: state.client.rpc_url.clone(),
                name: "check_tx_status".to_string(),
                redirected: false,
                response_time: elapsed.as_millis() as u64,
                status_code: 500,
                success: false,
                update: false,
                user: user.weighted_users_index,
                ..Default::default()
            })
        }
    }
}

/// Initialize user state
pub async fn setup_user(user: &mut GooseUser) -> TransactionResult {
    let global_args = user.get_base_url(); // We'll store global args in base_url for now
    
    // For simplicity, create a default state
    // In a real implementation, you'd pass global_args properly
    let state = UserState {
        client: StarknetClient::new("http://localhost:5050").unwrap(),
        account: Account::default_test_account(),
        nonce: Arc::new(Mutex::new(starknet::core::types::Felt::ZERO)),
    };

    // Initialize nonce from network
    match state.client.get_nonce(state.account.address).await {
        Ok(nonce) => {
            *state.nonce.lock().await = nonce;
            info!("Initialized user with nonce: {:#x}", nonce);
        }
        Err(e) => {
            warn!("Failed to get initial nonce: {}", e);
            // Continue with nonce 0
        }
    }

    user.set_session_data(state);
    user.set_success(&TransactionRequest::default())
}

/// Constant load scenario
pub async fn constant_load(
    mut attack: GooseAttack,
    global_args: GlobalArgs,
    users: usize,
    duration: Duration,
    tps: u64,
) -> Result<GooseAttack> {
    // Calculate request frequency per user
    let requests_per_user_per_sec = tps as f64 / users as f64;
    let wait_time = if requests_per_user_per_sec > 0.0 {
        Duration::from_millis((1000.0 / requests_per_user_per_sec) as u64)
    } else {
        Duration::from_secs(1)
    };

    Ok(attack
        .register_scenario(
            scenario!("KatanaConstantLoad")
                .register_transaction(transaction!(setup_user).set_on_start())
                .register_transaction(
                    transaction!(send_transaction)
                        .set_name("send_transaction")
                        .set_weight(10)?
                        .set_wait_time(wait_time, wait_time)?
                )
                .register_transaction(
                    transaction!(check_transaction_status)
                        .set_name("check_tx_status")
                        .set_weight(2)?
                )
        )
        .set_default(GooseDefault::Host, &global_args.rpc_url)?
        .set_default(GooseDefault::Users, users)?
        .set_default(GooseDefault::HatchRate, users as f64)?
        .set_default(GooseDefault::RunTime, duration.as_secs())?
    )
}

/// Ramp-up load scenario
pub async fn ramp_up_load(
    mut attack: GooseAttack,
    global_args: GlobalArgs,
    args: RampUpArgs,
) -> Result<GooseAttack> {
    // Calculate max users needed for max TPS
    let max_users = (args.max_tps as f64 * 1.5) as usize; // 1.5x buffer
    
    Ok(attack
        .register_scenario(
            scenario!("KatanaRampUpLoad")
                .register_transaction(transaction!(setup_user).set_on_start())
                .register_transaction(
                    transaction!(send_transaction)
                        .set_name("send_transaction")
                        .set_weight(10)?
                )
                .register_transaction(
                    transaction!(check_transaction_status)
                        .set_name("check_tx_status")
                        .set_weight(2)?
                )
        )
        .set_default(GooseDefault::Host, &global_args.rpc_url)?
        .set_default(GooseDefault::Users, max_users)?
        .set_default(GooseDefault::HatchRate, 2.0)? // Gradual ramp
        .set_default(GooseDefault::RunTime, args.ramp_duration + args.hold_duration)?
    )
}

/// Burst load scenario
pub async fn burst_load(
    mut attack: GooseAttack,
    global_args: GlobalArgs,
    args: BurstArgs,
) -> Result<GooseAttack> {
    let max_users = (args.burst_tps as f64 * 1.5) as usize;
    
    Ok(attack
        .register_scenario(
            scenario!("KatanaBurstLoad")
                .register_transaction(transaction!(setup_user).set_on_start())
                .register_transaction(
                    transaction!(send_transaction)
                        .set_name("send_transaction")
                        .set_weight(10)?
                )
                .register_transaction(
                    transaction!(check_transaction_status)
                        .set_name("check_tx_status")
                        .set_weight(2)?
                )
        )
        .set_default(GooseDefault::Host, &global_args.rpc_url)?
        .set_default(GooseDefault::Users, max_users)?
        .set_default(GooseDefault::HatchRate, 10.0)? // Fast burst
        .set_default(GooseDefault::RunTime, args.total_duration)?
    )
}
