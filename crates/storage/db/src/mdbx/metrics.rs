//! Metrics collection for database operations.
//!
//! This module provides metrics tracking for:
//! - Transaction lifecycle (creation, commit, abort)
//! - Database operations (get, put, delete, clear)
//! - Operation timing and success rates

use std::sync::Arc;
use std::time::Instant;

use katana_metrics::metrics::{Counter, Histogram};
use katana_metrics::Metrics;

/// Metrics for database operations.
#[derive(Clone, Debug)]
pub struct DbMetrics {
    inner: Arc<DbMetricsInner>,
}

impl Default for DbMetrics {
    fn default() -> Self {
        Self::new()
    }
}

impl DbMetrics {
    /// Creates a new instance of database metrics.
    pub fn new() -> Self {
        // database tables metrics
        ::metrics::describe_gauge!(
            "db.table_size",
            ::metrics::Unit::Bytes,
            "Total size of the table"
        );
        ::metrics::describe_gauge!(
            "db.table_pages",
            ::metrics::Unit::Count,
            "Number of pages in the table"
        );
        ::metrics::describe_gauge!(
            "db.table_entries",
            ::metrics::Unit::Count,
            "Number of entries in the table"
        );
        ::metrics::describe_gauge!(
            "db.freelist",
            ::metrics::Unit::Bytes,
            "Size of the database freelist"
        );

        Self {
            inner: Arc::new(DbMetricsInner {
                transaction: DbTransactionMetrics::default(),
                operations: DbOperationMetrics::default(),
            }),
        }
    }

    /// Records a transaction creation.
    pub fn record_ro_tx_create(&self) {
        self.inner.transaction.ro_created.increment(1);
    }

    pub fn record_rw_tx_create(&self) {
        self.inner.transaction.rw_created.increment(1);
    }

    /// Records a transaction commit with timing.
    ///
    /// ## Arguments
    ///
    /// * `duration` - Time taken for the get operation in seconds
    /// * `success` - Whether the commit operation completed sucessfully or not.
    pub fn record_tx_commit(&self, duration: f64, success: bool) {
        if success {
            self.inner.transaction.commits_successful.increment(1);
        } else {
            self.inner.transaction.commits_failed.increment(1);
        }

        self.inner.transaction.commit_time_seconds.record(duration);
    }

    /// Records a transaction abort.
    pub fn record_tx_abort(&self) {
        self.inner.transaction.aborts.increment(1);
    }

    /// Records a get operation.
    ///
    /// ## Arguments
    ///
    /// * `duration` - Time taken for the get operation in seconds
    /// * `found` - Whether the requested value was found
    pub fn record_get(&self, duration: f64, found: bool) {
        if found {
            self.inner.operations.get_hits.increment(1);
        } else {
            self.inner.operations.get_misses.increment(1);
        }

        self.inner.operations.get_time_seconds.record(duration);
    }

    /// Records a put operation.
    ///
    /// ## Arguments
    ///
    /// * `duration` - Time taken for the put operation in seconds
    pub fn record_put(&self, duration: f64) {
        self.inner.operations.puts.increment(1);
        self.inner.operations.put_time_seconds.record(duration);
    }

    /// Records a delete operation.
    ///
    /// ## Arguments
    ///
    /// * `duration` - Time taken for the delete operation in seconds
    pub fn record_delete(&self, duration: f64, deleted: bool) {
        if deleted {
            self.inner.operations.deletes_successful.increment(1);
        } else {
            self.inner.operations.deletes_failed.increment(1);
        }

        self.inner.operations.delete_time_seconds.record(duration);
    }

    /// Records a clear operation.
    ///
    /// ## Arguments
    ///
    /// * `duration` - Time taken for the clear operation in seconds
    pub fn record_clear(&self, duration: f64) {
        self.inner.operations.clears.increment(1);
        self.inner.operations.clear_time_seconds.record(duration);
    }
}

#[derive(Debug)]
struct DbMetricsInner {
    transaction: DbTransactionMetrics,
    operations: DbOperationMetrics,
}

/// Metrics for database transactions.
#[derive(Metrics, Clone)]
#[metrics(scope = "db.transaction")]
struct DbTransactionMetrics {
    /// Number of read-only transactions created
    ro_created: Counter,
    /// Number of read-write transactions created
    rw_created: Counter,
    /// Number of successful commits
    commits_successful: Counter,
    /// Number of failed commits
    commits_failed: Counter,
    /// Number of transaction aborts
    aborts: Counter,
    /// Time taken to commit a transaction
    commit_time_seconds: Histogram,
}

/// Metrics for database operations.
#[derive(Metrics, Clone)]
#[metrics(scope = "db.operation")]
struct DbOperationMetrics {
    /// Number of get operations that found a value
    get_hits: Counter,
    /// Number of get operations that didn't find a value
    get_misses: Counter,
    /// Time taken for get operations
    get_time_seconds: Histogram,
    /// Number of put operations
    puts: Counter,
    /// Time taken for put operations
    put_time_seconds: Histogram,
    /// Number of successful delete operations
    deletes_successful: Counter,
    /// Number of failed delete operations
    deletes_failed: Counter,
    /// Time taken for delete operations
    delete_time_seconds: Histogram,
    /// Number of clear operations
    clears: Counter,
    /// Time taken for clear operations
    clear_time_seconds: Histogram,
}

/// Helper for timing database operations.
pub(super) struct OpTimer {
    start: Instant,
}

impl OpTimer {
    /// Starts timing an operation.
    pub fn new() -> Self {
        Self { start: Instant::now() }
    }

    /// Returns the elapsed time in seconds.
    pub fn elapsed(&self) -> f64 {
        self.start.elapsed().as_secs_f64()
    }
}
