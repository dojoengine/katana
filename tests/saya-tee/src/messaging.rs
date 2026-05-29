//! Cross-chain message settlement regression coverage.
//!
//! The base flow ([`crate::main`]) only settles plain transfer blocks, so it never
//! builds a `messages_commitment` over a real cross-chain message — the path that
//! once silently broke. This module deploys an l1-handler contract on the appchain
//! and drives one message in each direction through a *settled* block:
//!
//! - **L1 → L2:** [`send_l1_to_l2`] calls `send_message_to_appchain` on the L2 piltover core. The
//!   appchain's messaging collector relays it into the `msg_handler_value` l1-handler, which lands
//!   in an L3 block that must settle.
//! - **L2 → L1:** [`send_l2_to_l1`] makes the appchain emit `send_message_to_l1`.
//!
//! **Regression target:** saya-tee must hash L1→L2 messages with the Poseidon
//! formula Katana commits to. If it uses the Ethereum `keccak256` formula,
//! piltover's `update_state` rejects the L1→L2-bearing block with
//! `'tee: invalid messages'` and settlement stalls — caught by the
//! [`crate::assertions::wait_for_settlement`] that follows, timing out.

use std::str::FromStr;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use katana_primitives::class::ContractClass;
use katana_primitives::utils::get_contract_address;
use katana_primitives::{ContractAddress, Felt};
use katana_rpc_types::RpcSierraContractClass;
use starknet::accounts::{Account, ExecutionEncoding, SingleOwnerAccount};
use starknet::contract::ContractFactory;
use starknet::core::types::{BlockId, BlockTag, Call, FlattenedSierraClass};
use starknet::macros::{felt, selector};
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::{JsonRpcClient, Provider};
use starknet::signers::{LocalWallet, SigningKey};

use crate::nodes::{L2InProcess, L3InProcess};

/// Prebuilt sierra class exposing the `msg_handler_value` `#[l1_handler]` and a
/// `send_message` entrypoint (which calls `send_message_to_l1`).
const MSG_HANDLER_SIERRA: &str =
    "crates/contracts/build/katana_messaging_contract_msg_starknet.contract_class.json";

/// Deterministic deploy salt for the l1-handler contract on the appchain.
const DEPLOY_SALT: Felt = felt!("0x1337");

/// The value `msg_handler_value` asserts it receives (`assert(value == 888)`).
const HANDLER_VALUE: Felt = felt!("888");

/// Declares + deploys the `contract_msg_starknet` l1-handler contract on the
/// appchain and returns its address. Mirrors how the contract would be deployed
/// in practice (runtime declare+deploy via UDC), since rollup genesis only
/// deploys account allocations.
pub async fn deploy_msg_handler(l3: &L3InProcess) -> Result<Felt> {
    let json = std::fs::read_to_string(MSG_HANDLER_SIERRA)
        .with_context(|| format!("reading {MSG_HANDLER_SIERRA}"))?;
    let class =
        ContractClass::from_str(&json).map_err(|e| anyhow!("parse msg-handler sierra: {e}"))?;
    let class_hash = class.class_hash().context("compute sierra class hash")?;
    let casm_hash = class.clone().compile().context("compile sierra->casm")?.class_hash()?;

    let sierra = class.to_sierra().ok_or_else(|| anyhow!("not a sierra class"))?;
    let flattened = FlattenedSierraClass::try_from(RpcSierraContractClass::from(sierra))
        .context("flatten sierra class")?;

    let account = l3.account();

    let declare = account
        .declare_v3(flattened.into(), casm_hash)
        .send()
        .await
        .context("declare msg-handler")?;
    wait_for_tx(&l3.provider(), declare.transaction_hash).await?;

    // No constructor args; non-unique so the address is salt+class deterministic.
    // `new` uses the legacy UDC, which Katana's dev genesis predeploys.
    #[allow(deprecated)]
    let factory = ContractFactory::new(class_hash, &account);
    let deploy = factory
        .deploy_v3(Vec::new(), DEPLOY_SALT, false)
        .send()
        .await
        .context("deploy msg-handler")?;
    wait_for_tx(&l3.provider(), deploy.transaction_hash).await?;

    Ok(get_contract_address(DEPLOY_SALT, class_hash, &[], ContractAddress::ZERO))
}

/// Current L3 block height (settlement target baseline).
pub async fn current_tip(l3: &L3InProcess) -> Result<u64> {
    l3.provider().block_number().await.context("fetch L3 tip")
}

/// L1 → L2: send a message from the L2 piltover core to the appchain's
/// `msg_handler_value` l1-handler. The appchain collector relays it.
pub async fn send_l1_to_l2(l2: &L2InProcess, piltover: Felt, handler: Felt) -> Result<()> {
    let account = l2_account(l2).await?;
    // send_message_to_appchain(to_address, selector, payload: Span<felt252>)
    // serializes as [to_address, selector, payload_len, ...payload].
    let call = Call {
        to: piltover,
        selector: selector!("send_message_to_appchain"),
        calldata: vec![handler, selector!("msg_handler_value"), Felt::ONE, HANDLER_VALUE],
    };
    let res =
        account.execute_v3(vec![call]).send().await.context("invoke send_message_to_appchain")?;
    wait_for_tx(&l2.provider(), res.transaction_hash).await
}

/// L2 → L1: have the appchain contract emit `send_message_to_l1`.
pub async fn send_l2_to_l1(l3: &L3InProcess, handler: Felt) -> Result<()> {
    let account = l3.account();
    // send_message(to_address, value) -> send_message_to_l1_syscall('MSG', [to, value])
    let call = Call {
        to: handler,
        selector: selector!("send_message"),
        calldata: vec![felt!("0x1"), HANDLER_VALUE],
    };
    let res = account.execute_v3(vec![call]).send().await.context("invoke send_message")?;
    wait_for_tx(&l3.provider(), res.transaction_hash).await
}

/// Waits until the appchain mines a block past `prev_tip` — i.e. the relayed
/// l1-handler tx has been included. Returns the new tip.
pub async fn wait_for_relay(l3: &L3InProcess, prev_tip: u64, timeout: Duration) -> Result<u64> {
    let deadline = Instant::now() + timeout;
    loop {
        let tip = current_tip(l3).await?;
        if tip > prev_tip {
            return Ok(tip);
        }
        if Instant::now() >= deadline {
            return Err(anyhow!(
                "appchain did not relay/mine the l1-handler within {timeout:?} (still at block \
                 {prev_tip})"
            ));
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

/// Builds a `SingleOwnerAccount` for the L2 prefunded dev account.
async fn l2_account(
    l2: &L2InProcess,
) -> Result<SingleOwnerAccount<JsonRpcClient<HttpTransport>, LocalWallet>> {
    let provider = l2.provider();
    let chain_id = provider.chain_id().await.context("fetch L2 chain id")?;
    let (address, private_key) = l2.prefunded_account_keys();
    let signer = LocalWallet::from_signing_key(SigningKey::from_secret_scalar(private_key));
    let mut account =
        SingleOwnerAccount::new(provider, signer, address, chain_id, ExecutionEncoding::New);
    account.set_block_id(BlockId::Tag(BlockTag::PreConfirmed));
    Ok(account)
}

async fn wait_for_tx(provider: &JsonRpcClient<HttpTransport>, tx_hash: Felt) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        match provider.get_transaction_receipt(tx_hash).await {
            Ok(_) => return Ok(()),
            Err(_) if Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
            Err(e) => return Err(anyhow!("tx {tx_hash:#x} not accepted: {e}")),
        }
    }
}
