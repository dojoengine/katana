//! Katana node spawning for the saya-tee e2e test.
//!
//! Spawns two in-process Katanas via [`katana_utils::TestNode`]:
//!
//! - **L2** — vanilla dev chain acting as the settlement layer. Hosts Piltover and the mock TEE
//!   registry that `saya-ops` deploys into it.
//! - **L3** — rollup chain whose `SettlementLayer::Starknet` points at L2's Piltover address.
//!   Configured with `Config.tee = TeeConfig { provider_type: Mock, .. }` so its
//!   `tee_generateQuote` RPC serves a stub attestation that `saya-tee --mock-prove` consumes.
//!
//! Both Nodes run in one process with independent [`ClassCache`] instances. Previously the L2
//! had to be spawned as a subprocess because Katana's executor shared a process-global
//! `OnceLock<ClassCache>` — when saya-ops deployed UDC on L2, the same cache entry would be
//! observed by L3's `GenesisTransactionsBuilder` as "already declared", nuking the rollup's
//! genesis. That global is gone (see `crates/executor/src/blockifier/cache.rs`), so each Node
//! owns its own cache and the two can safely coexist in one process.
//!
//! [`ClassCache`]: katana_executor::blockifier::cache::ClassCache

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use cainome::rs::abigen_legacy;
use katana_chain_spec::rollup::DEFAULT_APPCHAIN_FEE_TOKEN_ADDRESS;
use katana_chain_spec::{rollup, ChainSpec, FeeContracts, SettlementLayer, SettlementProofKind};
use katana_genesis::allocation::DevAllocationsGenerator;
use katana_genesis::constant::DEFAULT_PREFUNDED_ACCOUNT_BALANCE;
use katana_genesis::Genesis;
use katana_primitives::chain::ChainId;
use katana_primitives::U256;
use katana_sequencer_node::config::tee::TeeConfig;
use katana_tee::TeeProviderType;
use katana_utils::TestNode;
use starknet::accounts::SingleOwnerAccount;
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::JsonRpcClient;
use starknet::signers::LocalWallet;
use starknet_types_core::felt::Felt;
use url::Url;

/// L2 settlement Katana, spawned in-process via [`TestNode`] on a dev chain spec.
pub struct L2InProcess {
    inner: TestNode,
}

impl L2InProcess {
    pub fn url(&self) -> Url {
        Url::parse(&format!("http://{}", self.inner.rpc_addr()))
            .expect("rpc_addr produces valid URL")
    }

    pub fn provider(&self) -> JsonRpcClient<HttpTransport> {
        self.inner.starknet_provider()
    }

    /// The first prefunded dev account from the L2's genesis — used by `saya-ops` as the
    /// deployer for Piltover and the mock TEE registry.
    pub fn prefunded_account_keys(&self) -> (Felt, Felt) {
        let (address, account) = self
            .inner
            .backend()
            .chain_spec
            .genesis()
            .accounts()
            .next()
            .expect("L2 dev genesis must have at least one prefunded account");
        let private_key =
            account.private_key().expect("dev genesis accounts are seeded with a private key");
        ((*address).into(), private_key)
    }
}

/// L3 rollup Katana, spawned in-process via [`TestNode`].
pub struct L3InProcess {
    inner: TestNode,
}

impl L3InProcess {
    pub fn url(&self) -> Url {
        Url::parse(&format!("http://{}", self.inner.rpc_addr()))
            .expect("rpc_addr produces valid URL")
    }

    pub fn provider(&self) -> JsonRpcClient<HttpTransport> {
        self.inner.starknet_provider()
    }

    pub fn account(&self) -> SingleOwnerAccount<JsonRpcClient<HttpTransport>, LocalWallet> {
        self.inner.account()
    }
}

/// Spawns the L2 settlement Katana as an in-process `TestNode` on the standard dev chain spec
/// (`ChainId::SEPOLIA`, `fee: false`) — equivalent to `katana --dev --dev.no-fee`.
pub async fn spawn_l2() -> L2InProcess {
    let config = katana_utils::node::test_config();
    L2InProcess { inner: TestNode::new_with_config(config).await }
}

/// Spawns the L3 rollup Katana with TEE config and settlement pointed at L2.
///
/// Constructs a [`rollup::ChainSpec`] with `SettlementLayer::Starknet`
/// referencing the L2 Piltover address, and a [`Config`] with the mock TEE
/// provider enabled so `tee_generateQuote` works without real SEV-SNP
/// hardware.
pub async fn spawn_l3(l2: &L2InProcess, piltover_address: Felt) -> L3InProcess {
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
    // IMPORTANT: rollup chain specs run their genesis through
    // `GenesisTransactionsBuilder`, which emits explicit declare txs for the
    // ERC20, UDC, and account classes. `Genesis::default()` pre-declares
    // those same three classes directly into chain state, which causes the
    // builder's declare txs to fail with "already declared", cascading into
    // nonce mismatches that skip the `transfer_balance` calls and leave
    // prefunded accounts with zero balance. So we start from an empty-classes
    // genesis here.
    let mut genesis = Genesis { classes: Default::default(), ..Genesis::default() };
    genesis.extend_allocations(accounts.into_iter().map(|(k, v)| (k, v.into())));

    let settlement = SettlementLayer::Starknet {
        block: 0,
        id: l2_chain_id,
        rpc_url: l2_url,
        core_contract: piltover_address.into(),
        proof_kind: SettlementProofKind::Tee,
    };

    let l3_chain = rollup::ChainSpec {
        id: ChainId::parse("KATANA").expect("KATANA is a valid chain id"),
        genesis,
        fee_contracts,
        settlement,
    };

    let mut config = katana_utils::node::test_config();
    config.chain = Arc::new(ChainSpec::Rollup(l3_chain));
    config.tee = Some(TeeConfig { provider_type: TeeProviderType::Mock, fork_block_number: None });
    // Note: rollup chain specs (provable mode) never produce empty blocks
    // even with `block_time` set, per the upstream Saya README. The L3 only
    // advances when transactions are submitted; we drive that explicitly
    // via [`drive_l3_block`] below.

    L3InProcess { inner: TestNode::new_with_config(config).await }
}

/// Drives the L3 forward by one block via a single no-op self-transfer from the
/// prefunded test account. Provable-mode rollups never produce empty blocks, so
/// transactions are the only way to advance height.
pub async fn drive_l3_block(l3: &L3InProcess) -> Result<()> {
    use starknet::accounts::Account;

    abigen_legacy!(Erc20Contract, "crates/contracts/build/legacy/erc20.json", derives(Clone));

    let account = l3.account();
    let strk_contract = Erc20Contract::new(DEFAULT_APPCHAIN_FEE_TOKEN_ADDRESS.into(), &account);

    let result = strk_contract
        .transfer(&account.address().into(), &Uint256 { low: Felt::ONE, high: Felt::ZERO })
        .send()
        .await
        .context("driver tx failed to send")?;

    wait_for_tx(&l3.provider(), result.transaction_hash).await?;

    Ok(())
}

async fn wait_for_tx(provider: &JsonRpcClient<HttpTransport>, tx_hash: Felt) -> Result<()> {
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
