use katana_gateway::types::{ErrorCode, GatewayError};
use katana_primitives::block::BlockNumber;
use katana_primitives::class::{ClassHash, CompiledClassHash, ContractClass};
use katana_primitives::contract::{ContractAddress, Nonce, StorageKey, StorageValue};
use katana_primitives::state::StateUpdates;
use katana_primitives::Felt;
use katana_provider::api::contract::ContractClassProvider;
use katana_provider::api::state::{StateProofProvider, StateProvider, StateRootProvider};
use katana_provider::{ProviderError, ProviderResult};
use katana_rpc_types::ConversionError;
use tokio::runtime;

pub struct PendingStateProvider {
    base: Box<dyn StateProvider>,
    pending_block_id: BlockNumber,
    pending_state_updates: StateUpdates,
    gateway: katana_gateway::client::Client,
}

impl StateProvider for PendingStateProvider {
    fn nonce(&self, address: ContractAddress) -> ProviderResult<Option<Nonce>> {
        if let Some(nonce) = self.pending_state_updates.nonce_updates.get(&address) {
            return Ok(Some(*nonce));
        }

        self.base.nonce(address)
    }

    fn storage(
        &self,
        address: ContractAddress,
        storage_key: StorageKey,
    ) -> ProviderResult<Option<StorageValue>> {
        if let Some(contract_storage) = self.pending_state_updates.storage_updates.get(&address) {
            if let Some(value) = contract_storage.get(&storage_key) {
                return Ok(Some(*value));
            }
        }

        self.base.storage(address, storage_key)
    }

    fn class_hash_of_contract(
        &self,
        address: ContractAddress,
    ) -> ProviderResult<Option<ClassHash>> {
        if let Some(class_hash) = self.pending_state_updates.replaced_classes.get(&address) {
            return Ok(Some(*class_hash));
        }

        if let Some(class_hash) = self.pending_state_updates.deployed_contracts.get(&address) {
            return Ok(Some(*class_hash));
        }

        self.base.class_hash_of_contract(address)
    }
}

impl ContractClassProvider for PendingStateProvider {
    fn class(&self, hash: ClassHash) -> ProviderResult<Option<ContractClass>> {
        if let Some(class) = self.base.class(hash)? {
            return Ok(Some(class));
        }

        let result = runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(self.gateway.get_class(hash, katana_gateway::types::BlockId::Pending));

        match result {
            Ok(class) => {
                let class = class
                    .try_into()
                    .map_err(|e: ConversionError| ProviderError::Other(e.to_string()))?;
                Ok(Some(class))
            }

            Err(error) => {
                if let katana_gateway::client::Error::Sequencer(GatewayError {
                    code: ErrorCode::UndeclaredClass,
                    ..
                }) = error
                {
                    Ok(None)
                } else {
                    Err(ProviderError::Other(error.to_string()))
                }
            }
        }
    }

    fn compiled_class_hash_of_class_hash(
        &self,
        hash: ClassHash,
    ) -> ProviderResult<Option<CompiledClassHash>> {
        if let Some(compiled_hash) = self.pending_state_updates.declared_classes.get(&hash) {
            return Ok(Some(*compiled_hash));
        }

        // Fallback to the base provider
        self.base.compiled_class_hash_of_class_hash(hash)
    }
}

impl StateRootProvider for PendingStateProvider {}
impl StateProofProvider for PendingStateProvider {}
