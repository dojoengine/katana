use std::cmp::Ordering;

use katana_db::abstraction::{DbTx, DbTxMut};
use katana_db::models::storage::StorageEntry;
use katana_db::tables;
use katana_db::trie::TrieDbFactory;
use katana_primitives::block::BlockHashOrNumber;
use katana_primitives::class::{ClassHash, CompiledClassHash, ContractClass};
use katana_primitives::contract::{GenericContractInfo, Nonce, StorageKey, StorageValue};
use katana_primitives::{ContractAddress, Felt};
use katana_provider_api::block::{BlockNumberProvider, HeaderProvider};
use katana_provider_api::contract::{ContractClassProvider, ContractClassWriter};
use katana_provider_api::state::{
    StateFactoryProvider, StateProofProvider, StateProvider, StateRootProvider, StateWriter,
};
use katana_provider_api::ProviderError;
use katana_rpc_types::ContractStorageKeys;

use super::db::{self};
use super::ForkedProvider;
use crate::providers::fork::ForkedDb;
use crate::{BlockNumber, MutableProvider, ProviderFactory, ProviderResult};

impl<Tx1: DbTx> StateFactoryProvider for ForkedProvider<Tx1> {
    fn latest(&self) -> ProviderResult<Box<dyn StateProvider>> {
        let local_provider = db::state::LatestStateProvider(self.local_db.clone());
        let fork_provider = self.fork_db.clone();
        Ok(Box::new(LatestStateProvider { local_provider, fork_provider }))
    }

    fn historical(
        &self,
        block_id: BlockHashOrNumber,
    ) -> ProviderResult<Option<Box<dyn StateProvider>>> {
        let block_number = match block_id {
            BlockHashOrNumber::Num(num) => {
                let local_latest = match self.local_db.latest_number() {
                    Ok(num) => num,
                    Err(ProviderError::MissingLatestBlockNumber) => self.block_id(),
                    Err(err) => return Err(err),
                };
                let latest_num = local_latest.max(self.block_id());

                match num.cmp(&latest_num) {
                    Ordering::Less => Some(num),
                    Ordering::Equal => return self.latest().map(Some),
                    Ordering::Greater => return Ok(None),
                }
            }

            BlockHashOrNumber::Hash(hash) => {
                if let num @ Some(..) = self.local_db.block_number_by_hash(hash)? {
                    num
                } else if let num @ Some(..) =
                    self.fork_db.db.provider().block_number_by_hash(hash)?
                {
                    num
                } else {
                    None
                }
            }
        };

        let Some(block) = block_number else { return Ok(None) };

        let local_provider =
            db::state::HistoricalStateProvider::new(self.local_db.tx().clone(), block);

        Ok(Some(Box::new(HistoricalStateProvider {
            local_provider,
            fork_provider: self.fork_db.clone(),
        })))
    }
}

#[derive(Debug)]
pub(crate) struct LatestStateProvider<Tx1: DbTx> {
    pub(crate) local_provider: db::state::LatestStateProvider<Tx1>,
    pub(crate) fork_provider: ForkedDb,
}

impl<Tx1: DbTx> ContractClassProvider for LatestStateProvider<Tx1> {
    fn class(&self, hash: ClassHash) -> ProviderResult<Option<ContractClass>> {
        if let Some(class) = self.local_provider.class(hash)? {
            Ok(Some(class))
        } else if let Some(class) =
            self.fork_provider.backend.get_class_at(hash, self.fork_provider.block_id)?
        {
            let provider_mut = self.fork_provider.db.provider_mut();
            provider_mut.tx().put::<tables::Classes>(hash, class.clone().into())?;
            provider_mut.commit()?;

            Ok(Some(class))
        } else {
            Ok(None)
        }
    }

