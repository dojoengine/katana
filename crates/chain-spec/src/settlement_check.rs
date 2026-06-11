//! Validates that a Starknet settlement contract referenced by a chain spec is actually configured
//! for this chain. Run on node startup; mismatch aborts the launch before any sequencing work.
//!
//! Behavior depends on the chain's [`SettlementProofKind`] and the on-chain
//! [`piltover::ProgramInfo`] variant:
//! - [`SettlementProofKind::ValidityProof`] expects [`ProgramInfo::StarknetOs`] and checks the SNOS
//!   / layout-bridge / bootloader program hashes plus the SNOS config hash.
//! - [`SettlementProofKind::Tee`] expects [`ProgramInfo::KatanaTee`] and checks the
//!   `KatanaTeeConfig1`-tagged config hash only.
//! - Cross-mode mismatch (chain spec says ZK but contract is TEE-mode, or vice versa) is a
//!   [`SettlementValidationError::InvalidProgramInfoVariant`] error.
//!
//! Fact-registry validation is intentionally not performed here — the chain spec doesn't currently
//! store the expected fact registry, and `katana init` performs that check at deploy time.

use std::sync::Arc;

use katana_primitives::cairo::ShortString;
use katana_primitives::{felt, ContractAddress, Felt};
use piltover::{AppchainContractReader, ProgramInfo};
use starknet::core::crypto::compute_hash_on_elements;
use starknet::core::types::StarknetError;
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::{JsonRpcClient, ProviderError};
use thiserror::Error;
use url::Url;

use crate::tee::compute_katana_tee_config_hash;
use crate::SettlementProofKind;

/// The StarknetOS program (SNOS) is the cairo program that executes the state
/// transition of a new Katana block from the previous block. The settlement contract must know this
/// hash so it only accepts state transitions produced by a valid SNOS program.
///
/// <https://github.com/starkware-libs/sequencer/blob/v0.16.0-rc.0/crates/apollo_starknet_os_program/src/cairo/starkware/starknet/core/os/os.cairo>
pub const SNOS_PROGRAM_HASH: Felt =
    felt!("0x10e5341a417427d140af8f5def7d2cc687d84591ff8ec241623c590b5ca8c80");

/// SNOS requires the `all_cairo` layout, which the on-Starknet Cairo verifier can't verify
/// directly. The Layout Bridge program acts as a Cairo verifier using a layout the Cairo verifier
/// supports, producing a new proof verifiable on-chain.
///
/// <https://github.com/starkware-libs/cairo-lang/blob/8276ac35830148a397e1143389f23253c8b80e93/src/starkware/cairo/cairo_verifier/layouts/all_cairo/cairo_verifier.cairo>
pub const LAYOUT_BRIDGE_PROGRAM_HASH: Felt =
    felt!("0x43c5c4cc37c4614d2cf3a833379052c3a38cd18d688b617e2c720e8f941cb8");

/// Bootloader program hash. Used to run the layout bridge program in SHARP — the SHARP fact is
/// computed over (bootloader program hash, output), so the settlement contract needs to know it.
pub const BOOTLOADER_PROGRAM_HASH: Felt =
    felt!("0x5ab580b04e3532b6b18f81cfa654a05e29dd8e2352d88df1e765a84072db07");

/// The contract address that handles fact verification on Starknet mainnet.
///
/// Taken from <https://github.com/HerodotusDev/integrity/blob/main/deployed_contracts.md>.
const ATLANTIC_FACT_REGISTRY_MAINNET: Felt =
    felt!("0xcc63a1e8e7824642b89fa6baf996b8ed21fa4707be90ef7605570ca8e4f00b");

/// The contract address that handles fact verification on Starknet sepolia.
///
/// Taken from <https://github.com/HerodotusDev/integrity/blob/main/deployed_contracts.md>.
const ATLANTIC_FACT_REGISTRY_SEPOLIA: Felt =
    felt!("0x4ce7851f00b6c3289674841fd7a1b96b6fd41ed1edc248faccd672c26371b8c");

