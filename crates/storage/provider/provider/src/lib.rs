use std::fmt::Debug;

use katana_db::abstraction::Database;
use katana_fork::Backend;
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

use crate::providers::db::DbProvider;
use crate::providers::fork::{ForkedDb, ForkedProvider};

#[auto_impl::auto_impl(&, Box, Arc)]
pub trait ProviderFactory: Send + Sync + Debug {
    type Provider;
    type ProviderMut;

    fn provider(&self) -> Self::Provider;
    fn provider_mut(&self) -> Self::ProviderMut;
}

#[derive(Clone)]
pub struct DbProviderFactory<Db: Database = katana_db::Db> {
    db: Db,
}

impl<Db: Database> Debug for DbProviderFactory<Db> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DbProviderFactory").finish_non_exhaustive()
    }
}

impl<Db: Database> DbProviderFactory<Db> {
    pub fn new(db: Db) -> Self {
        Self { db }
    }

    pub fn inner(&self) -> &Db {
        &self.db
    }
}

impl DbProviderFactory<katana_db::Db> {
    pub fn new_in_memory() -> DbProviderFactory<katana_db::Db> {
        Self::new(katana_db::Db::in_memory().unwrap())
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

#[derive(Clone)]
pub struct ForkProviderFactory {
    backend: Backend,
    block_id: BlockNumber,
    local_factory: DbProviderFactory<katana_db::Db>,
    fork_factory: DbProviderFactory<katana_db::Db>,
}

impl Debug for ForkProviderFactory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ForkProviderFactory").finish_non_exhaustive()
    }
}

impl ForkProviderFactory {
    pub fn new(db: katana_db::Db, block_id: BlockNumber, starknet_client: StarknetClient) -> Self {
        let backend = Backend::new(starknet_client).expect("failed to create backend");

        let local_factory = DbProviderFactory::new(db);
        let fork_factory = DbProviderFactory::new_in_memory();

        Self { local_factory, fork_factory, backend, block_id }
    }

    pub fn block(&self) -> BlockNumber {
        self.block_id
    }
}

impl ProviderFactory for ForkProviderFactory {
    type Provider =
        ForkedProvider<<katana_db::Db as Database>::Tx, <katana_db::Db as Database>::TxMut>;

    type ProviderMut =
        ForkedProvider<<katana_db::Db as Database>::TxMut, <katana_db::Db as Database>::TxMut>;

    fn provider(&self) -> Self::Provider {
        ForkedProvider::new(
            self.local_factory.provider(),
            ForkedDb::new(self.backend.clone(), self.block_id, self.fork_factory.provider_mut()),
        )
    }

    fn provider_mut(&self) -> Self::ProviderMut {
        ForkedProvider::new(
            self.local_factory.provider_mut(),
            ForkedDb::new(self.backend.clone(), self.block_id, self.fork_factory.provider_mut()),
        )
    }
}