    fn compiled_class_hash_of_class_hash(
        &self,
        hash: ClassHash,
    ) -> ProviderResult<Option<CompiledClassHash>> {
        if let res @ Some(..) = self.local_provider.compiled_class_hash_of_class_hash(hash)? {
            Ok(res)
        } else if let Some(compiled_hash) =
            self.fork_provider.backend.get_compiled_class_hash(hash, self.fork_provider.block_id)?
        {
            let provider_mut = self.fork_provider.db.provider_mut();
            provider_mut.tx().put::<tables::CompiledClassHashes>(hash, compiled_hash)?;
            provider_mut.commit()?;

            Ok(Some(compiled_hash))
        } else {
            Ok(None)
        }
    }
}

impl<Tx1: DbTx> LatestStateProvider<Tx1> {
    /// Returns the latest block number, which is the maximum of local_db.latest_number() and
    /// fork_point. This ensures that even if local_db is empty, we return the fork point as the
    /// latest block.
    fn latest_block_number(&self) -> ProviderResult<katana_primitives::block::BlockNumber> {
        let fork_point = self.fork_provider.block_id;
        let local_latest = match self.local_provider.0.latest_number() {
            Ok(num) => num,
            Err(ProviderError::MissingLatestBlockNumber) => fork_point,
            Err(err) => return Err(err),
        };
        Ok(local_latest.max(fork_point))
    }
}

impl<Tx1: DbTx> StateProvider for LatestStateProvider<Tx1> {
    fn nonce(&self, address: ContractAddress) -> ProviderResult<Option<Nonce>> {
        if let Some(nonce) = self.local_provider.nonce(address)? {
            // TEMPFIX:
            //
            // This check is required due to the limitation on how we're storing updates for
            // contracts that were deployed before the fork point. For those contracts,
            // their corresponding entries for the `ContractInfo` table might not exist.
            // In which case, `BlockWriter::insert_block_with_states_and_receipts`
            // implementation of `ForkedProvider` would simply defaulted the fields to zero
            // (depending which field is being updated). Thus, this check is to
            // determine if the this contract's info is split across the local and fork dbs, where
            // the value zero means the data is stored in the forked db.
            //
            // False positive:
            //
            // Nonce can be zero
            if nonce == Nonce::ZERO {
                if let Some(nonce) = self.fork_provider.db.provider().latest()?.nonce(address)? {
                    if nonce != Nonce::ZERO {
                        return Ok(Some(nonce));
                    }
                }
            }

            Ok(Some(nonce))
        } else if let Some(nonce) =
            self.fork_provider.backend.get_nonce(address, self.fork_provider.block_id)?
        {
            let class_hash = self
                .fork_provider
                .backend
                .get_class_hash_at(address, self.fork_provider.block_id)?
                .ok_or(ProviderError::MissingContractClassHash { address })?;

            let entry = GenericContractInfo { nonce, class_hash };

            let provider_mut = self.fork_provider.db.provider_mut();
            provider_mut.tx().put::<tables::ContractInfo>(address, entry)?;
            provider_mut.commit()?;

            Ok(Some(nonce))
        } else {
            Ok(None)
        }
    }

    fn class_hash_of_contract(
        &self,
        address: ContractAddress,
    ) -> ProviderResult<Option<ClassHash>> {
        if let Some(hash) = self.local_provider.class_hash_of_contract(address)? {
            // TEMPFIX:
            //
            // This check is required due to the limitation on how we're storing updates for
            // contracts that were deployed before the fork point. For those contracts,
            // their corresponding entries for the `ContractInfo` table might not exist.
            // In which case, `BlockWriter::insert_block_with_states_and_receipts`
            // implementation of `ForkedProvider` would simply defaulted the fields to zero
            // (depending which field is being updated). Thus, this check is to
            // determine if the this contract's info is split across the local and fork dbs, where
            // the value zero means the data is stored in the forked db.
            //
            // False positive:
            //
            // Some contracts can have class hash of zero (ie special contracts like 0x1, 0x2) hence
            // why we simply return the value if it can't be found in the forked db.
            // This is very hacky but it works for now.
            if hash == ClassHash::ZERO {
                if let Some(hash) =
                    self.fork_provider.db.provider().latest()?.class_hash_of_contract(address)?
                {
                    return Ok(Some(hash));
                }
            }

            Ok(Some(hash))
        } else if let Some(class_hash) =
            self.fork_provider.backend.get_class_hash_at(address, self.fork_provider.block_id)?
        {
            let nonce = self
                .fork_provider
                .backend
                .get_nonce(address, self.fork_provider.block_id)?
                .unwrap_or_default();

            let entry = GenericContractInfo { class_hash, nonce };

            let provider_mut = self.fork_provider.db.provider_mut();
            provider_mut.tx().put::<tables::ContractInfo>(address, entry)?;
            provider_mut.commit()?;

            Ok(Some(class_hash))
        } else {
            Ok(None)
        }
    }

