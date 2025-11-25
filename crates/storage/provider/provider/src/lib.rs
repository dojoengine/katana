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
pub trait ProviderFactory: Send + Sync + Debug + 'static {
    type Provider;
    type ProviderMut: MutableProvider;

    fn provider(&self) -> Self::Provider;
    fn provider_mut(&self) -> Self::ProviderMut;
}

#[auto_impl::auto_impl(Box)]
pub trait MutableProvider: Sized + Send + Sync + 'static {
    fn commit(self) -> ProviderResult<()>;
}

#[derive(Clone)]
pub struct DbProviderFactory {
    db: katana_db::Db,
}

impl Debug for DbProviderFactory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DbProviderFactory").finish_non_exhaustive()
    }
}

impl DbProviderFactory {
    pub fn new(db: katana_db::Db) -> Self {
        Self { db }
    }

    pub fn new_in_memory() -> Self {
        Self::new(katana_db::Db::in_memory().unwrap())
    }

    pub fn inner(&self) -> &katana_db::Db {
        &self.db
    }
}

impl DbProviderFactory {}

impl ProviderFactory for DbProviderFactory {
    type Provider = DbProvider<<katana_db::Db as Database>::Tx>;
    type ProviderMut = DbProvider<<katana_db::Db as Database>::TxMut>;

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
    fork_factory: DbProviderFactory,
    local_factory: DbProviderFactory,
}

impl ForkProviderFactory {
    pub fn new(db: katana_db::Db, block_id: BlockNumber, starknet_client: StarknetClient) -> Self {
        let backend = Backend::new(starknet_client).expect("failed to create backend");

        let local_factory = DbProviderFactory::new(db);
        let fork_factory = DbProviderFactory::new_in_memory();

        Self { local_factory, fork_factory, backend, block_id }
    }

    pub fn new_in_memory(block_id: BlockNumber, starknet_client: StarknetClient) -> Self {
        Self::new(katana_db::Db::in_memory().unwrap(), block_id, starknet_client)
    }

    pub fn block(&self) -> BlockNumber {
        self.block_id
    }
}

impl Debug for ForkProviderFactory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ForkProviderFactory").finish_non_exhaustive()
    }
}

impl ProviderFactory for ForkProviderFactory {
    type Provider = ForkedProvider<<katana_db::Db as Database>::Tx>;

    type ProviderMut = ForkedProvider<<katana_db::Db as Database>::TxMut>;

    fn provider(&self) -> Self::Provider {
        ForkedProvider::new(
            self.local_factory.provider(),
            ForkedDb::new(self.backend.clone(), self.block_id, self.fork_factory.clone()),
        )
    }

    fn provider_mut(&self) -> Self::ProviderMut {
        ForkedProvider::new(
            self.local_factory.provider_mut(),
            ForkedDb::new(self.backend.clone(), self.block_id, self.fork_factory.clone()),
        )
    }
}
