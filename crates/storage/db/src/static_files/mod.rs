//! Static file storage for immutable, append-only block and transaction data.
//!
//! Heavy values (headers, transactions, receipts, traces, state updates) are stored in
//! sequential `.dat` files instead of MDBX B-trees. MDBX retains the role of authoritative
//! index — storing [`StaticFileRef`] pointers and all mutable/random-access data.
//!
//! See `crates/storage/db/docs/static-files.md` for the full design document.

pub mod column;
pub mod manifest;
pub mod segment;
pub mod store;

pub use segment::{StaticFiles, StaticFilesBuilder};
pub use store::{AnyStore, FileStore, FileStoreConfig, MemoryStore, StaticStore};