const CARTRIDGE_SN_MAINNET_PROVIDER: &str = "https://api.cartridge.gg/x/starknet/mainnet/rpc/v0_9";
const CARTRIDGE_SN_SEPOLIA_PROVIDER: &str = "https://api.cartridge.gg/x/starknet/sepolia/rpc/v0_9";

/// A JSON-RPC `Provider` for the settlement chain, with a stashed fact-registry address used by
/// `katana init`'s deploy path. The startup validator does not consult `fact_registry`.
#[derive(Debug, Clone)]
pub struct SettlementChainProvider {
    fact_registry: Felt,
    client: Arc<JsonRpcClient<HttpTransport>>,
    url: Url,
}

impl SettlementChainProvider {
    pub fn sn_mainnet() -> Self {
        let url = Url::parse(CARTRIDGE_SN_MAINNET_PROVIDER).expect("valid url");
        Self::new(url, ATLANTIC_FACT_REGISTRY_MAINNET)
    }

    pub fn sn_sepolia() -> Self {
        let url = Url::parse(CARTRIDGE_SN_SEPOLIA_PROVIDER).expect("valid url");
        Self::new(url, ATLANTIC_FACT_REGISTRY_SEPOLIA)
    }

    pub fn new(url: Url, fact_registry: Felt) -> Self {
        let client = Arc::new(JsonRpcClient::new(HttpTransport::new(url.clone())));
        Self { fact_registry, client, url }
    }

    pub fn set_fact_registry(&mut self, fact_registry: Felt) {
        self.fact_registry = fact_registry;
    }

    pub fn fact_registry(&self) -> Felt {
        self.fact_registry
    }

    pub fn url(&self) -> &Url {
        &self.url
    }
}

