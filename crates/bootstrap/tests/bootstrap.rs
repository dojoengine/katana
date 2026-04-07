//! Integration tests for `katana bootstrap`.
//!
//! These tests stand up a minimal in-process JSON-RPC server that mimics the subset of
//! Starknet RPC methods that the bootstrap executor touches: `chain_id`, `get_nonce`,
//! `get_class`, `get_class_hash_at`, `estimate_fee`, `add_declare_transaction`, and
//! `add_invoke_transaction`. Every incoming request is recorded so the tests can assert
//! on what bootstrap actually submitted (transaction *type*, *count*, and *nonce*) without
//! depending on a real katana node.
//!
//! The mock is intentionally permissive on response correctness — it returns just enough
//! data for `starknet-rs` to deserialize successfully and for the executor's polling loops
//! to terminate. The point is to verify the *outgoing* traffic, not to re-implement the
//! Starknet state machine.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex, OnceLock};

use jsonrpsee::server::{RpcModule, Server, ServerHandle};
use jsonrpsee::types::error::ErrorObjectOwned;
use jsonrpsee::types::Params;
use katana_bootstrap::executor::{self, execute_with_progress, BootstrapEvent, ExecutorConfig};
use katana_bootstrap::plan::{BootstrapPlan, ClassSource, DeclareStep, DeployStep};
use katana_contracts::contracts::Account as DevAccountClass;
use katana_primitives::class::ContractClass;
use katana_primitives::Felt;
use katana_rpc_types::RpcSierraContractClass;
use serde_json::{json, Value};
use starknet::core::types::FlattenedSierraClass;
use url::Url;

// ---------------------------------------------------------------------------
// Mock RPC server
// ---------------------------------------------------------------------------

/// One captured incoming JSON-RPC call.
#[derive(Debug, Clone)]
struct Recorded {
    method: String,
    params: Value,
}

/// Shared state mutated by handlers.
#[derive(Default)]
struct MockState {
    /// Every captured method call, in order.
    calls: Vec<Recorded>,
    /// Class hashes the mock should pretend are already declared *before* bootstrap runs.
    /// `get_class` consults this set so the executor's pre-check can find them.
    pre_declared: std::collections::HashSet<Felt>,
    /// FIFO of class hashes the mock should return from successive `add_declare_transaction`
    /// calls. The test pre-loads this with the hashes it expects bootstrap to declare.
    expected_declare_class_hashes: std::collections::VecDeque<Felt>,
    /// FIFO of (address, class_hash) pairs the test expects to be deployed via
    /// `add_invoke_transaction` (UDC invokes). One pair is popped per call; the
    /// address itself isn't validated against the request body because doing so would
    /// require decoding the UDC `deployContract` calldata.
    expected_deploy_addresses: std::collections::VecDeque<(Felt, Felt)>,
    /// Set of tx hashes handed out by the add_*_transaction handlers. Subsequent
    /// `getTransactionStatus` / `getTransactionReceipt` queries return success for
    /// any hash in this set — this is what the executor's `TxWaiter` polls.
    submitted_tx_hashes: std::collections::HashSet<Felt>,
    /// Contract addresses the mock should pretend are already deployed *before* the
    /// bootstrap runs. `get_class_hash_at` returns success for any address in this
    /// set so the executor's deploy precheck can find them; everything else gets
    /// `ContractNotFound`.
    pre_deployed: std::collections::HashSet<Felt>,
    /// Counter used to mint synthetic, unique transaction hashes for tx submission responses.
    next_tx_hash: u64,
}

impl MockState {
    fn next_tx_hash(&mut self) -> Felt {
        self.next_tx_hash += 1;
        Felt::from(self.next_tx_hash)
    }
}

/// A live mock server with a handle for assertions and a teardown guard.
struct MockServer {
    url: Url,
    state: Arc<Mutex<MockState>>,
    _handle: ServerHandle,
}

impl MockServer {
    async fn start() -> Self {
        let state = Arc::new(Mutex::new(MockState::default()));

        let addr = "127.0.0.1:0".parse::<SocketAddr>().unwrap();
        let server = Server::builder().build(addr).await.unwrap();
        let addr = server.local_addr().unwrap();

        let mut module = RpcModule::new(state.clone());
        register_handlers(&mut module);

        let handle = server.start(module);
        let url = Url::parse(&format!("http://{addr}/")).unwrap();

        Self { url, state, _handle: handle }
    }