    fn storage(
        &self,
        address: ContractAddress,
        key: StorageKey,
    ) -> ProviderResult<Option<StorageValue>> {
        if let res @ Some(..) = self.local_provider.storage(address, key)? {
            Ok(res)
        } else if let Some(value) =
            self.fork_provider.backend.get_storage(address, key, self.fork_provider.block_id)?
        {
            let entry = StorageEntry { key, value };

            let provider_mut = self.fork_provider.db.provider_mut();
            provider_mut.tx().put::<tables::ContractStorage>(address, entry)?;
            provider_mut.commit()?;

            Ok(Some(value))
        } else {
            Ok(None)
        }
    }
}

impl<Tx1: DbTx> StateProofProvider for LatestStateProvider<Tx1> {
    fn class_multiproof(&self, classes: Vec<ClassHash>) -> ProviderResult<katana_trie::MultiProof> {
        let fork_point = self.fork_provider.block_id;
        let latest_block_number = self.latest_block_number()?;

        if latest_block_number == fork_point {
            let result = self.fork_provider.backend.get_classes_proofs(classes, fork_point)?;
            let proofs = result.expect("proofs should exist for block");

            Ok(proofs.classes_proof.nodes.into())
        } else {
            let mut trie = TrieDbFactory::new(self.local_provider.0.tx().clone())
                .latest()
                .partial_classes_trie();

            let rpc_proof =
                self.fork_provider.backend.get_classes_proofs(classes.clone(), fork_point)?;
            let rpc_root = self.fork_provider.backend.get_global_roots(fork_point)?;

            let proof = rpc_proof
                .map(|p| p.classes_proof.nodes.into())
                .unwrap_or_else(|| katana_trie::MultiProof(Default::default()));
            let root = rpc_root.map(|r| r.global_roots.classes_tree_root).unwrap_or(Felt::ZERO);

            let proofs = trie.partial_multiproof(classes, Some(proof), Some(root));
            Ok(proofs)
        }
    }

    fn contract_multiproof(
        &self,
        addresses: Vec<ContractAddress>,
    ) -> ProviderResult<katana_trie::MultiProof> {
        let fork_point = self.fork_provider.block_id;
        let latest_block_number = self.latest_block_number()?;

        if latest_block_number == fork_point {
            let result = self.fork_provider.backend.get_contracts_proofs(addresses, fork_point)?;
            let proofs = result.expect("proofs should exist for block");

            Ok(proofs.contracts_proof.nodes.into())
        } else {
            let mut trie = TrieDbFactory::new(self.local_provider.0.tx().clone())
                .latest()
                .partial_contracts_trie();

            let rpc_proof =
                self.fork_provider.backend.get_contracts_proofs(addresses.clone(), fork_point)?;
            let rpc_root = self.fork_provider.backend.get_global_roots(fork_point)?;

            let proof = rpc_proof
                .map(|p| p.contracts_proof.nodes.into())
                .unwrap_or_else(|| katana_trie::MultiProof(Default::default()));
            let root = rpc_root.map(|r| r.global_roots.contracts_tree_root).unwrap_or(Felt::ZERO);

            let proofs = trie.partial_multiproof(addresses, Some(proof), Some(root));
            Ok(proofs)
        }
    }

