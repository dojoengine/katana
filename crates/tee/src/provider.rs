use std::fmt::Debug;

use crate::error::TeeError;

/// Trait for TEE providers that can generate attestation quotes.
///
/// Implementations of this trait interact with TEE hardware to generate
/// cryptographic attestation quotes that bind user-provided data to the
/// hardware state.
pub trait TeeProvider: Send + Sync + Debug {
    /// Generate an attestation quote with the given user data.
    ///
    /// # Arguments
    /// * `user_data` - A 64-byte slice of user-provided data to include in the quote.
    ///                 This is typically a hash commitment to application state.
    ///
    /// # Returns
    /// * `Ok(Vec<u8>)` - The raw attestation quote bytes.
    /// * `Err(TeeError)` - If quote generation fails.
    fn generate_quote(&self, user_data: &[u8; 64]) -> Result<Vec<u8>, TeeError>;

    /// Returns the name/type of this TEE provider for logging purposes.
    fn provider_type(&self) -> &'static str;
}