    fn calls(&self) -> Vec<Recorded> {
        self.state.lock().unwrap().calls.clone()
    }

    fn calls_to(&self, method: &str) -> Vec<Recorded> {
        self.calls().into_iter().filter(|c| c.method == method).collect()
    }

    /// Tell the mock about expected outcomes so its responses look real enough for
    /// starknet-rs / the executor to keep going.
    fn expect_declare(&self, class_hash: Felt) {
        self.state.lock().unwrap().expected_declare_class_hashes.push_back(class_hash);
    }

    fn expect_deploy(&self, address: Felt, class_hash: Felt) {
        self.state.lock().unwrap().expected_deploy_addresses.push_back((address, class_hash));
    }

    /// Mark a class as already-declared on the chain so the executor's pre-check sees it.
    fn pre_declare(&self, class_hash: Felt) {
        self.state.lock().unwrap().pre_declared.insert(class_hash);
    }

    /// Mark a contract address as already-deployed so the executor's deploy
    /// precheck sees it (mirrors `pre_declare` for the deploy side).
    fn pre_deploy(&self, address: Felt) {
        self.state.lock().unwrap().pre_deployed.insert(address);
    }
}

/// A canned Sierra class JSON used as the response body for `starknet_getClass`. Loaded
/// once from the embedded `dev_account` so we know it round-trips through starknet-rs's
/// `ContractClass` deserializer correctly.
fn canned_class_response() -> Arc<Value> {
    static CACHE: OnceLock<Arc<Value>> = OnceLock::new();
    CACHE
        .get_or_init(|| {
            let class: ContractClass = DevAccountClass::CLASS.clone();
            let sierra = class.to_sierra().expect("dev_account is sierra");
            let rpc_class = RpcSierraContractClass::from(sierra);
            let flattened: FlattenedSierraClass = rpc_class.try_into().unwrap();
            Arc::new(serde_json::to_value(&flattened).unwrap())
        })
        .clone()
}