/// Errors surfaced when validating an on-chain Piltover settlement contract against the chain
/// spec.
#[derive(Error, Debug)]
pub enum SettlementValidationError {
    #[error(
        "settlement core contract not found at {address} on the settlement chain — the chain spec \
         points at an address that has no deployed contract"
    )]
    CoreContractNotFound { address: ContractAddress },

    #[error(
        "invalid program info: layout bridge program hash mismatch - expected {expected:#x}, got \
         {actual:#x}"
    )]
    InvalidLayoutBridgeProgramHash { expected: Felt, actual: Felt },

    #[error(
        "invalid program info: bootloader program hash mismatch - expected {expected:#x}, got \
         {actual:#x}"
    )]
    InvalidBootloaderProgramHash { expected: Felt, actual: Felt },

    #[error(
        "invalid program info: snos program hash mismatch - expected {expected:#x}, got \
         {actual:#x}"
    )]
    InvalidSnosProgramHash { expected: Felt, actual: Felt },

    #[error(
        "invalid program info: config hash mismatch - expected {expected:#x}, got {actual:#x}"
    )]
    InvalidConfigHash { expected: Felt, actual: Felt },

    #[error(
        "invalid program info: katana tee config hash mismatch - expected {expected:#x}, got \
         {actual:#x}"
    )]
    InvalidKatanaTeeConfigHash { expected: Felt, actual: Felt },

    #[error(
        "invalid program info: settlement-mode mismatch - expected {expected} variant, got \
         {actual}"
    )]
    InvalidProgramInfoVariant { expected: &'static str, actual: &'static str },

    #[error("failed to read program info from settlement contract: {0}")]
    Provider(#[from] ProviderError),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// Validates the on-chain Piltover core contract against the chain's expected program info.
///
/// In [`SettlementProofKind::ValidityProof`] mode, validates SNOS, layout-bridge, and bootloader
/// program hashes plus the SNOS config hash. In [`SettlementProofKind::Tee`] mode, only the SNOS
/// config hash is validated — TEE settlement does not depend on the Cairo program hashes.
pub async fn validate_starknet_settlement(
    chain_id: Felt,
    fee_token: Felt,
    contract: ContractAddress,
    provider: &SettlementChainProvider,
    proof_kind: SettlementProofKind,
) -> Result<(), SettlementValidationError> {
    let appchain = AppchainContractReader::new(contract.into(), provider);
    let on_chain = appchain
        .get_program_info()
        .call()
        .await
        .map_err(|e| map_program_info_error(&e, contract))?;

    check_program_info(&on_chain, chain_id, fee_token, proof_kind)
}

/// Maps an error from `AppchainContractReader::get_program_info().call()` into a
/// [`SettlementValidationError`]. Surfaces `ContractNotFound` distinctly so the operator can tell
/// "wrong address in chain spec" apart from generic RPC failures.
///
/// We accept any error type and walk its `source()` chain rather than matching cainome's error
/// directly. Piltover and this crate currently resolve to different cainome versions, so the
/// types aren't compatible — but `starknet::providers::ProviderError` is shared via the workspace
/// `starknet` dep, and that's what cainome wraps as the `source` of its `Provider` variant.
fn map_program_info_error<E>(err: &E, contract: ContractAddress) -> SettlementValidationError
where
    E: std::error::Error + Send + Sync + 'static,
{
    let mut cause: Option<&(dyn std::error::Error + 'static)> = Some(err);
    while let Some(e) = cause {
        if let Some(provider_err) = e.downcast_ref::<ProviderError>() {
            if matches!(provider_err, ProviderError::StarknetError(StarknetError::ContractNotFound))
            {
                return SettlementValidationError::CoreContractNotFound { address: contract };
            }
            break;
        }
        cause = e.source();
    }

    SettlementValidationError::Other(anyhow::anyhow!("{err}"))
}

/// Pure comparison between an on-chain [`ProgramInfo`] and the chain's expected values. Split out
/// from the network-bound [`validate_starknet_settlement`] so it can be unit-tested without a
/// running settlement node.
fn check_program_info(
    on_chain: &ProgramInfo,
    chain_id: Felt,
    fee_token: Felt,
    proof_kind: SettlementProofKind,
) -> Result<(), SettlementValidationError> {
    match (proof_kind, on_chain) {
        (SettlementProofKind::ValidityProof, ProgramInfo::StarknetOs(info)) => {
            let expected_config_hash = compute_starknet_os_config_hash(chain_id, fee_token);

            if info.snos_config_hash != expected_config_hash {
                return Err(SettlementValidationError::InvalidConfigHash {
                    expected: expected_config_hash,
                    actual: info.snos_config_hash,
                });
            }

            if info.snos_program_hash != SNOS_PROGRAM_HASH {
                return Err(SettlementValidationError::InvalidSnosProgramHash {
                    expected: SNOS_PROGRAM_HASH,
                    actual: info.snos_program_hash,
                });
            }

            if info.layout_bridge_program_hash != LAYOUT_BRIDGE_PROGRAM_HASH {
                return Err(SettlementValidationError::InvalidLayoutBridgeProgramHash {
                    expected: LAYOUT_BRIDGE_PROGRAM_HASH,
                    actual: info.layout_bridge_program_hash,
                });
            }

            if info.bootloader_program_hash != BOOTLOADER_PROGRAM_HASH {
                return Err(SettlementValidationError::InvalidBootloaderProgramHash {
                    expected: BOOTLOADER_PROGRAM_HASH,
                    actual: info.bootloader_program_hash,
                });
            }
        }

        (SettlementProofKind::Tee, ProgramInfo::KatanaTee(info)) => {
            let expected = compute_katana_tee_config_hash(chain_id, fee_token);
            if info.katana_tee_config_hash != expected {
                return Err(SettlementValidationError::InvalidKatanaTeeConfigHash {
                    expected,
                    actual: info.katana_tee_config_hash,
                });
            }
        }

        (SettlementProofKind::ValidityProof, ProgramInfo::KatanaTee(_)) => {
            return Err(SettlementValidationError::InvalidProgramInfoVariant {
                expected: "StarknetOs",
                actual: "KatanaTee",
            });
        }

        (SettlementProofKind::Tee, ProgramInfo::StarknetOs(_)) => {
            return Err(SettlementValidationError::InvalidProgramInfoVariant {
                expected: "KatanaTee",
                actual: "StarknetOs",
            });
        }
    }

    Ok(())
}

// https://github.com/starkware-libs/sequencer/blob/e13acc4c582352e777f5beae3476d157e6bdf4cf/crates/apollo_starknet_os_program/src/cairo/starkware/starknet/core/os/os_config/os_config.cairo#L10
pub fn compute_starknet_os_config_hash(chain_id: Felt, fee_token: Felt) -> Felt {
    const STARKNET_OS_CONFIG_VERSION: ShortString = ShortString::from_ascii("StarknetOsConfig3");
    compute_hash_on_elements(&[STARKNET_OS_CONFIG_VERSION.into(), chain_id, fee_token])
}

mod provider {
    use starknet::core::types::{
        BlockHashAndNumber, BlockId, BroadcastedDeclareTransaction,
        BroadcastedDeployAccountTransaction, BroadcastedInvokeTransaction, BroadcastedTransaction,
        ConfirmedBlockId, ContractClass, ContractStorageKeys, DeclareTransactionResult,
        DeployAccountTransactionResult, EventFilter, EventsPage, FeeEstimate, Felt, FunctionCall,
        Hash256, InvokeTransactionResult, MaybePreConfirmedBlockWithReceipts,
        MaybePreConfirmedBlockWithTxHashes, MaybePreConfirmedBlockWithTxs,
        MaybePreConfirmedStateUpdate, MessageFeeEstimate, MessageStatus, MsgFromL1,
        SimulatedTransaction, SimulationFlag, SimulationFlagForEstimateFee, StorageProof,
        SyncStatusType, Transaction, TransactionReceiptWithBlockInfo, TransactionStatus,
        TransactionTrace, TransactionTraceWithHash,
    };
    use starknet::providers::{Provider, ProviderError, ProviderRequestData, ProviderResponseData};

    #[async_trait::async_trait]
    impl Provider for super::SettlementChainProvider {
        async fn get_messages_status(
            &self,
            transaction_hash: Hash256,
        ) -> Result<Vec<MessageStatus>, ProviderError> {
            self.client.get_messages_status(transaction_hash).await
        }

        async fn get_storage_proof<B, H, A, K>(
            &self,
            block_id: B,
            class_hashes: H,
            contract_addresses: A,
            contracts_storage_keys: K,
        ) -> Result<StorageProof, ProviderError>
        where
            B: AsRef<ConfirmedBlockId> + Send + Sync,
            H: AsRef<[Felt]> + Send + Sync,
            A: AsRef<[Felt]> + Send + Sync,
            K: AsRef<[ContractStorageKeys]> + Send + Sync,
        {
            self.client
                .get_storage_proof(
                    block_id,
                    class_hashes,
                    contract_addresses,
                    contracts_storage_keys,
                )
                .await
        }

        async fn spec_version(&self) -> Result<String, ProviderError> {
            self.client.spec_version().await
        }

        async fn get_block_with_tx_hashes<B>(
            &self,
            block_id: B,
        ) -> Result<MaybePreConfirmedBlockWithTxHashes, ProviderError>
        where
            B: AsRef<BlockId> + Send + Sync,
        {
            self.client.get_block_with_tx_hashes(block_id).await
        }

        async fn get_block_with_txs<B>(
            &self,
            block_id: B,
        ) -> Result<MaybePreConfirmedBlockWithTxs, ProviderError>
        where
            B: AsRef<BlockId> + Send + Sync,
        {
            self.client.get_block_with_txs(block_id).await
        }

        async fn get_block_with_receipts<B>(
            &self,
            block_id: B,
        ) -> Result<MaybePreConfirmedBlockWithReceipts, ProviderError>
        where
            B: AsRef<BlockId> + Send + Sync,
        {
            self.client.get_block_with_receipts(block_id).await
        }

        async fn get_state_update<B>(
            &self,
            block_id: B,
        ) -> Result<MaybePreConfirmedStateUpdate, ProviderError>
        where
            B: AsRef<BlockId> + Send + Sync,
        {
            self.client.get_state_update(block_id).await
        }

        async fn get_storage_at<A, K, B>(
            &self,
            contract_address: A,
            key: K,
            block_id: B,
        ) -> Result<Felt, ProviderError>
        where
            A: AsRef<Felt> + Send + Sync,
            K: AsRef<Felt> + Send + Sync,
            B: AsRef<BlockId> + Send + Sync,
        {
            self.client.get_storage_at(contract_address, key, block_id).await
        }

        async fn get_transaction_status<H>(
            &self,
            transaction_hash: H,
        ) -> Result<TransactionStatus, ProviderError>
        where
            H: AsRef<Felt> + Send + Sync,
        {
            self.client.get_transaction_status(transaction_hash).await
        }

        async fn get_transaction_by_hash<H>(
            &self,
            transaction_hash: H,
        ) -> Result<Transaction, ProviderError>
        where
            H: AsRef<Felt> + Send + Sync,
        {
            self.client.get_transaction_by_hash(transaction_hash).await
        }

        async fn get_transaction_by_block_id_and_index<B>(
            &self,
            block_id: B,
            index: u64,
        ) -> Result<Transaction, ProviderError>
        where
            B: AsRef<BlockId> + Send + Sync,
        {
            self.client.get_transaction_by_block_id_and_index(block_id, index).await
        }

        async fn get_transaction_receipt<H>(
            &self,
            transaction_hash: H,
        ) -> Result<TransactionReceiptWithBlockInfo, ProviderError>
        where
            H: AsRef<Felt> + Send + Sync,
        {
            self.client.get_transaction_receipt(transaction_hash).await
        }

        async fn get_class<B, H>(
            &self,
            block_id: B,
            class_hash: H,
        ) -> Result<ContractClass, ProviderError>
        where
            B: AsRef<BlockId> + Send + Sync,
            H: AsRef<Felt> + Send + Sync,
        {
            self.client.get_class(block_id, class_hash).await
        }

        async fn get_class_hash_at<B, A>(
            &self,
            block_id: B,
            contract_address: A,
        ) -> Result<Felt, ProviderError>
        where
            B: AsRef<BlockId> + Send + Sync,
            A: AsRef<Felt> + Send + Sync,
        {
            self.client.get_class_hash_at(block_id, contract_address).await
        }

        async fn get_class_at<B, A>(
            &self,
            block_id: B,
            contract_address: A,
        ) -> Result<ContractClass, ProviderError>
        where
            B: AsRef<BlockId> + Send + Sync,
            A: AsRef<Felt> + Send + Sync,
        {
            self.client.get_class_at(block_id, contract_address).await
        }

        async fn get_block_transaction_count<B>(&self, block_id: B) -> Result<u64, ProviderError>
        where
            B: AsRef<BlockId> + Send + Sync,
        {
            self.client.get_block_transaction_count(block_id).await
        }

        async fn call<R, B>(&self, request: R, block_id: B) -> Result<Vec<Felt>, ProviderError>
        where
            R: AsRef<FunctionCall> + Send + Sync,
            B: AsRef<BlockId> + Send + Sync,
        {
            self.client.call(request, block_id).await
        }

        async fn estimate_fee<R, S, B>(
            &self,
            request: R,
            simulation_flags: S,
            block_id: B,
        ) -> Result<Vec<FeeEstimate>, ProviderError>
        where
            R: AsRef<[BroadcastedTransaction]> + Send + Sync,
            S: AsRef<[SimulationFlagForEstimateFee]> + Send + Sync,
            B: AsRef<BlockId> + Send + Sync,
        {
            self.client.estimate_fee(request, simulation_flags, block_id).await
        }

        async fn estimate_message_fee<M, B>(
            &self,
            message: M,
            block_id: B,
        ) -> Result<MessageFeeEstimate, ProviderError>
        where
            M: AsRef<MsgFromL1> + Send + Sync,
            B: AsRef<BlockId> + Send + Sync,
        {
            self.client.estimate_message_fee(message, block_id).await
        }

        async fn block_number(&self) -> Result<u64, ProviderError> {
            self.client.block_number().await
        }

        async fn block_hash_and_number(&self) -> Result<BlockHashAndNumber, ProviderError> {
            self.client.block_hash_and_number().await
        }

        async fn chain_id(&self) -> Result<Felt, ProviderError> {
            self.client.chain_id().await
        }

        async fn syncing(&self) -> Result<SyncStatusType, ProviderError> {
            self.client.syncing().await
        }

        async fn get_events(
            &self,
            filter: EventFilter,
            continuation_token: Option<String>,
            chunk_size: u64,
        ) -> Result<EventsPage, ProviderError> {
            self.client.get_events(filter, continuation_token, chunk_size).await
        }

        async fn get_nonce<B, A>(
            &self,
            block_id: B,
            contract_address: A,
        ) -> Result<Felt, ProviderError>
        where
            B: AsRef<BlockId> + Send + Sync,
            A: AsRef<Felt> + Send + Sync,
        {
            self.client.get_nonce(block_id, contract_address).await
        }

        async fn add_invoke_transaction<I>(
            &self,
            invoke_transaction: I,
        ) -> Result<InvokeTransactionResult, ProviderError>
        where
            I: AsRef<BroadcastedInvokeTransaction> + Send + Sync,
        {
            self.client.add_invoke_transaction(invoke_transaction).await
        }

        async fn add_declare_transaction<D>(
            &self,
            declare_transaction: D,
        ) -> Result<DeclareTransactionResult, ProviderError>
        where
            D: AsRef<BroadcastedDeclareTransaction> + Send + Sync,
        {
            self.client.add_declare_transaction(declare_transaction).await
        }

        async fn add_deploy_account_transaction<D>(
            &self,
            deploy_account_transaction: D,
        ) -> Result<DeployAccountTransactionResult, ProviderError>
        where
            D: AsRef<BroadcastedDeployAccountTransaction> + Send + Sync,
        {
            self.client.add_deploy_account_transaction(deploy_account_transaction).await
        }

        async fn trace_transaction<H>(
            &self,
            transaction_hash: H,
        ) -> Result<TransactionTrace, ProviderError>
        where
            H: AsRef<Felt> + Send + Sync,
        {
            self.client.trace_transaction(transaction_hash).await
        }

        async fn simulate_transactions<B, T, S>(
            &self,
            block_id: B,
            transactions: T,
            simulation_flags: S,
        ) -> Result<Vec<SimulatedTransaction>, ProviderError>
        where
            B: AsRef<BlockId> + Send + Sync,
            T: AsRef<[BroadcastedTransaction]> + Send + Sync,
            S: AsRef<[SimulationFlag]> + Send + Sync,
        {
            self.client.simulate_transactions(block_id, transactions, simulation_flags).await
        }

        async fn trace_block_transactions<B>(
            &self,
            block_id: B,
        ) -> Result<Vec<TransactionTraceWithHash>, ProviderError>
        where
            B: AsRef<ConfirmedBlockId> + Send + Sync,
        {
            self.client.trace_block_transactions(block_id).await
        }

        async fn batch_requests<R>(
            &self,
            requests: R,
        ) -> Result<Vec<ProviderResponseData>, ProviderError>
        where
            R: AsRef<[ProviderRequestData]> + Send + Sync,
        {
            self.client.batch_requests(requests).await
        }
    }
}

#[cfg(test)]
mod tests {
    use katana_primitives::{felt, ContractAddress, Felt};
    use piltover::{KatanaTeeProgramInfo, ProgramInfo, StarknetOsProgramInfo};
    use starknet::core::chain_id::{MAINNET, SEPOLIA};
    use starknet::core::types::StarknetError;
    use starknet::providers::ProviderError;

    use super::{
        check_program_info, compute_starknet_os_config_hash, map_program_info_error,
        SettlementValidationError, BOOTLOADER_PROGRAM_HASH, LAYOUT_BRIDGE_PROGRAM_HASH,
        SNOS_PROGRAM_HASH,
    };
    use crate::tee::compute_katana_tee_config_hash;
    use crate::SettlementProofKind;

    const STRK_FEE_TOKEN: Felt =
        felt!("0x04718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d");

    const TEST_CHAIN_ID: Felt = felt!("0x4b4154414e41");
    const TEST_FEE_TOKEN: Felt = felt!("0xfee");

    // Source:
    //
    // - https://github.com/starkware-libs/cairo-lang/blob/v0.14.0.1/src/starkware/starknet/core/os/os_config/os_config_hash.json
    // - https://docs.starknet.io/tools/important-addresses/#fee_tokens
    #[rstest::rstest]
    #[case::mainnet(felt!("0x70c7b342f93155315d1cb2da7a4e13a3c2430f51fb5696c1b224c3da5508dfb"), MAINNET)]
    #[case::testnet(felt!("0x1b9900f77ff5923183a7795fcfbb54ed76917bc1ddd4160cc77fa96e36cf8c5"), SEPOLIA)]
    fn calculate_config_hash(#[case] config_hash: Felt, #[case] chain: Felt) {
        let computed = compute_starknet_os_config_hash(chain, STRK_FEE_TOKEN);
        assert_eq!(computed, config_hash);
    }

    fn well_formed_starknet_os() -> ProgramInfo {
        ProgramInfo::StarknetOs(StarknetOsProgramInfo {
            snos_config_hash: compute_starknet_os_config_hash(TEST_CHAIN_ID, TEST_FEE_TOKEN),
            snos_program_hash: SNOS_PROGRAM_HASH,
            bootloader_program_hash: BOOTLOADER_PROGRAM_HASH,
            layout_bridge_program_hash: LAYOUT_BRIDGE_PROGRAM_HASH,
        })
    }

    fn well_formed_katana_tee() -> ProgramInfo {
        ProgramInfo::KatanaTee(KatanaTeeProgramInfo {
            katana_tee_config_hash: compute_katana_tee_config_hash(TEST_CHAIN_ID, TEST_FEE_TOKEN),
        })
    }

    #[test]
    fn validity_proof_accepts_correct_starknet_os_info() {
        let info = well_formed_starknet_os();
        check_program_info(
            &info,
            TEST_CHAIN_ID,
            TEST_FEE_TOKEN,
            SettlementProofKind::ValidityProof,
        )
        .expect("well-formed StarknetOs info must validate");
    }

    #[test]
    fn tee_accepts_correct_katana_tee_info() {
        let info = well_formed_katana_tee();
        check_program_info(&info, TEST_CHAIN_ID, TEST_FEE_TOKEN, SettlementProofKind::Tee)
            .expect("well-formed KatanaTee info must validate");
    }

    #[test]
    fn cross_mode_tee_chain_with_starknet_os_contract_rejected() {
        let info = well_formed_starknet_os();
        let err =
            check_program_info(&info, TEST_CHAIN_ID, TEST_FEE_TOKEN, SettlementProofKind::Tee)
                .expect_err("TEE chain pointing at StarknetOs contract must be rejected");
        assert!(matches!(
            err,
            SettlementValidationError::InvalidProgramInfoVariant {
                expected: "KatanaTee",
                actual: "StarknetOs"
            }
        ));
    }

    #[test]
    fn cross_mode_validity_proof_chain_with_tee_contract_rejected() {
        let info = well_formed_katana_tee();
        let err = check_program_info(
            &info,
            TEST_CHAIN_ID,
            TEST_FEE_TOKEN,
            SettlementProofKind::ValidityProof,
        )
        .expect_err("ZK chain pointing at KatanaTee contract must be rejected");
        assert!(matches!(
            err,
            SettlementValidationError::InvalidProgramInfoVariant {
                expected: "StarknetOs",
                actual: "KatanaTee"
            }
        ));
    }

    #[test]
    fn validity_proof_rejects_snos_program_hash_mismatch() {
        let info = ProgramInfo::StarknetOs(StarknetOsProgramInfo {
            snos_config_hash: compute_starknet_os_config_hash(TEST_CHAIN_ID, TEST_FEE_TOKEN),
            snos_program_hash: Felt::from(0xbadu32),
            bootloader_program_hash: BOOTLOADER_PROGRAM_HASH,
            layout_bridge_program_hash: LAYOUT_BRIDGE_PROGRAM_HASH,
        });
        let err = check_program_info(
            &info,
            TEST_CHAIN_ID,
            TEST_FEE_TOKEN,
            SettlementProofKind::ValidityProof,
        )
        .expect_err("must reject SNOS program hash mismatch");
        assert!(matches!(err, SettlementValidationError::InvalidSnosProgramHash { .. }));
    }

    #[test]
    fn validity_proof_rejects_layout_bridge_mismatch() {
        let info = ProgramInfo::StarknetOs(StarknetOsProgramInfo {
            snos_config_hash: compute_starknet_os_config_hash(TEST_CHAIN_ID, TEST_FEE_TOKEN),
            snos_program_hash: SNOS_PROGRAM_HASH,
            bootloader_program_hash: BOOTLOADER_PROGRAM_HASH,
            layout_bridge_program_hash: Felt::from(0xbadu32),
        });
        let err = check_program_info(
            &info,
            TEST_CHAIN_ID,
            TEST_FEE_TOKEN,
            SettlementProofKind::ValidityProof,
        )
        .expect_err("must reject layout bridge mismatch");
        assert!(matches!(err, SettlementValidationError::InvalidLayoutBridgeProgramHash { .. }));
    }

    #[test]
    fn validity_proof_rejects_bootloader_mismatch() {
        let info = ProgramInfo::StarknetOs(StarknetOsProgramInfo {
            snos_config_hash: compute_starknet_os_config_hash(TEST_CHAIN_ID, TEST_FEE_TOKEN),
            snos_program_hash: SNOS_PROGRAM_HASH,
            bootloader_program_hash: Felt::from(0xbadu32),
            layout_bridge_program_hash: LAYOUT_BRIDGE_PROGRAM_HASH,
        });
        let err = check_program_info(
            &info,
            TEST_CHAIN_ID,
            TEST_FEE_TOKEN,
            SettlementProofKind::ValidityProof,
        )
        .expect_err("must reject bootloader mismatch");
        assert!(matches!(err, SettlementValidationError::InvalidBootloaderProgramHash { .. }));
    }

    #[test]
    fn validity_proof_rejects_config_hash_mismatch() {
        let info = ProgramInfo::StarknetOs(StarknetOsProgramInfo {
            snos_config_hash: Felt::from(0xbadu32),
            snos_program_hash: SNOS_PROGRAM_HASH,
            bootloader_program_hash: BOOTLOADER_PROGRAM_HASH,
            layout_bridge_program_hash: LAYOUT_BRIDGE_PROGRAM_HASH,
        });
        let err = check_program_info(
            &info,
            TEST_CHAIN_ID,
            TEST_FEE_TOKEN,
            SettlementProofKind::ValidityProof,
        )
        .expect_err("must reject SNOS config hash mismatch");
        assert!(matches!(err, SettlementValidationError::InvalidConfigHash { .. }));
    }

    /// Stand-in for cainome's error: a `Display`-able wrapper whose `source()` returns the inner
    /// `ProviderError`. The real cainome `Error::Provider(ProviderError)` shapes its source the
    /// same way (via `#[from]`), so this exercises the same path `map_program_info_error` walks
    /// in production.
    #[derive(Debug)]
    struct CainomeLikeError(ProviderError);

    impl std::fmt::Display for CainomeLikeError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "provider error: {}", self.0)
        }
    }

    impl std::error::Error for CainomeLikeError {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            Some(&self.0)
        }
    }

    #[test]
    fn maps_contract_not_found_to_distinct_error() {
        let address = ContractAddress::from(felt!("0xdeadbeef"));
        let err = map_program_info_error(
            &CainomeLikeError(ProviderError::StarknetError(StarknetError::ContractNotFound)),
            address,
        );
        match err {
            SettlementValidationError::CoreContractNotFound { address: a } => {
                assert_eq!(a, address);
            }
            other => panic!("expected CoreContractNotFound, got {other:?}"),
        }
    }

    #[test]
    fn maps_other_provider_errors_to_other() {
        let address = ContractAddress::from(felt!("0xdeadbeef"));
        let err = map_program_info_error(&CainomeLikeError(ProviderError::RateLimited), address);
        assert!(matches!(err, SettlementValidationError::Other(_)));
    }

    #[test]
    fn tee_rejects_katana_tee_config_hash_mismatch() {
        let info = ProgramInfo::KatanaTee(KatanaTeeProgramInfo {
            katana_tee_config_hash: Felt::from(0xdeadu32),
        });
        let err =
            check_program_info(&info, TEST_CHAIN_ID, TEST_FEE_TOKEN, SettlementProofKind::Tee)
                .expect_err("must reject KatanaTee config hash mismatch");
        assert!(matches!(err, SettlementValidationError::InvalidKatanaTeeConfigHash { .. }));
    }
}