    fn storage_multiproof(
        &self,
        address: ContractAddress,
        storage_keys: Vec<StorageKey>,
    ) -> ProviderResult<katana_trie::MultiProof> {
        let fork_point = self.fork_provider.block_id;
        let latest_block_number = self.latest_block_number()?;

        if latest_block_number == fork_point {
            let key = vec![ContractStorageKeys { address, keys: storage_keys }];
            let result = self.fork_provider.backend.get_storages_proofs(key, fork_point)?;

            let proof = result
                .and_then(|mut p| p.contracts_storage_proofs.nodes.pop())
                .map(|p| p.into())
                .unwrap_or_else(|| katana_trie::MultiProof(Default::default()));

            Ok(proof)
        } else {
            let mut trie = TrieDbFactory::new(self.local_provider.0.tx().clone())
                .latest()
                .partial_storages_trie(address);

            let key = vec![ContractStorageKeys { address, keys: storage_keys.clone() }];
            let rpc_proof = self.fork_provider.backend.get_storages_proofs(key, fork_point)?;
            let rpc_root = self.fork_provider.backend.get_storage_root(address, fork_point)?;

            let proof = rpc_proof
                .and_then(|mut p| p.contracts_storage_proofs.nodes.pop())
                .map(|p| p.into())
                .unwrap_or_else(|| katana_trie::MultiProof(Default::default()));

            let root = rpc_root.unwrap_or(Felt::ZERO);

            let proofs = trie.partial_multiproof(storage_keys, Some(proof), Some(root));
            Ok(proofs)
        }
    }
}

impl<Tx1: DbTx> StateRootProvider for LatestStateProvider<Tx1> {
    fn classes_root(&self) -> ProviderResult<Felt> {
        let fork_point = self.fork_provider.block_id;
        let latest_block_number = self.latest_block_number()?;

        if latest_block_number == fork_point {
            let result = self.fork_provider.backend.get_global_roots(fork_point)?;
            return Ok(result
                .expect("proofs should exist for block")
                .global_roots
                .classes_tree_root);
        }

        let trie = TrieDbFactory::new(self.local_provider.0.tx().clone()).latest().classes_trie();
        let root = trie.root();

        if root == Felt::ZERO {
            let result = self.fork_provider.backend.get_global_roots(fork_point)?;
            Ok(result.expect("proofs should exist for block").global_roots.classes_tree_root)
        } else {
            Ok(root)
        }
    }

    fn contracts_root(&self) -> ProviderResult<Felt> {
        let fork_point = self.fork_provider.block_id;
        let latest_block_number = self.latest_block_number()?;

        if latest_block_number == fork_point {
            let result = self.fork_provider.backend.get_global_roots(fork_point)?;
            return Ok(result
                .expect("proofs should exist for block")
                .global_roots
                .contracts_tree_root);
        }

        let trie = TrieDbFactory::new(self.local_provider.0.tx().clone()).latest().contracts_trie();
        let root = trie.root();

        if root == Felt::ZERO {
            let result = self.fork_provider.backend.get_global_roots(fork_point)?;
            Ok(result.expect("proofs should exist for block").global_roots.contracts_tree_root)
        } else {
            Ok(root)
        }
    }

    fn storage_root(&self, contract: ContractAddress) -> ProviderResult<Option<Felt>> {
        let fork_point = self.fork_provider.block_id;
        let latest_block_number = self.latest_block_number()?;

        if latest_block_number == fork_point {
            let result = self.fork_provider.backend.get_storage_root(contract, fork_point)?;
            let root = result.expect("proofs should exist for block");
            Ok(Some(root))
        } else {
            let root = TrieDbFactory::new(self.local_provider.0.tx().clone())
                .latest()
                .storages_trie(contract)
                .root();

            if root == Felt::ZERO {
                Ok(self
                    .fork_provider
                    .backend
                    .get_storage_root(contract, fork_point)?
                    .or(Some(Felt::ZERO)))
            } else {
                Ok(Some(root))
            }
        }
    }
}