fn register_handlers(module: &mut RpcModule<Arc<Mutex<MockState>>>) {
    // Capture+respond pattern: every handler records the call (method + raw params) into
    // the shared state, then computes a response off the same locked state.
    module
        .register_method("starknet_chainId", |params, state, _| {
            record(state, "starknet_chainId", &params);
            // 'KATANA_TEST' as a short-string felt — the value is irrelevant, but
            // starknet-rs uses it for tx hash computation so it must be a valid felt.
            Ok::<_, ErrorObjectOwned>(json!("0x4b4154414e415f54455354"))
        })
        .unwrap();

    module
        .register_method("starknet_specVersion", |params, state, _| {
            record(state, "starknet_specVersion", &params);
            Ok::<_, ErrorObjectOwned>(json!("0.9.0"))
        })
        .unwrap();

    module
        .register_method("starknet_getNonce", |params, state, _| {
            record(state, "starknet_getNonce", &params);
            // Bootstrap fetches the starting nonce once and increments locally; returning
            // 0 is fine and the test asserts the resulting tx nonces are 0, 1, 2, …
            Ok::<_, ErrorObjectOwned>(json!("0x0"))
        })
        .unwrap();

    module
        .register_method("starknet_getClass", |params, state, _| {
            // Used by the executor's `is_declared` precheck (one-shot lookup before
            // every declare). Returns success only for class hashes registered via
            // `MockServer::pre_declare`.
            record(state, "starknet_getClass", &params);

            let value = parse_value(&params);
            let class_hash = value
                .as_array()
                .and_then(|arr| arr.get(1))
                .or_else(|| value.get("class_hash"))
                .and_then(|v| v.as_str())
                .and_then(|s| Felt::from_hex(s).ok());

            let guard = state.lock().unwrap();
            let known = class_hash.map(|h| guard.pre_declared.contains(&h)).unwrap_or(false);
            drop(guard);

            if known {
                Ok::<_, ErrorObjectOwned>((*canned_class_response()).clone())
            } else {
                Err(starknet_error(28, "Class hash not found"))
            }
        })
        .unwrap();

    module
        .register_method("starknet_getClassHashAt", |params, state, _| {
            // Used by the executor's `is_deployed` precheck (one-shot lookup before
            // every deploy). Returns success only for addresses registered via
            // `MockServer::pre_deploy`; everything else gets ContractNotFound so the
            // submission proceeds.
            record(state, "starknet_getClassHashAt", &params);

            let value = parse_value(&params);
            let address = value
                .as_array()
                .and_then(|arr| arr.get(1))
                .or_else(|| value.get("contract_address"))
                .and_then(|v| v.as_str())
                .and_then(|s| Felt::from_hex(s).ok());

            let guard = state.lock().unwrap();
            let known = address.map(|a| guard.pre_deployed.contains(&a)).unwrap_or(false);
            drop(guard);

            if known {
                Ok::<_, ErrorObjectOwned>(json!("0x1"))
            } else {
                Err(starknet_error(20, "Contract not found"))
            }
        })
        .unwrap();

    // ---- TxWaiter polling methods --------------------------------------------------
    //
    // After each declare/invoke submission the executor delegates the wait to
    // `katana_utils::TxWaiter`, which alternates `getTransactionStatus` until the tx
    // is included, then `getTransactionReceipt` to confirm execution succeeded. The
    // mock returns "ACCEPTED_ON_L2 / SUCCEEDED" immediately for any hash it has handed
    // out via add_*_transaction.

    module
        .register_method("starknet_getTransactionStatus", |params, state, _| {
            record(state, "starknet_getTransactionStatus", &params);
            let value = parse_value(&params);
            let hash = extract_first_felt(&value);

            let guard = state.lock().unwrap();
            let known = hash.map(|h| guard.submitted_tx_hashes.contains(&h)).unwrap_or(false);
            drop(guard);

            if known {
                // Tagged enum: { "finality_status": "ACCEPTED_ON_L2", "execution_status":
                // "SUCCEEDED" }
                Ok::<_, ErrorObjectOwned>(json!({
                    "finality_status": "ACCEPTED_ON_L2",
                    "execution_status": "SUCCEEDED",
                }))
            } else {
                Err(starknet_error(29, "Transaction hash not found"))
            }
        })
        .unwrap();

    module
        .register_method("starknet_getTransactionReceipt", |params, state, _| {
            record(state, "starknet_getTransactionReceipt", &params);
            let value = parse_value(&params);
            let hash = extract_first_felt(&value);

            let guard = state.lock().unwrap();
            let known = hash.map(|h| guard.submitted_tx_hashes.contains(&h)).unwrap_or(false);
            drop(guard);

            if !known {
                return Err(starknet_error(29, "Transaction hash not found"));
            }

            // Confirmed-block invoke receipt with SUCCEEDED execution. Shape matches
            // katana_rpc_types::receipt::TxReceiptWithBlockInfo. Doubles as both
            // declare and invoke responses since TxWaiter only inspects the
            // `execution_status` and `finality_status` fields.
            Ok::<_, ErrorObjectOwned>(json!({
                "transaction_hash": format!("{:#x}", hash.unwrap_or(Felt::ZERO)),
                "type": "INVOKE",
                "actual_fee": { "amount": "0x0", "unit": "FRI" },
                "finality_status": "ACCEPTED_ON_L2",
                "execution_status": "SUCCEEDED",
                "messages_sent": [],
                "events": [],
                "execution_resources": { "l1_gas": 0, "l1_data_gas": 0, "l2_gas": 0 },
                "block_hash": "0x1",
                "block_number": 1,
            }))
        })
        .unwrap();

    module
        .register_method("starknet_getBlockWithTxs", |params, state, _| {
            record(state, "starknet_getBlockWithTxs", &params);
            // Minimal pre-confirmed block. starknet-rs's account fetches this *after*
            // estimate_fee in order to compute a median tip; an empty `transactions` list
            // results in `median_tip() == 0`, which is exactly what we want.
            Ok::<_, ErrorObjectOwned>(json!({
                "transactions": [],
                "block_number": 1,
                "timestamp": 0,
                "sequencer_address": "0x0",
                "l1_gas_price":      { "price_in_fri": "0x1", "price_in_wei": "0x1" },
                "l2_gas_price":      { "price_in_fri": "0x1", "price_in_wei": "0x1" },
                "l1_data_gas_price": { "price_in_fri": "0x1", "price_in_wei": "0x1" },
                "l1_da_mode": "CALLDATA",
                "starknet_version": "0.13.0",
            }))
        })
        .unwrap();

    module
        .register_method("starknet_estimateFee", |params, state, _| {
            record(state, "starknet_estimateFee", &params);
            // The values themselves don't matter — bootstrap just forwards them as
            // resource bounds. We just need a syntactically-valid v3 FeeEstimate.
            Ok::<_, ErrorObjectOwned>(json!([{
                "l1_gas_consumed": "0x1",
                "l1_gas_price": "0x1",
                "l2_gas_consumed": "0x1",
                "l2_gas_price": "0x1",
                "l1_data_gas_consumed": "0x1",
                "l1_data_gas_price": "0x1",
                "overall_fee": "0x1",
                "unit": "FRI"
            }]))
        })
        .unwrap();

    module
        .register_method("starknet_addDeclareTransaction", |params, state, _| {
            record(state, "starknet_addDeclareTransaction", &params);
            let mut guard = state.lock().unwrap();

            let class_hash = guard.expected_declare_class_hashes.pop_front().ok_or_else(|| {
                starknet_error(
                    1,
                    "mock received an unexpected add_declare_transaction call (no expected class \
                     hashes left)",
                )
            })?;
            let tx_hash = guard.next_tx_hash();
            guard.submitted_tx_hashes.insert(tx_hash);

            Ok::<_, ErrorObjectOwned>(json!({
                "transaction_hash": format!("{tx_hash:#x}"),
                "class_hash": format!("{class_hash:#x}"),
            }))
        })
        .unwrap();

    module
        .register_method("starknet_addInvokeTransaction", |params, state, _| {
            record(state, "starknet_addInvokeTransaction", &params);
            let mut guard = state.lock().unwrap();

            // We still pop from the queue so the test can size-check that the right
            // number of deploys were submitted, but the address itself isn't validated
            // against the request body — the executor's BootstrapEvent::DeployCompleted
            // already covers that contract.
            let _ = guard.expected_deploy_addresses.pop_front().ok_or_else(|| {
                starknet_error(
                    1,
                    "mock received an unexpected add_invoke_transaction call (no expected deploy \
                     addresses left)",
                )
            })?;
            let tx_hash = guard.next_tx_hash();
            guard.submitted_tx_hashes.insert(tx_hash);

            Ok::<_, ErrorObjectOwned>(json!({
                "transaction_hash": format!("{tx_hash:#x}"),
            }))
        })
        .unwrap();
}

