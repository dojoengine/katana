use crate::data_availability::DataAvailabilityPointer;
use anyhow::Result;
use starknet::core::types::StateUpdate;
use std::future::Future;

mod in_memory;
pub use in_memory::InMemoryStorageBackend;

mod sql_lite;
pub use sql_lite::SqliteDb;

pub trait StorageBackend {
    fn get_chain_head(&self) -> impl Future<Output = ChainHead>;

    fn set_chain_head(&mut self, block: BlockWithDa) -> impl Future<Output = ()> + Send;
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum ChainHead {
    Genesis,
    Block(BlockWithDa),
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct BlockWithDa {
    pub height: u64,
    pub da_pointer: DataAvailabilityPointer,
}

pub enum Step {
    Snos,
    Bridge,
}
pub enum Query {
    SnosProof,
    BridgeProof,
    BridgeTrace,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockStatus {
    Mined,
    SnosPieGenerated,
    SnosProofSubmitted,
    SnosProofGenerated,
    BridgePieSubmitted,
    BridgePieGenerated,
    BridgeProofSubmitted,
    BridgeProofGenerated,
    VerifiedProof,
    Settled,
    Failed,
}

impl std::fmt::Display for BlockStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BlockStatus::Mined => write!(f, "mined"),
            BlockStatus::SnosPieGenerated => write!(f, "snos_pie_generated"),
            BlockStatus::SnosProofSubmitted => write!(f, "snos_proof_submitted"),
            BlockStatus::SnosProofGenerated => write!(f, "snos_proof_generated"),
            BlockStatus::BridgePieSubmitted => write!(f, "bridge_pie_submitted"),
            BlockStatus::BridgePieGenerated => write!(f, "bridge_pie_generated"),
            BlockStatus::BridgeProofSubmitted => write!(f, "bridge_proof_submitted"),
            BlockStatus::BridgeProofGenerated => write!(f, "bridge_proof_generated"),
            BlockStatus::VerifiedProof => write!(f, "verified_proof"),
            BlockStatus::Settled => write!(f, "settled"),
            BlockStatus::Failed => write!(f, "failed"),
        }
    }
}

impl From<&str> for BlockStatus {
    fn from(s: &str) -> Self {
        match s {
            "mined" => BlockStatus::Mined,
            "snos_pie_generated" => BlockStatus::SnosPieGenerated,
            "snos_proof_submitted" => BlockStatus::SnosProofSubmitted,
            "snos_proof_generated" => BlockStatus::SnosProofGenerated,
            "bridge_pie_submitted" => BlockStatus::BridgePieSubmitted,
            "bridge_pie_generated" => BlockStatus::BridgePieGenerated,
            "bridge_proof_submitted" => BlockStatus::BridgeProofSubmitted,
            "bridge_proof_generated" => BlockStatus::BridgeProofGenerated,
            "verified_proof" => BlockStatus::VerifiedProof,
            "failed" => BlockStatus::Failed,
            "settled" => BlockStatus::Settled,
            _ => panic!("Invalid block status"),
        }
    }
}

pub trait PersistantStorage {
    fn initialize_block(&self, block_number: u32) -> impl Future<Output = Result<()>> + Send;

    fn remove_block(&self, block_number: u32) -> impl Future<Output = Result<()>> + Send;

    fn add_pie(
        &self,
        block_number: u32,
        pie: Vec<u8>,
        step: Step,
    ) -> impl Future<Output = Result<()>> + Send;

    fn get_pie(
        &self,
        block_number: u32,
        step: Step,
    ) -> impl Future<Output = Result<Vec<u8>>> + Send;

    fn add_proof(
        &self,
        block_number: u32,
        proof: Vec<u8>,
        step: Step,
    ) -> impl Future<Output = Result<()>> + Send;

    fn get_proof(
        &self,
        block_number: u32,
        step: Step,
    ) -> impl Future<Output = Result<Vec<u8>>> + Send;

    fn add_query_id(
        &self,
        block_number: u32,
        query_id: String,
        query_type: Query,
    ) -> impl Future<Output = Result<()>> + Send;

    fn get_query_id(
        &self,
        block_number: u32,
        query_type: Query,
    ) -> impl Future<Output = Result<String>> + Send;

    fn set_status(
        &self,
        block_number: u32,
        status: String,
    ) -> impl Future<Output = Result<()>> + Send;

    fn get_status(&self, block_number: u32) -> impl Future<Output = Result<BlockStatus>> + Send;

    fn get_first_db_block(&self) -> impl Future<Output = Result<u32>> + Send;

    fn add_failed_block(
        &self,
        block_number: u32,
        failure_reason: String,
    ) -> impl Future<Output = Result<()>> + Send;

    fn get_failed_blocks(&self) -> impl Future<Output = Result<Vec<(u32, String)>>> + Send;

    fn mark_failed_blocks_as_handled(
        &self,
        block_id: &[u32],
    ) -> impl Future<Output = Result<()>> + Send;

    fn add_state_update(
        &self,
        block_number: u32,
        state_update: StateUpdate,
    ) -> impl Future<Output = Result<()>> + Send;

    fn get_state_update(
        &self,
        block_number: u32,
    ) -> impl Future<Output = Result<StateUpdate>> + Send;
}
