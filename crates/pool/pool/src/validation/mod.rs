pub mod stateful;

use katana_pool_api::validation::{ValidationOutcome, ValidationResult, Validator};
use katana_pool_api::PoolTransaction;

/// A no-op validator that does nothing and assume all incoming transactions are valid.
#[derive(Debug)]
pub struct NoopValidator<T>(std::marker::PhantomData<T>);

impl<T> NoopValidator<T> {
    pub fn new() -> Self {
        Self(std::marker::PhantomData)
    }
}

impl<T: PoolTransaction> Validator for NoopValidator<T> {
    type Transaction = T;

    #[allow(clippy::manual_async_fn)]
    fn validate(
        &self,
        tx: Self::Transaction,
    ) -> impl std::future::Future<Output = ValidationResult<Self::Transaction>> + Send {
        async move { ValidationResult::Ok(ValidationOutcome::Valid(tx)) }
    }
}

impl<T> Default for NoopValidator<T> {
    fn default() -> Self {
        Self::new()
    }
}