/// Pull the first felt out of a serde_json `Value`, regardless of whether the request
/// arrived as a positional array (`["0x1"]`) or a named object (`{"transaction_hash": "0x1"}`).
fn extract_first_felt(value: &Value) -> Option<Felt> {
    let s = value
        .as_array()
        .and_then(|a| a.first())
        .or_else(|| value.get("transaction_hash"))
        .and_then(|v| v.as_str())?;
    Felt::from_hex(s).ok()
}

fn record(state: &Arc<Mutex<MockState>>, method: &str, params: &Params<'_>) {
    let value = parse_value(params);
    state.lock().unwrap().calls.push(Recorded { method: method.to_string(), params: value });
}

/// Convert jsonrpsee's `Params` into a serde_json `Value` for capture/inspection. We
/// don't care whether the request used positional or named params — we just want a
/// stable representation.
fn parse_value(params: &Params<'_>) -> Value {
    let raw = params.as_str().unwrap_or("null");
    serde_json::from_str(raw).unwrap_or(Value::Null)
}

fn starknet_error(code: i32, msg: &'static str) -> ErrorObjectOwned {
    ErrorObjectOwned::owned::<()>(code, msg, None)
}

// ---------------------------------------------------------------------------
// Plan helpers
// ---------------------------------------------------------------------------

/// Build a plan with N declares of the embedded `dev_account` class. Each declare gets
/// its own local alias so they don't collide. (The class hash will be the same for all
/// because the underlying class is identical — that's fine for these tests since the
/// mock just records calls and doesn't dedupe by class hash.)
fn dev_account_step(local_alias: &str) -> DeclareStep {
    use katana_contracts::contracts::Account as A;
    DeclareStep {
        name: local_alias.to_string(),
        class: std::sync::Arc::new(A::CLASS.clone()),
        class_hash: A::HASH,
        casm_hash: A::CASM_HASH,
        source: ClassSource::Embedded("dev_account"),
    }
}

