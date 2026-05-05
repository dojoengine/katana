//! TEE settlement backend — submits `PiltoverInput::TeeInput` to the Piltover contract.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Result};
use cainome::cairo_serde::ContractAddress as CainomeContractAddress;
use cainome::rs::abigen;
use garaga_rs::definitions::CurveID;
use katana_primitives::{ContractAddress, Felt};
use katana_rpc_types::{L1ToL2Message, L2ToL1Message};
use katana_tee::amd::OnchainProof;
use saya_core::prover::TeeProof;
use saya_core::service::{Daemon, FinishHandle, ShutdownHandle};
use saya_core::settlement::{SettlementBackend, SettlementCursor, TeeSettlementBackendBuilder};
use sha3::{Digest, Keccak256};
use starknet::accounts::{ExecutionEncoding, SingleOwnerAccount};
use starknet::core::types::{BlockId, BlockTag, FunctionCall, TransactionReceipt};
use starknet::macros::selector;
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::{JsonRpcClient, Provider};
use starknet::signers::{LocalWallet, SigningKey};
use tokio::sync::mpsc::{Receiver, Sender};
use tracing::{debug, error};
use url::Url;

const POLLING_INTERVAL: Duration = Duration::from_secs(2);

abigen!(PiltoverAppchainCoreContract, [
    {
        "type": "interface",
        "name": "piltover::interface::IAppchain",
        "items": [
            {
                "type": "function",
                "name": "update_state",
                "inputs": [
                    {
                        "name": "piltover_input",
                        "type": "piltover::input::component::PiltoverInput"
                    }
                ],
                "outputs": [],
                "state_mutability": "external"
            }
        ]
    },
    {
        "type": "enum",
        "name": "piltover::input::component::PiltoverInput",
        "variants": [
            {
                "name": "LayoutBridgeOutputNoDa",
                "type": "core::array::Span::<core::felt252>"
            },
            {
                "name": "LayoutBridgeOutputWithDa",
                "type": "(core::array::Span::<core::felt252>, piltover::input::layout_bridge::DaLayerInfo)"
            },
            {
                "name": "TeeInput",
                "type": "piltover::input::tee_input::TEEInput"
            }
        ]
    },
    {
        "type": "struct",
        "name": "piltover::input::tee_input::TEEInput",
        "members": [
            {
                "name": "sp1_proof",
                "type": "core::array::Span::<core::felt252>"
            },
            { "name": "prev_state_root", "type": "core::felt252" },
            { "name": "state_root", "type": "core::felt252" },
            { "name": "prev_block_hash", "type": "core::felt252" },
            { "name": "block_hash", "type": "core::felt252" },
            { "name": "prev_block_number", "type": "core::felt252" },
            { "name": "block_number", "type": "core::felt252" },
            { "name": "messages_commitment", "type": "core::felt252" },
            {
                "name": "messages_to_starknet",
                "type": "core::array::Span::<piltover::input::snos_output::MessageToStarknet>"
            },
            {
                "name": "messages_to_appchain",
                "type": "core::array::Span::<piltover::input::snos_output::MessageToAppchain>"
            },
            {
                "name": "l1_to_l2_msg_hashes",
                "type": "core::array::Span::<core::felt252>"
            }
        ]
    },
    {
        "type": "struct",
        "name": "piltover::input::snos_output::MessageToStarknet",
        "members": [
            {
                "name": "from_address",
                "type": "core::starknet::contract_address::ContractAddress"
            },
            {
                "name": "to_address",
                "type": "core::starknet::contract_address::ContractAddress"
            },
            {
                "name": "payload",
                "type": "core::array::Span::<core::felt252>"
            }
        ]
    },
    {
        "type": "struct",
        "name": "piltover::input::snos_output::MessageToAppchain",
        "members": [
            {
                "name": "from_address",
                "type": "core::starknet::contract_address::ContractAddress"
            },
            {
                "name": "to_address",
                "type": "core::starknet::contract_address::ContractAddress"
            },
            { "name": "nonce", "type": "core::felt252" },
            { "name": "selector", "type": "core::felt252" },
            {
                "name": "payload",
                "type": "core::array::Span::<core::felt252>"
            }
        ]
    },
    {
        "type": "struct",
        "name": "piltover::input::layout_bridge::DaLayerInfo",
        "members": [
            { "name": "height", "type": "core::felt252" },
            { "name": "commitment", "type": "core::felt252" },
            { "name": "namespace", "type": "core::felt252" }
        ]
    }
]);