#[derive(Debug)]
struct HistoricalStateProvider<Tx1: DbTx> {
    local_provider: db::state::HistoricalStateProvider<Tx1>,
    fork_provider: ForkedDb,
}

impl<Tx1: DbTx> HistoricalStateProvider<Tx1> {
    fn target_block(&self) -> BlockNumber {
        self.local_provider.block().min(self.fork_provider.block_id)
    }
}

impl<Tx1: DbTx> ContractClassProvider for HistoricalStateProvider<Tx1> {
    fn class(&self, hash: ClassHash) -> ProviderResult<Option<ContractClass>> {
        if let res @ Some(..) = self.local_provider.class(hash)? {
            return Ok(res);
        }

        if let class @ Some(..) =
            self.fork_provider.backend.get_class_at(hash, self.fork_provider.block_id)?
        {
            Ok(class)
        } else {
            Ok(None)
        }
    }

    fn compiled_class_hash_of_class_hash(
        &self,
        hash: ClassHash,
    ) -> ProviderResult<Option<CompiledClassHash>> {
        if let res @ Some(..) = self.local_provider.compiled_class_hash_of_class_hash(hash)? {
            return Ok(res);
        }

        if let Some(compiled_hash) =
            self.fork_provider.backend.get_compiled_class_hash(hash, self.fork_provider.block_id)?
        {
            let provider_mut = self.fork_provider.db.provider_mut();
            provider_mut.tx().put::<tables::CompiledClassHashes>(hash, compiled_hash)?;
            provider_mut.commit()?;

            Ok(Some(compiled_hash))
        } else {
            Ok(None)
        }
    }
}

impl<Tx1: DbTx> StateProvider for HistoricalStateProvider<Tx1> {
    fn nonce(&self, address: ContractAddress) -> ProviderResult<Option<Nonce>> {
        if let res @ Some(..) = self.local_provider.nonce(address)? {
            return Ok(res);
        }

        let block_id = self.target_block();

        Ok(self.fork_provider.backend.get_nonce(address, block_id)?)
    }

    fn class_hash_of_contract(
        &self,
        address: ContractAddress,
    ) -> ProviderResult<Option<ClassHash>> {
        if let res @ Some(..) = self.local_provider.class_hash_of_contract(address)? {
            return Ok(res);
        }

        let block_id = self.target_block();

        Ok(self.fork_provider.backend.get_class_hash_at(address, block_id)?)
    }

    fn storage(
        &self,
        address: ContractAddress,
        key: StorageKey,
    ) -> ProviderResult<Option<StorageValue>> {
        if let res @ Some(..) = self.local_provider.storage(address, key)? {
            return Ok(res);
        }

        let block_id = self.target_block();

        Ok(self.fork_provider.backend.get_storage(address, key, block_id)?)
    }
}

impl<Tx1: DbTx> StateProofProvider for HistoricalStateProvider<Tx1> {
    fn class_multiproof(&self, classes: Vec<ClassHash>) -> ProviderResult<katana_trie::MultiProof> {
        if self.local_provider.block() > self.fork_provider.block_id {
            let mut trie = TrieDbFactory::new(self.local_provider.tx().clone())
                .historical(self.local_provider.block())
                .ok_or(ProviderError::StateProofNotSupported)?
                .partial_classes_trie();

            let rpc_proof = self
                .fork_provider
                .backend
                .get_classes_proofs(classes.clone(), self.fork_provider.block_id)?;
            let rpc_root =
                self.fork_provider.backend.get_global_roots(self.fork_provider.block_id)?;

            let proof = rpc_proof
                .map(|p| p.classes_proof.nodes.into())
                .unwrap_or_else(|| katana_trie::MultiProof(Default::default()));
            let root = rpc_root.map(|r| r.global_roots.classes_tree_root).unwrap_or(Felt::ZERO);

            let proofs = trie.partial_multiproof(classes, Some(proof), Some(root));
            Ok(proofs)
        } else {
            let result = self
                .fork_provider
                .backend
                .get_classes_proofs(classes, self.local_provider.block())?;
            let proofs = result.expect("block should exist");

            Ok(proofs.classes_proof.nodes.into())
        }
    }

