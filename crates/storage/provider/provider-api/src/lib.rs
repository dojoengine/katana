//! `-Ext` suffixed traits means they are not meant to be used in the main program flow. Usually as
//! a way to restrict certain operations/functions from being accessed at certain parts of the
//! program.

pub mod block;
pub mod contract;
pub mod env;
mod error;
pub mod stage;
pub mod state;
pub mod state_update;
pub mod transaction;
pub mod trie;

pub use error::ProviderError;

/// A result type for blockchain providers.
pub type ProviderResult<T> = Result<T, error::ProviderError>;
