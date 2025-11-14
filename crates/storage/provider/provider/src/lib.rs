use katana_db::abstraction::Database;
use katana_fork::{Backend, BackendClient};
use katana_primitives::block::BlockNumber;
pub use katana_provider_api::{ProviderError, ProviderResult};
use katana_rpc_client::starknet::Client as StarknetClient;

// Re-export the API module
pub mod api {
    pub use katana_provider_api::*;
}

pub mod providers;
#[cfg(any(test, feature = "test-utils"))]
pub mod test_utils;

use crate::providers::{db::DbProvider, fork::ForkedProvider};

pub trait ProviderFactory {
    type Provider;
    type ProviderMut;

    fn provider(&self) -> Self::Provider;
    fn provider_mut(&self) -> Self::ProviderMut;
}

#[derive(Debug)]
pub struct DbProviderFactory<Db: Database = katana_db::Db> {
    db: Db,
}

impl<Db: Database> DbProviderFactory<Db> {
    pub fn new(db: Db) -> Self {
        Self { db }
    }
}

impl<Db: Database> ProviderFactory for DbProviderFactory<Db> {
    type Provider = DbProvider<Db::Tx>;
    type ProviderMut = DbProvider<Db::TxMut>;

    fn provider(&self) -> Self::Provider {
        DbProvider::new(self.db.tx().unwrap())
    }

    fn provider_mut(&self) -> Self::ProviderMut {
        DbProvider::new(self.db.tx_mut().unwrap())
    }
}

#[derive(Debug)]
pub struct ForkProviderFactory<Db: Database = katana_db::Db> {
    base_factory: DbProviderFactory<Db>,
    backend: BackendClient,
    block_id: BlockNumber,
}

impl<Db: Database> ForkProviderFactory<Db> {
    pub fn new(db: Db, block_id: BlockNumber, provider: StarknetClient) -> Self {
        let base_factory = DbProviderFactory::new(db);
        let backend = Backend::new(provider).expect("failed to create backend");
        Self { base_factory, backend, block_id }
    }
}

impl<Db: Database> ProviderFactory for ForkProviderFactory<Db> {
    type Provider = ForkedProvider<Db::Tx>;
    type ProviderMut = ForkedProvider<Db::TxMut>;

    fn provider(&self) -> Self::Provider {
        ForkedProvider::new(self.block_id, self.base_factory.provider(), self.backend.clone())
    }

    fn provider_mut(&self) -> Self::ProviderMut {
        ForkedProvider::new(self.block_id, self.base_factory.provider_mut(), self.backend.clone())
    }
}
