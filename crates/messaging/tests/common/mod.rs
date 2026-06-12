#![allow(dead_code)]

use std::fs::File;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use cairo_lang_starknet_classes::casm_contract_class::CasmContractClass;
use cairo_lang_starknet_classes::contract_class::ContractClass;
use katana_pool::api::TransactionPool;
use katana_pool::ordering::FiFo;
use katana_pool::pool::Pool;
use katana_pool::validation::NoopValidator;
use katana_primitives::class::CompiledClass;
use katana_primitives::transaction::{ExecutableTxWithHash, TxHash};
use katana_primitives::Felt;
use katana_provider::api::messaging::{
    MessagingCheckpoint, MessagingCheckpointProvider, MessagingL1ToL2IndexProvider,
};
use katana_provider::{DbProviderFactory, MutableProvider, ProviderFactory};
use starknet::core::types::contract::SierraClass;
use starknet::core::types::FlattenedSierraClass;

/// A `TxPool` flavor wired with a no-op validator. The messaging service only
/// drives the pool through `add_transaction`, so we don't need the full
/// stateful validator machinery to exercise the gather/insert/checkpoint loop.
pub type TestPool =
    Pool<ExecutableTxWithHash, NoopValidator<ExecutableTxWithHash>, FiFo<ExecutableTxWithHash>>;

pub fn build_test_pool() -> TestPool {
    Pool::new(NoopValidator::new(), FiFo::new())
}

pub fn build_test_provider() -> DbProviderFactory {
    DbProviderFactory::new_in_memory()
}

/// Poll the pool until it contains `expected` transactions, or `timeout`
/// elapses. Returns the size observed at the end.
pub async fn wait_for_pool_size(
    pool: &TestPool,
    expected: usize,
    timeout: Duration,
) -> Result<usize> {
    let deadline = Instant::now() + timeout;
    loop {
        let size = pool.size();
        if size >= expected {
            return Ok(size);
        }
        if Instant::now() >= deadline {
            return Err(anyhow!(
                "timed out waiting for pool to reach size {expected} (got {size})"
            ));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Read the L1->L2 index entry for `l1_tx_hash` from the provider.
pub fn l2_txs_for_l1(provider: &DbProviderFactory, l1_tx_hash: &[u8; 32]) -> Vec<TxHash> {
    let tx = provider.provider_mut();
    let res = tx.l2_txs_for_l1(l1_tx_hash).expect("read l1->l2 index");
    tx.commit().expect("commit read tx");
    res
}

/// Read the messaging checkpoint for the default messaging id ("messaging") from the provider.
pub fn messaging_checkpoint(provider: &DbProviderFactory) -> Option<MessagingCheckpoint> {
    let tx = provider.provider_mut();
    let res = tx.messaging_checkpoint().expect("read messaging checkpoint");
    tx.commit().expect("commit read tx");
    res
}

pub fn prepare_contract_declaration_params(
    artifact_path: &PathBuf,
) -> Result<(FlattenedSierraClass, Felt)> {
    let flattened_class = get_flattened_class(artifact_path)
        .map_err(|e| anyhow!("error flattening the contract class: {e}"))?;
    let compiled_class_hash = get_compiled_class_hash(artifact_path)
        .map_err(|e| anyhow!("error computing compiled class hash: {e}"))?;
    Ok((flattened_class, compiled_class_hash))
}

fn get_flattened_class(artifact_path: &PathBuf) -> Result<FlattenedSierraClass> {
    let file = File::open(artifact_path)?;
    let contract_artifact: SierraClass = serde_json::from_reader(&file)?;
    Ok(contract_artifact.flatten()?)
}

fn get_compiled_class_hash(artifact_path: &PathBuf) -> Result<Felt> {
    let file = File::open(artifact_path)?;
    let casm_contract_class: ContractClass = serde_json::from_reader(file)?;
    let casm_contract =
        CasmContractClass::from_contract_class(casm_contract_class, true, usize::MAX)
            .map_err(|e| anyhow!("CasmContractClass from ContractClass error: {e}"))?;
    let res = serde_json::to_string_pretty(&casm_contract)?;
    let compiled: CompiledClass = serde_json::from_str(&res)?;
    Ok(compiled.class_hash()?)
}