/// Computes the Starknet L1→L2 message hash using keccak256.
///
/// Matches the Ethereum StarknetMessaging.sol formula:
/// `keccak256(abi.encodePacked(from_address, to_address, nonce, selector, payload.length,
/// payload))` where `from_address` is a 20-byte Ethereum address (lower 20 bytes of the felt252).
fn compute_l1_to_l2_msg_hash(msg: &L1ToL2Message) -> Felt {
    let mut hasher = Keccak256::new();
    hasher.update(&msg.from_address.as_slice()[12..]);
    hasher.update(msg.to_address.to_bytes_be());
    hasher.update(msg.nonce.to_bytes_be());
    hasher.update(msg.entry_point_selector.to_bytes_be());
    hasher.update(Felt::from(msg.payload.len() as u64).to_bytes_be());
    for p in &msg.payload {
        hasher.update(p.to_bytes_be());
    }
    Felt::from_bytes_be(&hasher.finalize().into())
}

fn messages_to_starknet(msgs: &[L2ToL1Message]) -> Vec<MessageToStarknet> {
    msgs.iter()
        .map(|m| MessageToStarknet {
            from_address: CainomeContractAddress(m.from_address.into()),
            to_address: CainomeContractAddress(m.to_address),
            payload: m.payload.clone(),
        })
        .collect()
}

fn messages_to_appchain(msgs: &[L1ToL2Message]) -> Vec<MessageToAppchain> {
    msgs.iter()
        .map(|m| {
            let from_address = Felt::from_bytes_be_slice(m.from_address.as_slice());
            let from_address = CainomeContractAddress(from_address);

            let to_address = CainomeContractAddress(m.to_address.into());

            MessageToAppchain {
                from_address,
                to_address,
                nonce: m.nonce,
                selector: m.entry_point_selector,
                payload: m.payload.clone(),
            }
        })
        .collect()
}

/// Builds `PiltoverInput::TeeInput` calldata using the piltover bindgen.
///
/// In real proving mode (`mock_prove = false`), `proof.data` is a JSON-encoded
/// `OnchainProof` produced by the SP1 prover network, and is converted to
/// Garaga calldata via [`StarknetCalldata::from_proof`].
///
/// In mock proving mode (`mock_prove = true`), `proof.data` is a raw
/// big-endian felt buffer produced by [`crate::mock_proof::serialize_mock_journal`]
/// — a Cairo-Serde-serialized stub `VerifierJournal` — which the paired
/// `piltover_mock_amd_tee_registry` contract decodes as-is. We forward the
/// felts directly to `TEEInput.sp1_proof` without going through `OnchainProof`.
fn build_piltover_tee_input(proof: &TeeProof, mock_prove: bool) -> Result<PiltoverInput> {
    let sp1_proof: Vec<Felt> = if mock_prove {
        crate::mock_proof::bytes_to_felts(&proof.data)
            .ok_or_else(|| anyhow::anyhow!("mock proof data length is not a multiple of 32"))?
    } else {
        let proof = OnchainProof::decode_json(&proof.data)?;
        onchain_proof_to_calldata(&proof)?
    };

    let l1_to_l2_msg_hashes: Vec<Felt> =
        proof.l1_to_l2_messages.iter().map(compute_l1_to_l2_msg_hash).collect();

    Ok(PiltoverInput::TeeInput(TEEInput {
        sp1_proof,
        prev_state_root: proof.prev_state_root,
        state_root: proof.state_root,
        prev_block_hash: proof.prev_block_hash,
        block_hash: proof.block_hash,
        prev_block_number: proof.prev_block_number.into(),
        block_number: proof.block_number.into(),
        messages_commitment: proof.messages_commitment,
        messages_to_starknet: messages_to_starknet(&proof.l2_to_l1_messages),
        messages_to_appchain: messages_to_appchain(&proof.l1_to_l2_messages),
        l1_to_l2_msg_hashes,
    }))
}