    fn contract_multiproof(
        &self,
        addresses: Vec<ContractAddress>,
    ) -> ProviderResult<katana_trie::MultiProof> {
        if self.local_provider.block() > self.fork_provider.block_id {
            let mut trie = TrieDbFactory::new(self.local_provider.tx().clone())
                .historical(self.local_provider.block())
                .ok_or(ProviderError::StateProofNotSupported)?
                .partial_contracts_trie();

            let rpc_proof = self
                .fork_provider
                .backend
                .get_contracts_proofs(addresses.clone(), self.fork_provider.block_id)?;
            let rpc_root =
                self.fork_provider.backend.get_global_roots(self.fork_provider.block_id)?;

            let proof = rpc_proof
                .map(|p| p.contracts_proof.nodes.into())
                .unwrap_or_else(|| katana_trie::MultiProof(Default::default()));
            let root = rpc_root.map(|r| r.global_roots.contracts_tree_root).unwrap_or(Felt::ZERO);

            let proofs = trie.partial_multiproof(addresses, Some(proof), Some(root));
            Ok(proofs)
        } else {
            let result = self
                .fork_provider
                .backend
                .get_contracts_proofs(addresses, self.local_provider.block())?;
            let proofs = result.expect("block should exist");

            Ok(proofs.contracts_proof.nodes.into())
        }
    }

    fn storage_multiproof(
        &self,
        address: ContractAddress,
        storage_keys: Vec<StorageKey>,
    ) -> ProviderResult<katana_trie::MultiProof> {
        if self.local_provider.block() > self.fork_provider.block_id {
            let mut trie = TrieDbFactory::new(self.local_provider.tx().clone())
                .historical(self.local_provider.block())
                .ok_or(ProviderError::StateProofNotSupported)?
                .partial_storages_trie(address);

            let key = vec![ContractStorageKeys { address, keys: storage_keys.clone() }];
            let rpc_proof =
                self.fork_provider.backend.get_storages_proofs(key, self.fork_provider.block_id)?;
            let rpc_root = self
                .fork_provider
                .backend
                .get_storage_root(address, self.fork_provider.block_id)?;

            let proof = rpc_proof
                .and_then(|mut p| p.contracts_storage_proofs.nodes.pop())
                .map(|p| p.into())
                .unwrap_or_else(|| katana_trie::MultiProof(Default::default()));

            let root = rpc_root.unwrap_or(Felt::ZERO);

            let proofs = trie.partial_multiproof(storage_keys, Some(proof), Some(root));
            Ok(proofs)
        } else {
            let key = vec![ContractStorageKeys { address, keys: storage_keys }];
            let result =
                self.fork_provider.backend.get_storages_proofs(key, self.local_provider.block())?;

            let mut proofs = result.expect("block should exist");
            let storage_proof = proofs.contracts_storage_proofs.nodes.pop().unwrap_or_default();

            Ok(storage_proof.into())
        }
    }
}

impl<Tx1: DbTx> StateRootProvider for HistoricalStateProvider<Tx1> {
    fn state_root(&self) -> ProviderResult<Felt> {
        match self.local_provider.state_root() {
            Ok(root) => Ok(root),

            Err(ProviderError::MissingBlockHeader(..)) => {
                let block_id = BlockHashOrNumber::from(self.local_provider.block());

                if let Some(header) = self.fork_provider.db.provider().header(block_id)? {
                    return Ok(header.state_root);
                }

                if self.fork_provider.fetch_historical_blocks(block_id)? {
                    let header = self.fork_provider.db.provider().header(block_id)?.unwrap();
                    Ok(header.state_root)
                } else {
                    Err(ProviderError::MissingBlockHeader(self.local_provider.block()))
                }
            }

            Err(err) => Err(err),
        }
    }

