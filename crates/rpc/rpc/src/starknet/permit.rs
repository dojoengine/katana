use std::sync::Arc;

use tokio::sync::Semaphore;

use crate::starknet::StarknetApiError;

#[derive(Debug, Clone)]
pub struct Permit {
    semaphore: Arc<Semaphore>,
}

impl Permit {
    pub fn new(permits: u32) -> Self {
        Self { semaphore: Arc::new(Semaphore::new(permits as usize)) }
    }

    pub async fn acquire(&self) -> Result<tokio::sync::OwnedSemaphorePermit, StarknetApiError> {
        self.semaphore.clone().acquire_owned().await.map_err(|_| {
            StarknetApiError::UnexpectedError {
                reason: "Failed to acquire estimate_fee semaphore permit".to_string(),
            }
        })
    }
}