/// Settlement backend that submits TEE proofs to the Piltover contract via `update_state`.
#[derive(Debug)]
pub struct TeePiltoverSettlementBackend {
    provider: Arc<JsonRpcClient<HttpTransport>>,
    account: SingleOwnerAccount<Arc<JsonRpcClient<HttpTransport>>, LocalWallet>,
    piltover_address: ContractAddress,
    /// When `true`, decode `TeeProof.data` as a raw felt buffer (mock journal)
    /// instead of `OnchainProof` JSON. Must be paired with the upstream
    /// `TeeProver` running with `mock_prove = true`.
    mock_prove: bool,
    proof_channel: Receiver<TeeProof>,
    cursor_channel: Sender<SettlementCursor>,
    finish_handle: FinishHandle,
}

#[derive(Debug)]
pub struct TeePiltoverSettlementBackendBuilder {
    rpc_url: Url,
    piltover_address: ContractAddress,
    account_address: ContractAddress,
    account_private_key: Felt,
    mock_prove: bool,
    proof_channel: Option<Receiver<TeeProof>>,
    cursor_channel: Option<Sender<SettlementCursor>>,
}

impl TeePiltoverSettlementBackendBuilder {
    pub fn new(
        rpc_url: Url,
        piltover_address: ContractAddress,
        account_address: ContractAddress,
        account_private_key: Felt,
        mock_prove: bool,
    ) -> Self {
        Self {
            rpc_url,
            piltover_address,
            account_address,
            account_private_key,
            mock_prove,
            proof_channel: None,
            cursor_channel: None,
        }
    }
}

impl TeeSettlementBackendBuilder for TeePiltoverSettlementBackendBuilder {
    type Backend = TeePiltoverSettlementBackend;

    async fn build(self) -> Result<Self::Backend> {
        let provider = Arc::new(JsonRpcClient::new(HttpTransport::new(self.rpc_url)));
        let chain_id = provider.chain_id().await?;

        let mut account = SingleOwnerAccount::new(
            provider.clone(),
            LocalWallet::from_signing_key(SigningKey::from_secret_scalar(self.account_private_key)),
            self.account_address.into(),
            chain_id,
            ExecutionEncoding::New,
        );
        account.set_block_id(BlockId::Tag(BlockTag::Latest));

        Ok(TeePiltoverSettlementBackend {
            provider,
            account,
            piltover_address: self.piltover_address,
            mock_prove: self.mock_prove,
            proof_channel: self
                .proof_channel
                .ok_or_else(|| anyhow::anyhow!("`proof_channel` not set"))?,
            cursor_channel: self
                .cursor_channel
                .ok_or_else(|| anyhow::anyhow!("`cursor_channel` not set"))?,
            finish_handle: FinishHandle::new(),
        })
    }

    fn proof_channel(mut self, proof_channel: Receiver<TeeProof>) -> Self {
        self.proof_channel = Some(proof_channel);
        self
    }

    fn cursor_channel(mut self, cursor_channel: Sender<SettlementCursor>) -> Self {
        self.cursor_channel = Some(cursor_channel);
        self
    }
}

impl TeePiltoverSettlementBackend {
    async fn get_piltover_block_number(&self) -> Result<Felt> {
        let raw = self
            .provider
            .call(
                FunctionCall {
                    contract_address: self.piltover_address.into(),
                    entry_point_selector: selector!("get_state"),
                    calldata: vec![],
                },
                BlockId::Tag(BlockTag::Latest),
            )
            .await?;
        // AppchainState: [state_root, block_number, block_hash] — block_number is index 1.
        raw.get(1).copied().ok_or_else(|| anyhow::anyhow!("get_state returned fewer than 2 felts"))
    }

    async fn watch_tx(&self, tx_hash: Felt) -> Result<()> {
        loop {
            tokio::time::sleep(POLLING_INTERVAL).await;
            match self.provider.get_transaction_receipt(tx_hash).await {
                Ok(receipt) => match receipt.receipt {
                    TransactionReceipt::Invoke(r) => {
                        use starknet::core::types::ExecutionResult;
                        match r.execution_result {
                            ExecutionResult::Succeeded => return Ok(()),
                            ExecutionResult::Reverted { reason } => {
                                return Err(anyhow::anyhow!("Transaction reverted: {reason}"))
                            }
                        }
                    }
                    _ => return Ok(()),
                },
                Err(starknet::providers::ProviderError::StarknetError(
                    starknet::core::types::StarknetError::TransactionHashNotFound,
                )) => continue,
                Err(e) => return Err(e.into()),
            }
        }
    }