    fn classes_root(&self) -> ProviderResult<Felt> {
        if self.local_provider.block() > self.fork_provider.block_id {
            let root = TrieDbFactory::new(self.local_provider.tx().clone())
                .historical(self.local_provider.block())
                .ok_or(ProviderError::StateProofNotSupported)?
                .classes_trie()
                .root();

            if root == Felt::ZERO {
                let result =
                    self.fork_provider.backend.get_global_roots(self.fork_provider.block_id)?;
                return Ok(result
                    .expect("proofs should exist for block")
                    .global_roots
                    .classes_tree_root);
            } else {
                return Ok(root);
            }
        } else {
            let result =
                self.fork_provider.backend.get_global_roots(self.local_provider.block())?;
            let roots = result.expect("block should exist");
            Ok(roots.global_roots.classes_tree_root)
        }
    }

    fn contracts_root(&self) -> ProviderResult<Felt> {
        if self.local_provider.block() > self.fork_provider.block_id {
            let root = TrieDbFactory::new(self.local_provider.tx().clone())
                .historical(self.local_provider.block())
                .ok_or(ProviderError::StateProofNotSupported)?
                .contracts_trie()
                .root();

            if root == Felt::ZERO {
                let result =
                    self.fork_provider.backend.get_global_roots(self.fork_provider.block_id)?;
                return Ok(result
                    .expect("proofs should exist for block")
                    .global_roots
                    .contracts_tree_root);
            } else {
                return Ok(root);
            }
        } else {
            let result =
                self.fork_provider.backend.get_global_roots(self.local_provider.block())?;
            let roots = result.expect("block should exist");
            Ok(roots.global_roots.contracts_tree_root)
        }
    }

    fn storage_root(&self, contract: ContractAddress) -> ProviderResult<Option<Felt>> {
        if self.local_provider.block() > self.fork_provider.block_id {
            let root = TrieDbFactory::new(self.local_provider.tx().clone())
                .historical(self.local_provider.block())
                .ok_or(ProviderError::StateProofNotSupported)?
                .storages_trie(contract)
                .root();

            if root == Felt::ZERO {
                Ok(self
                    .fork_provider
                    .backend
                    .get_storage_root(contract, self.fork_provider.block_id)?
                    .or(Some(Felt::ZERO)))
            } else {
                Ok(Some(root))
            }
        } else {
            let result = self
                .fork_provider
                .backend
                .get_storage_root(contract, self.local_provider.block())?;
            Ok(result)
        }
    }
}

impl<Tx1: DbTxMut> StateWriter for ForkedProvider<Tx1> {
    fn set_class_hash_of_contract(
        &self,
        address: ContractAddress,
        class_hash: ClassHash,
    ) -> ProviderResult<()> {
        self.local_db.set_class_hash_of_contract(address, class_hash)
    }

    fn set_nonce(&self, address: ContractAddress, nonce: Nonce) -> ProviderResult<()> {
        self.local_db.set_nonce(address, nonce)
    }

    fn set_storage(
        &self,
        address: ContractAddress,
        storage_key: StorageKey,
        storage_value: StorageValue,
    ) -> ProviderResult<()> {
        self.local_db.set_storage(address, storage_key, storage_value)
    }
}

impl<Tx1: DbTxMut> ContractClassWriter for ForkedProvider<Tx1> {
    fn set_class(&self, hash: ClassHash, class: ContractClass) -> ProviderResult<()> {
        self.local_db.set_class(hash, class)
    }

    fn set_compiled_class_hash_of_class_hash(
        &self,
        hash: ClassHash,
        compiled_hash: CompiledClassHash,
    ) -> ProviderResult<()> {
        self.local_db.set_compiled_class_hash_of_class_hash(hash, compiled_hash)
    }
}
