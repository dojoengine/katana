//! In-process Katana node spawning for the saya-tee e2e test.
//!
//! Spawns two Katanas:
//! - **L2** — vanilla dev chain. Acts as the settlement chain that hosts
//!   Piltover and the mock TEE registry. Uses the default `katana_utils`
//!   `test_config` (`ChainSpec::Dev`).
//! - **L3** — rollup chain whose `SettlementLayer::Starknet` points at L2's
//!   Piltover address. Has `Config.tee = TeeConfig { provider_type: Mock, .. }`
//!   so its `tee_generateQuote` RPC serves a stub attestation that
//!   `saya-tee --mock-prove` consumes.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use katana_chain_spec::rollup::DEFAULT_APPCHAIN_FEE_TOKEN_ADDRESS;
use katana_chain_spec::{rollup, ChainSpec, FeeContracts, SettlementLayer};
use katana_genesis::allocation::DevAllocationsGenerator;
use katana_genesis::constant::DEFAULT_PREFUNDED_ACCOUNT_BALANCE;
use katana_genesis::Genesis;
use katana_primitives::chain::ChainId;
use katana_primitives::U256;
use katana_sequencer_node::config::tee::TeeConfig;
use katana_tee::TeeProviderType;
use katana_utils::TestNode;
use starknet::accounts::{Account, SingleOwnerAccount};
use starknet::core::types::{Call, Felt};
use starknet::macros::selector;
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::JsonRpcClient;
use starknet::signers::LocalWallet;
use url::Url;

/// Holds an in-process Katana node and provides convenience accessors used by
/// the saya-tee e2e test.
pub struct Node {
    inner: TestNode,
}

impl Node {
    pub fn url(&self) -> Url {
        Url::parse(&format!("http://{}", self.inner.rpc_addr()))
            .expect("rpc_addr produces valid URL")
    }

    pub fn account(&self) -> SingleOwnerAccount<JsonRpcClient<HttpTransport>, LocalWallet> {
        self.inner.account()
    }

    pub fn provider(&self) -> JsonRpcClient<HttpTransport> {
        self.inner.starknet_provider()
    }

    /// Returns the address and private key of the first prefunded genesis
    /// account on this node. Used by the saya-ops bootstrap to declare and
    /// deploy contracts on L2 without going through the `LocalWallet` opaque
    /// signing key.
    pub fn prefunded_account_keys(&self) -> (Felt, Felt) {
        let chain_spec = &self.inner.backend().chain_spec;
        let (address, account) = chain_spec
            .genesis()
            .accounts()
            .next()
            .expect("dev genesis has at least one prefunded account");
        let private_key = account
            .private_key()
            .expect("dev genesis accounts have private keys");
        ((*address).into(), private_key)
    }
}

/// Spawns the L2 settlement Katana — a plain dev chain.
pub async fn spawn_l2() -> Node {
    Node { inner: TestNode::new().await }
}

/// Spawns the L3 rollup Katana with TEE config and settlement pointed at L2.
///
/// Constructs a [`rollup::ChainSpec`] with `SettlementLayer::Starknet`
/// referencing the L2 Piltover address, and a [`Config`] with the mock TEE
/// provider enabled so `tee_generateQuote` works without real SEV-SNP
/// hardware.
pub async fn spawn_l3(l2: &Node, piltover_address: Felt) -> Node {
    let l2_url = l2.url();
    let l2_chain_id = ChainId::SEPOLIA; // Matches katana_utils::test_config()'s default.

    // Use the default appchain fee token so it matches what saya assumes
    // for the StarknetOsConfig hash computation. (saya-tee `--mock-prove`
    // doesn't actually rely on this, but keeping it canonical avoids
    // accidental mismatches if the flag is removed in the future.)
    let fee_contracts = FeeContracts {
        eth: DEFAULT_APPCHAIN_FEE_TOKEN_ADDRESS,
        strk: DEFAULT_APPCHAIN_FEE_TOKEN_ADDRESS,
    };

    // Generate prefunded accounts for the L3 so the test can drive
    // transactions through it.
    let accounts = DevAllocationsGenerator::new(10)
        .with_balance(U256::from(DEFAULT_PREFUNDED_ACCOUNT_BALANCE))
        .generate();
    let mut genesis = Genesis::default();
    genesis.extend_allocations(accounts.into_iter().map(|(k, v)| (k, v.into())));

    let settlement = SettlementLayer::Starknet {
        block: 0,
        id: l2_chain_id,
        rpc_url: l2_url,
        core_contract: piltover_address.into(),
    };

    let l3_chain = rollup::ChainSpec {
        id: ChainId::parse("KATANA").expect("KATANA is a valid chain id"),
        genesis,
        fee_contracts,
        settlement,
    };

    let mut config = katana_utils::node::test_config();
    config.chain = Arc::new(ChainSpec::Rollup(l3_chain));
    config.tee = Some(TeeConfig {
        provider_type: TeeProviderType::Mock,
        fork_block_number: None,
    });

    Node { inner: TestNode::new_with_config(config).await }
}

/// Drives the L3 to advance its block height by submitting `n` no-op self
/// transfers via the prefunded test account.
///
/// Each call awaits transaction acceptance to ensure block production has
/// progressed before returning.
pub async fn drive_l3_blocks(l3: &Node, n: u64) -> Result<()> {
    let account = l3.account();
    let address = account.address();

    // STRK fee token address that the rollup chain spec uses.
    let strk: Felt = DEFAULT_APPCHAIN_FEE_TOKEN_ADDRESS.into();

    for i in 0..n {
        let call = Call {
            to: strk,
            selector: selector!("transfer"),
            calldata: vec![address, Felt::from(1u64), Felt::ZERO],
        };

        let result = account
            .execute_v3(vec![call])
            .send()
            .await
            .with_context(|| format!("driver tx {i} failed to send"))?;

        // Wait for the tx to be accepted so we know the block has been produced.
        wait_for_tx(&l3.provider(), result.transaction_hash).await?;
    }

    Ok(())
}

async fn wait_for_tx(
    provider: &JsonRpcClient<HttpTransport>,
    tx_hash: Felt,
) -> Result<()> {
    use starknet::providers::Provider;
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        match provider.get_transaction_receipt(tx_hash).await {
            Ok(_) => return Ok(()),
            Err(_) if std::time::Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
            Err(e) => return Err(anyhow::anyhow!("tx {tx_hash:#x} not accepted: {e}")),
        }
    }
}