fn deploy_step(class_name: &str, class_hash: Felt, salt: Felt, calldata: Vec<Felt>) -> DeployStep {
    DeployStep {
        label: None,
        class_hash,
        class_name: class_name.to_string(),
        salt,
        unique: false,
        calldata,
    }
}

fn dummy_signer_config(url: Url) -> ExecutorConfig {
    // Any non-zero felt is a valid private key for signing purposes; the mock doesn't
    // verify signatures.
    ExecutorConfig {
        rpc_url: url,
        account_address: Felt::from(0x1234u64).into(),
        private_key: Felt::from(0xabcdu64),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn declare_only_submits_one_declare_with_nonce_zero() {
    let mock = MockServer::start().await;
    mock.expect_declare(katana_contracts::contracts::Account::HASH);

    let plan = BootstrapPlan { declares: vec![dev_account_step("a")], deploys: vec![] };

    let cfg = dummy_signer_config(mock.url.clone());
    executor::execute(&plan, &cfg).await.expect("bootstrap should succeed");

    let declares = mock.calls_to("starknet_addDeclareTransaction");
    assert_eq!(declares.len(), 1, "expected exactly one declare submission");
    assert_eq!(mock.calls_to("starknet_addInvokeTransaction").len(), 0);

    assert_tx_type(&declares[0], "declare_transaction", "DECLARE", "0x3");
    assert_eq!(declare_nonce(&declares[0]), Felt::ZERO, "first tx must use nonce 0");

    // Sanity: bootstrap fetched the nonce exactly once at startup.
    assert_eq!(mock.calls_to("starknet_getNonce").len(), 1);
}

#[tokio::test]
async fn declare_then_deploy_uses_sequential_nonces() {
    let mock = MockServer::start().await;
    let class_hash = katana_contracts::contracts::Account::HASH;
    mock.expect_declare(class_hash);

    // Address values are arbitrary — the mock just needs *something* to flag as
    // "deployed" once the invoke comes in. The executor computes the deterministic
    // address itself and polls `get_class_hash_at` for it; we use the same derivation.
    let salt = Felt::from(0x1u64);
    let calldata = vec![Felt::from(0xaau64)];
    let address = katana_primitives::utils::get_contract_address(
        salt,
        class_hash,
        &calldata,
        Felt::ZERO.into(),
    );
    mock.expect_deploy(address, class_hash);

    let plan = BootstrapPlan {
        declares: vec![dev_account_step("a")],
        deploys: vec![deploy_step("a", class_hash, salt, calldata)],
    };

    let cfg = dummy_signer_config(mock.url.clone());
    executor::execute(&plan, &cfg).await.expect("bootstrap should succeed");

    let declares = mock.calls_to("starknet_addDeclareTransaction");
    let invokes = mock.calls_to("starknet_addInvokeTransaction");

    assert_eq!(declares.len(), 1, "exactly one declare");
    assert_eq!(invokes.len(), 1, "exactly one deploy invoke");

    assert_tx_type(&declares[0], "declare_transaction", "DECLARE", "0x3");
    assert_tx_type(&invokes[0], "invoke_transaction", "INVOKE", "0x3");

    assert_eq!(declare_nonce(&declares[0]), Felt::ZERO);
    assert_eq!(invoke_nonce(&invokes[0]), Felt::ONE);
}

#[tokio::test]
async fn multiple_deploys_increment_nonce() {
    let mock = MockServer::start().await;
    let class_hash = katana_contracts::contracts::Account::HASH;
    mock.expect_declare(class_hash);

    let salts = [Felt::from(0x1u64), Felt::from(0x2u64), Felt::from(0x3u64)];
    let mut deploys = Vec::new();
    for salt in salts {
        let calldata = vec![Felt::from(0xaau64)];
        let addr = katana_primitives::utils::get_contract_address(
            salt,
            class_hash,
            &calldata,
            Felt::ZERO.into(),
        );
        mock.expect_deploy(addr, class_hash);
        deploys.push(deploy_step("a", class_hash, salt, calldata));
    }

    let plan = BootstrapPlan { declares: vec![dev_account_step("a")], deploys };

    let cfg = dummy_signer_config(mock.url.clone());
    executor::execute(&plan, &cfg).await.expect("bootstrap should succeed");

    let invokes = mock.calls_to("starknet_addInvokeTransaction");
    assert_eq!(invokes.len(), 3);

    let nonces: Vec<Felt> = invokes.iter().map(invoke_nonce).collect();
    assert_eq!(
        nonces,
        vec![Felt::ONE, Felt::from(2u64), Felt::from(3u64)],
        "deploy nonces must be sequential after the leading declare"
    );
}

#[tokio::test]
async fn already_declared_class_is_skipped_and_does_not_burn_a_nonce() {
    // Bootstrap is canonically idempotent: re-running a plan whose classes are
    // already on-chain should converge silently. The skipped declare must NOT
    // consume a nonce, so the following deploy starts at nonce 0.
    let mock = MockServer::start().await;
    let class_hash = katana_contracts::contracts::Account::HASH;
    mock.pre_declare(class_hash);

    let salt = Felt::from(0x9u64);
    let calldata = vec![Felt::from(0xbbu64)];
    let address = katana_primitives::utils::get_contract_address(
        salt,
        class_hash,
        &calldata,
        Felt::ZERO.into(),
    );
    mock.expect_deploy(address, class_hash);

    let plan = BootstrapPlan {
        declares: vec![dev_account_step("a")],
        deploys: vec![deploy_step("a", class_hash, salt, calldata)],
    };

    let cfg = dummy_signer_config(mock.url.clone());
    executor::execute(&plan, &cfg).await.expect("bootstrap should succeed");

    assert_eq!(
        mock.calls_to("starknet_addDeclareTransaction").len(),
        0,
        "already-declared classes should be silently skipped"
    );

    let invokes = mock.calls_to("starknet_addInvokeTransaction");
    assert_eq!(invokes.len(), 1);
    // Critical: because the declare was skipped (not just deduped post-submit), the
    // deploy must reuse nonce 0 — bootstrap should NOT burn a nonce on a no-op declare.
    assert_eq!(invoke_nonce(&invokes[0]), Felt::ZERO);
}

#[tokio::test]
async fn already_deployed_contract_is_skipped_and_does_not_burn_a_nonce() {
    // Mirror of the declare-side test on the deploy side. A contract that's already
    // at the deterministic address should be silently skipped, and the next op (here
    // a fresh deploy of a different salt) should reuse the un-burned nonce.
    let mock = MockServer::start().await;
    let class_hash = katana_contracts::contracts::Account::HASH;
    mock.expect_declare(class_hash);

    // Deploy 1: pre-deployed → should be skipped.
    let salt_a = Felt::from(0x1u64);
    let calldata_a = vec![Felt::from(0xaau64)];
    let addr_a = katana_primitives::utils::get_contract_address(
        salt_a,
        class_hash,
        &calldata_a,
        Felt::ZERO.into(),
    );
    mock.pre_deploy(addr_a);

    // Deploy 2: not pre-deployed → should go through.
    let salt_b = Felt::from(0x2u64);
    let calldata_b = vec![Felt::from(0xbbu64)];
    let addr_b = katana_primitives::utils::get_contract_address(
        salt_b,
        class_hash,
        &calldata_b,
        Felt::ZERO.into(),
    );
    mock.expect_deploy(addr_b, class_hash);

    let plan = BootstrapPlan {
        declares: vec![dev_account_step("a")],
        deploys: vec![
            deploy_step("a", class_hash, salt_a, calldata_a),
            deploy_step("a", class_hash, salt_b, calldata_b),
        ],
    };

    let cfg = dummy_signer_config(mock.url.clone());
    executor::execute(&plan, &cfg).await.expect("bootstrap should succeed");

    let invokes = mock.calls_to("starknet_addInvokeTransaction");
    assert_eq!(invokes.len(), 1, "only the not-yet-deployed contract should submit");
    // Declare took nonce 0; deploy 1 was skipped (no nonce burned); deploy 2 takes nonce 1.
    assert_eq!(invoke_nonce(&invokes[0]), Felt::ONE);
}

#[tokio::test]
async fn execute_with_progress_emits_events_in_order() {
    // Verifies the streaming variant used by the TUI: with one declare and two deploys,
    // the receiver should observe Started/Completed pairs in plan order, then Done.
    let mock = MockServer::start().await;
    let class_hash = katana_contracts::contracts::Account::HASH;
    mock.expect_declare(class_hash);

    let mut deploys = Vec::new();
    for salt in [Felt::from(0x10u64), Felt::from(0x20u64)] {
        let calldata = vec![Felt::from(0x99u64)];
        let addr = katana_primitives::utils::get_contract_address(
            salt,
            class_hash,
            &calldata,
            Felt::ZERO.into(),
        );
        mock.expect_deploy(addr, class_hash);
        deploys.push(deploy_step("a", class_hash, salt, calldata));
    }

    let plan = BootstrapPlan { declares: vec![dev_account_step("a")], deploys };
    let cfg = dummy_signer_config(mock.url.clone());

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    execute_with_progress(&plan, &cfg, Some(tx)).await.expect("bootstrap should succeed");

    // Drain the receiver into a Vec for ordered assertions.
    let mut events = Vec::new();
    while let Ok(event) = rx.try_recv() {
        events.push(event);
    }

    // Map each event to a short tag so the assertion is readable.
    fn tag(e: &BootstrapEvent) -> String {
        match e {
            BootstrapEvent::DeclareStarted { idx, .. } => format!("declare-start[{idx}]"),
            BootstrapEvent::DeclareCompleted { idx, .. } => format!("declare-done[{idx}]"),
            BootstrapEvent::DeployStarted { idx, .. } => format!("deploy-start[{idx}]"),
            BootstrapEvent::DeployCompleted { idx, .. } => format!("deploy-done[{idx}]"),
            BootstrapEvent::Done { .. } => "done".to_string(),
            BootstrapEvent::Failed { .. } => "failed".to_string(),
        }
    }
    let tags: Vec<String> = events.iter().map(tag).collect();
    assert_eq!(
        tags,
        vec![
            "declare-start[0]",
            "declare-done[0]",
            "deploy-start[0]",
            "deploy-done[0]",
            "deploy-start[1]",
            "deploy-done[1]",
            "done",
        ],
    );

    // Spot-check the typed payloads of the terminal Done event.
    let Some(BootstrapEvent::Done { report }) = events.last() else {
        panic!("last event should be Done");
    };
    assert_eq!(report.declared.len(), 1);
    assert_eq!(report.deployed.len(), 2);
}

// ---------------------------------------------------------------------------
// Param extraction helpers
// ---------------------------------------------------------------------------

fn declare_nonce(call: &Recorded) -> Felt {
    extract_nonce(&call.params, "declare_transaction")
}

fn invoke_nonce(call: &Recorded) -> Felt {
    extract_nonce(&call.params, "invoke_transaction")
}

/// Assert the recorded submission is a v3 transaction of the expected type. The Starknet
/// JSON-RPC encodes the transaction kind via two fields on the inner tx object: a string
/// `type` (e.g. `"DECLARE"`) and a `version` (e.g. `"0x3"`). Asserting both is what
/// pins this test to v3 (rather than accepting any version of the type).
fn assert_tx_type(call: &Recorded, tx_field: &str, expected_type: &str, expected_version: &str) {
    let tx = call
        .params
        .get(tx_field)
        .or_else(|| call.params.as_array().and_then(|a| a.first()))
        .unwrap_or_else(|| panic!("missing `{tx_field}` in params: {}", call.params));
    let actual_type = tx.get("type").and_then(|v| v.as_str()).unwrap_or("<missing>");
    let actual_version = tx.get("version").and_then(|v| v.as_str()).unwrap_or("<missing>");
    assert_eq!(actual_type, expected_type, "tx type mismatch");
    assert_eq!(actual_version, expected_version, "tx version mismatch");
}

fn extract_nonce(params: &Value, tx_field: &str) -> Felt {
    // Params can be sent as either an object (`{"declare_transaction": {...}}`) or as
    // a positional array. starknet-rs uses the named-object form, but the test stays
    // robust either way.
    let tx = params
        .get(tx_field)
        .or_else(|| params.as_array().and_then(|a| a.first()))
        .unwrap_or_else(|| panic!("missing `{tx_field}` in params: {params}"));
    let nonce_str = tx
        .get("nonce")
        .and_then(|v| v.as_str())
        .unwrap_or_else(|| panic!("missing nonce in {tx_field}: {tx}"));
    Felt::from_hex(nonce_str).expect("nonce should be hex felt")
}
