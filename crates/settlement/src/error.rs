//! The settlement service's error type.

use crate::backend::tee::TeeProverError;
use crate::piltover::PiltoverError;

/// Errors produced by the embedded settlement service.
///
/// The umbrella over each layer's concrete error: the Piltover chain client
/// ([`PiltoverError`]), the proving backend ([`TeeProverError`]), attestation
/// construction ([`katana_tee::attestation::AttestationError`]), and local
/// chain reads ([`katana_provider::ProviderError`]).
#[derive(Debug, thiserror::Error)]
pub enum SettlementError {
    #[error(transparent)]
    Piltover(#[from] PiltoverError),

    #[error(transparent)]
    Prover(#[from] TeeProverError),

    #[error(transparent)]
    Attestation(#[from] katana_tee::attestation::AttestationError),

    #[error(transparent)]
    Provider(#[from] katana_provider::ProviderError),
}
