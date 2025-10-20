use katana_db::abstraction::Database;

use crate::providers::db::DbProvider;

#[auto_impl::auto_impl(&, Box)]
pub trait ProviderFactory: Send + Sync {
    type Provider;
    type ProviderMut;

    fn provider(&self) -> Self::Provider;
    fn provider_mut(&self) -> Self::ProviderMut;
}

pub struct DbProviderFactory<Db: Database> {
    database: Db,
}

impl<Db: Database> ProviderFactory for DbProviderFactory<Db> {
    type Provider = DbProvider<Db::Tx>;
    type ProviderMut = DbProvider<Db::TxMut>;

    fn provider(&self) -> Self::Provider {
        DbProvider::new(self.database.tx().unwrap())
    }

    fn provider_mut(&self) -> Self::ProviderMut {
        DbProvider::new(self.database.tx_mut().unwrap())
    }
}
