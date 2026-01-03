//! Transaction wrapper for libmdbx-sys.

use std::str::FromStr;
use std::sync::Arc;

use libmdbx::ffi::DBI;
use libmdbx::{TransactionKind, WriteFlags, RW};
use parking_lot::RwLock;
use tracing::error;

use super::cursor::Cursor;
use super::metrics::{DbMetrics, OpTimer};
use super::stats::TableStat;
use crate::abstraction::{DbTx, DbTxMut};
use crate::codecs::{Compress, Encode};
use crate::error::DatabaseError;
use crate::tables::{DupSort, Table, Tables, NUM_TABLES};
use crate::utils::decode_one;

/// Alias for read-only transaction.
pub type TxRO = Tx<libmdbx::RO>;
/// Alias for read-write transaction.
pub type TxRW = Tx<libmdbx::RW>;

/// Database transaction.
///
/// Wrapper for a `libmdbx` transaction.
pub struct Tx<K: TransactionKind> {
    /// Libmdbx-sys transaction.
    pub(super) inner: libmdbx::Transaction<K>,
    /// Database table handle cache.
    db_handles: Arc<RwLock<[Option<DBI>; NUM_TABLES]>>,
    /// Metrics for tracking operations.
    metrics: DbMetrics,
}

impl<K: TransactionKind> std::fmt::Debug for Tx<K> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tx").field("id", &self.inner.id()).finish_non_exhaustive()
    }
}

impl<K: TransactionKind> Tx<K> {
    /// Creates new `Tx` object with a `RO` or `RW` transaction.
    pub fn new(inner: libmdbx::Transaction<K>, metrics: DbMetrics) -> Self {
        Self { inner, db_handles: Arc::new(RwLock::new([None; NUM_TABLES])), metrics }
    }

    pub fn get_dbi<T: Table>(&self) -> Result<DBI, DatabaseError> {
        let mut handles = self.db_handles.write();
        let table = Tables::from_str(T::NAME).expect("requested table should be part of `Tables`.");

        let dbi_handle = handles.get_mut(table as usize).expect("should exist");
        if dbi_handle.is_none() {
            *dbi_handle =
                Some(self.inner.open_db(Some(T::NAME)).map_err(DatabaseError::OpenDb)?.dbi());
        }

        Ok(dbi_handle.expect("is some; qed"))
    }

    /// Retrieves statistics for a specific table.
    pub fn stat<T: Table>(&self) -> Result<TableStat, DatabaseError> {
        let dbi = self.get_dbi::<T>()?;
        let stat = self.inner.db_stat_with_dbi(dbi).map_err(DatabaseError::Stat)?;
        Ok(TableStat::new(stat))
    }
}

impl<K: TransactionKind> Clone for Tx<K> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            metrics: self.metrics.clone(),
            db_handles: self.db_handles.clone(),
        }
    }
}

impl<K: TransactionKind> DbTx for Tx<K> {
    type Cursor<T: Table> = Cursor<K, T>;
    type DupCursor<T: DupSort> = Self::Cursor<T>;

    fn cursor<T: Table>(&self) -> Result<Cursor<K, T>, DatabaseError> {
        self.inner
            .cursor_with_dbi(self.get_dbi::<T>()?)
            .map(Cursor::new)
            .map_err(DatabaseError::CreateCursor)
    }

    fn cursor_dup<T: DupSort>(&self) -> Result<Cursor<K, T>, DatabaseError> {
        self.inner
            .cursor_with_dbi(self.get_dbi::<T>()?)
            .map(Cursor::new)
            .map_err(DatabaseError::CreateCursor)
    }

    #[tracing::instrument(level = "trace", name = "db_get", skip_all, fields(table = T::NAME, txn_id = self.inner.id()))]
    fn get<T: Table>(&self, key: T::Key) -> Result<Option<<T as Table>::Value>, DatabaseError> {
        let timer = OpTimer::new();
        let key = Encode::encode(key);
        let result = self
            .inner
            .get(self.get_dbi::<T>()?, key.as_ref())
            .map_err(DatabaseError::Read)?
            .map(decode_one::<T>)
            .transpose();

        let found = result.as_ref().map(|r| r.is_some()).unwrap_or(false);
        self.metrics.record_get(timer.elapsed(), found);
        result
    }

    fn entries<T: Table>(&self) -> Result<usize, DatabaseError> {
        self.inner
            .db_stat_with_dbi(self.get_dbi::<T>()?)
            .map(|stat| stat.entries())
            .map_err(DatabaseError::Stat)
    }

    #[tracing::instrument(level = "trace", name = "db_commit", skip_all, fields(txn_id = self.inner.id()))]
    fn commit(self) -> Result<bool, DatabaseError> {
        let timer = OpTimer::new();
        let result = self.inner.commit().map_err(DatabaseError::Commit).inspect_err(|error| {
            error!(%error, "Commit failed");
        });

        let success = result.is_ok();
        self.metrics.record_tx_commit(timer.elapsed(), success);
        result
    }

    fn abort(self) {
        self.metrics.record_tx_abort();
        drop(self.inner)
    }
}

impl DbTxMut for Tx<RW> {
    type Cursor<T: Table> = Cursor<RW, T>;
    type DupCursor<T: DupSort> = <Self as DbTxMut>::Cursor<T>;

    fn cursor_mut<T: Table>(&self) -> Result<<Self as DbTxMut>::Cursor<T>, DatabaseError> {
        DbTx::cursor(self)
    }

    fn cursor_dup_mut<T: DupSort>(&self) -> Result<<Self as DbTxMut>::DupCursor<T>, DatabaseError> {
        self.inner
            .cursor_with_dbi(self.get_dbi::<T>()?)
            .map(Cursor::new)
            .map_err(DatabaseError::CreateCursor)
    }

    #[tracing::instrument(level = "trace", name = "db_put", skip_all, fields(table = T::NAME, txn_id = self.inner.id()))]
    fn put<T: Table>(&self, key: T::Key, value: T::Value) -> Result<(), DatabaseError> {
        let timer = OpTimer::new();
        let key = key.encode();
        let value = value.compress()?;
        let result = self.inner.put(self.get_dbi::<T>()?, &key, value, WriteFlags::UPSERT).map_err(
            |error| DatabaseError::Write { error, table: T::NAME, key: Box::from(key.as_ref()) },
        );

        self.metrics.record_put(timer.elapsed());
        result
    }

    #[tracing::instrument(level = "trace", name = "db_delete", skip_all, fields(table = T::NAME, txn_id = self.inner.id()))]
    fn delete<T: Table>(
        &self,
        key: T::Key,
        value: Option<T::Value>,
    ) -> Result<bool, DatabaseError> {
        let timer = OpTimer::new();
        let encoded_key = key.encode();
        let value = match value {
            Some(v) => Some(v.compress()?),
            None => None,
        };
        let value_ref = value.as_ref().map(|v| v.as_ref());
        let result = self
            .inner
            .del(self.get_dbi::<T>()?, encoded_key, value_ref)
            .map_err(DatabaseError::Delete);

        let deleted = result.as_ref().map(|&d| d).unwrap_or(false);
        self.metrics.record_delete(timer.elapsed(), deleted);
        result
    }

    #[tracing::instrument(level = "trace", name = "db_clear", skip_all, fields(table = T::NAME, txn_id = self.inner.id()))]
    fn clear<T: Table>(&self) -> Result<(), DatabaseError> {
        let timer = OpTimer::new();
        let result = self.inner.clear_db(self.get_dbi::<T>()?).map_err(DatabaseError::Clear);
        self.metrics.record_clear(timer.elapsed());
        result
    }
}