    async fn run(mut self) {
        loop {
            let proof = tokio::select! {
                _ = self.finish_handle.shutdown_requested() => break,
                p = self.proof_channel.recv() => match p {
                    Some(p) => p,
                    None => {
                        debug!("Proof channel closed, shutting down");
                        break;
                    }
                },
            };

            let tee_input = match build_piltover_tee_input(&proof, self.mock_prove) {
                Ok(c) => c,
                Err(e) => {
                    error!(
                        "Failed to build TEE calldata for block {}: {}",
                        proof.block_number.to_hex_string(),
                        e
                    );
                    continue;
                }
            };

            let piltover_contract =
                PiltoverAppchainCoreContract::new(self.piltover_address.into(), &self.account);

            let _ = match piltover_contract.update_state(&tee_input).estimate_fee().await {
                Ok(f) => f,
                Err(e) => {
                    error!(
                        "Fee estimation failed for block {}: {}",
                        proof.block_number.to_hex_string(),
                        e
                    );
                    continue;
                }
            };

            let transaction = match piltover_contract.update_state(&tee_input).send().await {
                Ok(t) => t,
                Err(e) => {
                    error!("Settlement transaction failed for block {}: {}", proof.block_number, e);
                    continue;
                }
            };

            match self.watch_tx(transaction.transaction_hash).await {
                Ok(()) => {}
                Err(e) => {
                    error!(
                        "Settlement tx confirmation failed for block {}: {}",
                        proof.block_number, e
                    );
                    continue;
                }
            }

            let new_cursor = SettlementCursor {
                block_number: u64::try_from(proof.block_number).unwrap_or_else(|_| {
                    panic!(
                        "Block number {} does not fit in u64",
                        proof.block_number.to_hex_string()
                    )
                }),
                transaction_hash: transaction.transaction_hash,
            };

            tokio::select! {
                _ = self.finish_handle.shutdown_requested() => break,
                _ = self.cursor_channel.send(new_cursor) => {},
            }
        }

        debug!("TeePiltoverSettlementBackend graceful shutdown finished");
        self.finish_handle.finish();
    }
}

impl SettlementBackend for TeePiltoverSettlementBackend {
    async fn get_block_number(&self) -> Result<Felt> {
        self.get_piltover_block_number().await
    }
}

impl Daemon for TeePiltoverSettlementBackend {
    fn shutdown_handle(&self) -> ShutdownHandle {
        self.finish_handle.shutdown_handle()
    }

    fn start(self) {
        tokio::spawn(self.run());
    }
}

pub fn onchain_proof_to_calldata(proof: &OnchainProof) -> Result<Vec<Felt>> {
    use garaga_rs::calldata::full_proof_with_hints::groth16::{
        get_groth16_calldata, get_sp1_vk, Groth16Proof,
    };

    // Check for mock/empty proof
    if proof.onchain_proof.is_empty() {
        return Err(anyhow!("Cannot generate calldata from empty proof (mock mode?)"));
    }

    // Extract components from proof:
    // - vkey: The verification key (program ID)
    // - public_values: From raw_proof.journal
    // - proof_bytes: The onchain_proof bytes (includes 4-byte selector)

    // The verifier_id is the SP1 program vkey as bytes
    let vkey_bytes = proof.program_id.verifier_id.as_slice().to_vec();
    // The public values are in the raw_proof journal
    let public_values = proof.raw_proof.journal.to_vec();
    // The onchain_proof contains the groth16 proof with selector
    let proof_bytes = proof.onchain_proof.to_vec();
    // Create Garaga Groth16 proof structure
    // from_sp1 expects: (vkey, public_values, proof_with_selector)
    let groth16_proof = Groth16Proof::from_sp1(vkey_bytes, public_values, proof_bytes);
    // Get the universal SP1 Groth16 verification key from Garaga
    let sp1_vk = get_sp1_vk();

    let calldata = get_groth16_calldata(&groth16_proof, &sp1_vk, CurveID::BN254)
        .map_err(|e| anyhow!("failed to generate calldata: {e}"))?;
    let calldata = calldata.into_iter().map(Felt::from).collect();

    Ok(calldata)
}
