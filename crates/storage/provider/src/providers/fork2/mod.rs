use std::sync::Arc;

use katana_db::abstraction::Database;
use katana_db::mdbx::DbEnv;
use katana_fork::{Backend, BackendClient};
use katana_primitives::block::BlockHashOrNumber;
use starknet::providers::JsonRpcClient;
use starknet::providers::jsonrpc::HttpTransport;

use super::db::{self, DbProvider};

mod state;

#[derive(Debug)]
pub struct ForkedProvider<Db: Database = DbEnv> {
    provider: DbProvider<Db>,
    backend: BackendClient,
}

impl<Db: Database> ForkedProvider<Db> {
    pub fn new(
        db: Db,
        block_id: BlockHashOrNumber,
        provider: Arc<JsonRpcClient<HttpTransport>>,
    ) -> Self {
        let backend = Backend::new(provider, block_id).expect("failed to create backend");
        Self { provider: DbProvider::new(db), backend }
    }

    pub fn db(&self) -> &Db {
        &self.provider.0
    }

    pub fn backend(&self) -> &BackendClient {
        &self.backend
    }
}
