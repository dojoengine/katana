use std::cell::{OnceCell, RefCell};
use std::collections::HashSet;
use std::sync::Arc;

use alloy_primitives::U256;
use katana_contracts::contracts;
use katana_genesis::allocation::{DevGenesisAccount, GenesisAccountAlloc};
use katana_genesis::constant::DEFAULT_STRK_FEE_TOKEN_ADDRESS;
use katana_primitives::class::{ClassHash, ContractClass};
use katana_primitives::contract::{ContractAddress, Nonce};
use katana_primitives::da::DataAvailabilityMode;
use katana_primitives::fee::{AllResourceBoundsMapping, ResourceBounds, ResourceBoundsMapping};
use katana_primitives::transaction::{
    DeclareTx, DeclareTxV0, DeclareTxV3, DeclareTxWithClass, DeployAccountTx, DeployAccountTxV3,
    ExecutableTx, ExecutableTxWithHash, InvokeTx, InvokeTxV3,
};
use katana_primitives::utils::transaction::compute_deploy_account_v3_tx_hash;
use katana_primitives::utils::{get_contract_address, split_u256};
use katana_primitives::{felt, Felt};
use num_traits::FromPrimitive;
use starknet::core::utils::get_selector_from_name;
use starknet::signers::SigningKey;

use crate::rollup::ChainSpec;

/// The fee-token initial supply that the rollup ERC20 was originally minted with
/// (`u256{ low: u128::MAX, high: u128::MAX }`). Now used by
/// [`crate::rollup::ChainSpec::state_updates`] when seeding the pre-allocated fee token's balance
/// for the genesis master account, so behavior matches the previous on-chain `deploy_contract`
/// flow.
pub(crate) const ROLLUP_FEE_TOKEN_INITIAL_SUPPLY: U256 = U256::MAX;

/// Salt used when computing the genesis master account's address.
const MASTER_ACCOUNT_SALT: Felt = Felt::ONE;

/// Private key the genesis master account is bootstrapped with.
const MASTER_ACCOUNT_PRIVATE_KEY: Felt = felt!("0xa55");

/// Computes the address of the genesis master account.
///
/// The master account is the sender of every declare/deploy/invoke transaction emitted by
/// [`GenesisTransactionsBuilder`]; it is deployed in `build_master_account` and is not a regular
/// `Genesis::allocations` entry. Exposed so [`crate::rollup::ChainSpec::state_updates`] can credit
/// it with the fee token's initial supply at the pre-allocated STRK contract.
pub(crate) fn master_account_address() -> ContractAddress {
    let signer = SigningKey::from_secret_scalar(MASTER_ACCOUNT_PRIVATE_KEY);
    let pubkey = signer.verifying_key().scalar();
    let class_hash = contracts::GenesisAccount::HASH;
    let address =
        get_contract_address(MASTER_ACCOUNT_SALT, class_hash, &[pubkey], ContractAddress::ZERO);
    address.into()
}

/// A convenience builder for creating valid and executable transactions for the genesis block based
/// on the [`Genesis`].
///
/// The transactions are crafted in a way that can be executed by the StarknetOS Cairo program and
/// thus `blockifier`.
#[derive(Debug)]
pub struct GenesisTransactionsBuilder<'c> {
    chain_spec: &'c ChainSpec,
    fee_token: OnceCell<ContractAddress>,
    master_address: OnceCell<ContractAddress>,
    master_signer: SigningKey,
    master_nonce: RefCell<Nonce>,
    transactions: RefCell<Vec<ExecutableTxWithHash>>,
    declared_classes: RefCell<HashSet<ClassHash>>,
}

impl<'c> GenesisTransactionsBuilder<'c> {
    /// Creates a new [`GenesisTransactionsBuilder`] for the given [`ChainSpec`].
    pub fn new(chain_spec: &'c ChainSpec) -> Self {
        Self {
            chain_spec,
            fee_token: OnceCell::new(),
            master_address: OnceCell::new(),
            transactions: RefCell::new(Vec::new()),
            master_nonce: RefCell::new(Nonce::ZERO),
            declared_classes: RefCell::new(HashSet::new()),
            master_signer: SigningKey::from_secret_scalar(MASTER_ACCOUNT_PRIVATE_KEY),
        }
    }

    fn legacy_declare(&self, class: ContractClass) -> ClassHash {
        if matches!(class, ContractClass::Class(..)) {
            panic!("legacy_declare must be called only with legacy class")
        }

        let class = Arc::new(class);
        let class_hash = class.class_hash().unwrap();

        // No need to declare the same class if it was already declared.
        if self.declared_classes.borrow_mut().contains(&class_hash) {
            return class_hash;
        }

        let transaction = ExecutableTx::Declare(DeclareTxWithClass {
            transaction: DeclareTx::V0(DeclareTxV0 {
                sender_address: Felt::ONE.into(),
                chain_id: self.chain_spec.id,
                signature: Vec::new(),
                class_hash,
                max_fee: 0,
            }),
            class,
        });

        let tx_hash = transaction.calculate_hash(false);
        self.declared_classes.borrow_mut().insert(class_hash);
        self.transactions.borrow_mut().push(ExecutableTxWithHash { hash: tx_hash, transaction });

        class_hash
    }

    fn declare(&self, class: ContractClass) -> ClassHash {
        let class_hash = class.class_hash().unwrap();

        // No need to declare the same class if it was already declared. This check must
        // come *before* consuming a nonce: a dedup hit that still bumped `master_nonce`
        // would leave a gap in the master account's nonce sequence, making every
        // subsequent genesis declare tx fail with `InvalidNonce`. This bites classes
        // preloaded via `--cartridge.controllers`: their hashes sort after the
        // already-declared Account/UDC classes, so a dedup-induced gap aborts the
        // controller declares and they never land at their canonical class hash.
        if self.declared_classes.borrow().contains(&class_hash) {
            return class_hash;
        }

        let nonce = self.master_nonce.replace_with(|&mut n| n + Felt::ONE);
        let sender_address = *self.master_address.get().expect("must be initialized first");

        let compiled_class_hash = class.clone().compile().unwrap().class_hash().unwrap();

        let mut transaction = DeclareTxV3 {
            chain_id: self.chain_spec.id,
            signature: Vec::new(),
            compiled_class_hash,
            sender_address,
            class_hash,
            nonce,
            account_deployment_data: vec![],
            fee_data_availability_mode: DataAvailabilityMode::L1,
            nonce_data_availability_mode: DataAvailabilityMode::L1,
            resource_bounds: ResourceBoundsMapping::All(AllResourceBoundsMapping {
                l1_gas: ResourceBounds { max_amount: 0, max_price_per_unit: 0 },
                l2_gas: ResourceBounds { max_amount: u64::MAX, max_price_per_unit: 0 },
                l1_data_gas: ResourceBounds { max_amount: 0, max_price_per_unit: 0 },
            }),
            paymaster_data: vec![],
            tip: 0,
        };

        let hash = DeclareTx::V3(transaction.clone()).calculate_hash(false);
        let signature = self.master_signer.sign(&hash).unwrap();
        transaction.signature = vec![signature.r, signature.s];

        self.transactions.borrow_mut().push(ExecutableTxWithHash {
            transaction: ExecutableTx::Declare(DeclareTxWithClass {
                transaction: DeclareTx::V3(transaction),
                class: class.into(),
            }),
            hash,
        });

        self.declared_classes.borrow_mut().insert(class_hash);

        class_hash
    }

    fn deploy(&self, class: ClassHash, ctor_args: Vec<Felt>, salt: Felt) -> ContractAddress {
        use std::iter;

        const DEPLOY_CONTRACT_SELECTOR: &str = "deploy_contract";
        let master_address = *self.master_address.get().expect("must be initialized first");

        let contract_address = get_contract_address(salt, class, &ctor_args, ContractAddress::ZERO);

        let ctor_args_len = Felt::from_usize(ctor_args.len()).unwrap();
        let args: Vec<Felt> = iter::once(class) // class_hash
			.chain(iter::once(salt)) // contract_address_salt
			.chain(iter::once(ctor_args_len)) // constructor_calldata_len
			.chain(ctor_args) // constructor_calldata
			.chain(iter::once(Felt::ONE)) // deploy_from_zero
			.collect();

        self.invoke(master_address, DEPLOY_CONTRACT_SELECTOR, args);

        contract_address.into()
    }

    fn invoke(&self, contract: ContractAddress, function: &str, args: Vec<Felt>) {
        use std::iter;

        let nonce = self.master_nonce.replace_with(|&mut n| n + Felt::ONE);
        let sender_address = *self.master_address.get().expect("must be initialized first");
        let selector = get_selector_from_name(function).unwrap();

        let args_len = Felt::from_usize(args.len()).unwrap();
        let calldata: Vec<Felt> = iter::once(Felt::ONE)
            .chain(iter::once(contract.into()))
            .chain(iter::once(selector))
            .chain(iter::once(Felt::ZERO))
            .chain(iter::once(args_len))
            .chain(iter::once(args_len))
            .chain(args)
            .collect();

        let mut transaction = InvokeTxV3 {
            chain_id: self.chain_spec.id,
            signature: Vec::new(),
            sender_address,
            calldata,
            nonce,
            resource_bounds: ResourceBoundsMapping::All(AllResourceBoundsMapping {
                l1_gas: ResourceBounds { max_amount: 0, max_price_per_unit: 0 },
                l2_gas: ResourceBounds { max_amount: u64::MAX, max_price_per_unit: 0 },
                l1_data_gas: ResourceBounds { max_amount: 0, max_price_per_unit: 0 },
            }),
            tip: 0,
            paymaster_data: vec![],
            account_deployment_data: vec![],
            nonce_data_availability_mode: DataAvailabilityMode::L1,
            fee_data_availability_mode: DataAvailabilityMode::L1,
        };

        let tx_hash = InvokeTx::V3(transaction.clone()).calculate_hash(false);
        let signature = self.master_signer.sign(&tx_hash).unwrap();
        transaction.signature = vec![signature.r, signature.s];

        self.transactions.borrow_mut().push(ExecutableTxWithHash {
            transaction: ExecutableTx::Invoke(InvokeTx::V3(transaction)),
            hash: tx_hash,
        });
    }

    fn deploy_predeployed_dev_account(&self, account: &DevGenesisAccount) -> ContractAddress {
        let signer = SigningKey::from_secret_scalar(account.private_key);
        let pubkey = signer.verifying_key().scalar();

        let class_hash = account.class_hash;
        let calldata = vec![pubkey];
        let account_address =
            get_contract_address(account.salt, class_hash, &calldata, ContractAddress::ZERO);

        let tx_hash = compute_deploy_account_v3_tx_hash(
            account_address,
            &calldata,
            class_hash,
            account.salt,
            0,
            &ResourceBounds { max_amount: 0, max_price_per_unit: 0 },
            &ResourceBounds { max_amount: u64::MAX, max_price_per_unit: 0 },
            Some(&ResourceBounds { max_amount: 0, max_price_per_unit: 0 }),
            &[],
            self.chain_spec.id.into(),
            Felt::ZERO,
            &DataAvailabilityMode::L1,
            &DataAvailabilityMode::L1,
            false,
        );

        let signature = signer.sign(&tx_hash).unwrap();

        let transaction = ExecutableTx::DeployAccount(DeployAccountTx::V3(DeployAccountTxV3 {
            signature: vec![signature.r, signature.s],
            contract_address: account_address.into(),
            constructor_calldata: calldata,
            chain_id: self.chain_spec.id,
            contract_address_salt: account.salt,
            nonce: Felt::ZERO,
            class_hash,
            fee_data_availability_mode: DataAvailabilityMode::L1,
            nonce_data_availability_mode: DataAvailabilityMode::L1,
            paymaster_data: vec![],
            tip: 0,
            resource_bounds: ResourceBoundsMapping::All(AllResourceBoundsMapping {
                l1_gas: ResourceBounds { max_amount: 0, max_price_per_unit: 0 },
                l2_gas: ResourceBounds { max_amount: u64::MAX, max_price_per_unit: 0 },
                l1_data_gas: ResourceBounds { max_amount: 0, max_price_per_unit: 0 },
            }),
        }));

        let tx_hash = transaction.calculate_hash(false);
        self.transactions.borrow_mut().push(ExecutableTxWithHash { hash: tx_hash, transaction });

        account_address.into()
    }

    fn deploy_predeployed_account(&self, salt: Felt, public_key: Felt) -> ContractAddress {
        self.deploy(contracts::Account::HASH, vec![public_key], salt)
    }

    fn build_master_account(&self) {
        let account_class_hash = self.legacy_declare(contracts::GenesisAccount::CLASS.clone());

        let master_pubkey = self.master_signer.verifying_key().scalar();
        let calldata = vec![master_pubkey];
        let salt = MASTER_ACCOUNT_SALT;
        let master_address =
            get_contract_address(salt, account_class_hash, &calldata, ContractAddress::ZERO);

        self.master_address.set(master_address.into()).expect("must be uninitialized");

        let deploy_account_tx_hash = compute_deploy_account_v3_tx_hash(
            master_address,
            &calldata,
            account_class_hash,
            salt,
            0,
            &ResourceBounds { max_amount: 0, max_price_per_unit: 0 },
            &ResourceBounds { max_amount: u64::MAX, max_price_per_unit: 0 },
            Some(&ResourceBounds { max_amount: 0, max_price_per_unit: 0 }),
            &[],
            self.chain_spec.id.into(),
            Felt::ZERO,
            &DataAvailabilityMode::L1,
            &DataAvailabilityMode::L1,
            false,
        );

        let signature = self.master_signer.sign(&deploy_account_tx_hash).unwrap();

        let transaction = ExecutableTx::DeployAccount(DeployAccountTx::V3(DeployAccountTxV3 {
            signature: vec![signature.r, signature.s],
            nonce: Felt::ZERO,
            contract_address_salt: salt,
            contract_address: master_address.into(),
            constructor_calldata: calldata,
            class_hash: account_class_hash,
            chain_id: self.chain_spec.id,
            fee_data_availability_mode: DataAvailabilityMode::L1,
            nonce_data_availability_mode: DataAvailabilityMode::L1,
            paymaster_data: vec![],
            tip: 0,
            resource_bounds: ResourceBoundsMapping::All(AllResourceBoundsMapping {
                l1_gas: ResourceBounds { max_amount: 0, max_price_per_unit: 0 },
                l2_gas: ResourceBounds { max_amount: u64::MAX, max_price_per_unit: 0 },
                l1_data_gas: ResourceBounds { max_amount: 0, max_price_per_unit: 0 },
            }),
        }));

        let tx_hash = transaction.calculate_hash(false);
        self.transactions.borrow_mut().push(ExecutableTxWithHash { hash: tx_hash, transaction });
        self.master_nonce.replace(Nonce::ONE);
    }

    fn build_core_contracts(&mut self) {
        let udc_class_hash = self.declare(contracts::OpenZeppelinUniversalDeployer::CLASS.clone());
        self.deploy(udc_class_hash, Vec::new(), Felt::ZERO);

        // Legacy UDC (Cairo 0): declared and deployed alongside the current one so tooling
        // that still targets the legacy UDC's canonical address keeps working.
        let legacy_udc_class_hash =
            self.legacy_declare(contracts::UniversalDeployer::CLASS.clone());
        self.deploy(legacy_udc_class_hash, Vec::new(), Felt::ZERO);

        // STRK fee token is pre-allocated to genesis state at the canonical Starknet mainnet
        // address by [`crate::rollup::ChainSpec::state_updates`], not deployed via UDC here —
        // UDC-derived addresses can't land at the canonical address. The master account holds
        // the full initial supply (see [`ROLLUP_FEE_TOKEN_INITIAL_SUPPLY`]) so the
        // `transfer_balance` invokes below behave the same as before.
        self.fee_token.set(DEFAULT_STRK_FEE_TOKEN_ADDRESS).expect("must be uninitialized");
    }

    fn build_allocated_accounts(&mut self) {
        let default_account_class_hash = self.declare(contracts::Account::CLASS.clone());

        for (expected_addr, account) in self.chain_spec.genesis.accounts() {
            if account.class_hash() != default_account_class_hash {
                panic!(
                    "unexpected account class hash; expected {default_account_class_hash:#x}, got \
                     {:#x}",
                    account.class_hash()
                )
            }

            let address = match account {
                GenesisAccountAlloc::DevAccount(account) => {
                    self.deploy_predeployed_dev_account(account)
                }
                GenesisAccountAlloc::Account(account) => {
                    self.deploy_predeployed_account(account.salt, account.public_key)
                }
            };

            debug_assert_eq!(&address, expected_addr);

            if let Some(amount) = account.balance() {
                self.transfer_balance(address, amount)
            }
        }
    }

    fn transfer_balance(&self, recipient: ContractAddress, balance: U256) {
        let fee_token = *self.fee_token.get().expect("must be initialized first");

        let (low_amount, high_amount) = split_u256(balance);
        let args = vec![recipient.into(), low_amount, high_amount];

        const TRANSFER: &str = "transfer";
        self.invoke(fee_token, TRANSFER, args);
    }

    /// Declare any remaining classes preloaded into the genesis via a real declare
    /// transaction. This is how opt-in additions such as the Cartridge Controller
    /// account classes (`katana init rollup --cartridge-controllers`) land on the
    /// rollup: declaring them through a transaction — rather than only inserting
    /// them into `genesis.classes` — is what makes them available by their
    /// canonical class hash. `declare` recomputes the hash from the class artifact,
    /// so the class is declared under its canonical hash even if the genesis.json
    /// round-trip shifted the `genesis.classes` map key.
    ///
    /// The UDC and account classes are already declared by the steps above, so
    /// `declare`/`legacy_declare`'s dedup makes them no-ops here; the STRK fee token
    /// class is pre-allocated into state (not declared via a tx) and is skipped
    /// explicitly. So a rollup without `--cartridge-controllers` has byte-identical
    /// genesis transactions. This must run last, after those steps have populated
    /// `declared_classes`.
    fn build_preloaded_classes(&self) {
        // Collect first so we don't hold a borrow on `self.chain_spec` across the
        // `declare` calls.
        let classes: Vec<Arc<ContractClass>> =
            self.chain_spec.genesis.classes.values().cloned().collect();

        for class in classes {
            // HACK: the STRK fee token (LegacyERC20) class is already handled by
            // `build_core_contracts`, so skip it here to avoid declaring it twice.
            if class.class_hash().unwrap() == contracts::LegacyERC20::HASH {
                continue;
            }
            match class.as_ref() {
                ContractClass::Class(..) => self.declare(class.as_ref().clone()),
                ContractClass::Legacy(..) => self.legacy_declare(class.as_ref().clone()),
            };
        }
    }

    pub fn build(mut self) -> Vec<ExecutableTxWithHash> {
        self.build_master_account();
        self.build_core_contracts();
        self.build_allocated_accounts();
        self.build_preloaded_classes();
        self.transactions.into_inner()
    }
}

#[cfg(test)]
mod tests {

    use alloy_primitives::U256;
    use katana_contracts::contracts;
    use katana_genesis::allocation::{
        DevAllocationsGenerator, GenesisAccount, GenesisAccountAlloc, GenesisAllocation,
    };
    use katana_genesis::constant::DEFAULT_PREFUNDED_ACCOUNT_BALANCE;
    use katana_genesis::Genesis;
    use katana_primitives::chain::ChainId;
    use katana_primitives::class::ClassHash;
    use katana_primitives::transaction::TxType;
    use katana_primitives::Felt;
    use url::Url;

    use super::GenesisTransactionsBuilder;
    use crate::rollup::{ChainSpec, FeeContracts};
    use crate::SettlementLayer;

    fn chain_spec(n_dev_accounts: u16, with_balance: bool) -> ChainSpec {
        let accounts = if with_balance {
            DevAllocationsGenerator::new(n_dev_accounts)
                .with_balance(U256::from(DEFAULT_PREFUNDED_ACCOUNT_BALANCE))
                .generate()
        } else {
            DevAllocationsGenerator::new(n_dev_accounts).generate()
        };

        let mut genesis = Genesis::default();
        genesis.extend_allocations(accounts.into_iter().map(|(k, v)| (k, v.into())));

        let id = ChainId::parse("KATANA").unwrap();
        let fee_contracts = FeeContracts::default();

        let settlement = SettlementLayer::Starknet {
            block: 0,
            id: ChainId::default(),
            core_contract: Default::default(),
            rpc_url: Url::parse("http://localhost:5050").unwrap(),
            proof_kind: Default::default(),
        };

        ChainSpec { id, genesis, settlement, fee_contracts, settlement_runtime: None }
    }

    #[test]
    fn strk_pre_allocated_at_canonical_address() {
        use katana_genesis::constant::{
            DEFAULT_STRK_FEE_TOKEN_ADDRESS, ERC20_DECIMAL_STORAGE_SLOT, ERC20_NAME_STORAGE_SLOT,
            ERC20_SYMBOL_STORAGE_SLOT, ERC20_TOTAL_SUPPLY_STORAGE_SLOT,
        };
        use katana_primitives::cairo::ShortString;
        use katana_primitives::transaction::{DeclareTx, ExecutableTx};

        let chain_spec = chain_spec(1, true);

        // (a) The genesis tx body must not declare or deploy the LegacyERC20 class — STRK
        // lives in pre-allocated state now.
        let txs = GenesisTransactionsBuilder::new(&chain_spec).build();
        for tx in &txs {
            if let ExecutableTx::Declare(declare) = &tx.transaction {
                let hash = match &declare.transaction {
                    DeclareTx::V0(t) => t.class_hash,
                    DeclareTx::V1(t) => t.class_hash,
                    DeclareTx::V2(t) => t.class_hash,
                    DeclareTx::V3(t) => t.class_hash,
                };
                assert_ne!(
                    hash,
                    contracts::LegacyERC20::HASH,
                    "LegacyERC20 must not appear in genesis tx declares"
                );
            }
        }

        // (b) state_updates pre-allocates LegacyERC20 at the canonical Starknet mainnet address
        // with the expected ERC20 metadata slots populated.
        let states = chain_spec.state_updates();
        assert_eq!(
            states.state_updates.deployed_contracts.get(&DEFAULT_STRK_FEE_TOKEN_ADDRESS),
            Some(&contracts::LegacyERC20::HASH),
        );
        assert!(states
            .state_updates
            .deprecated_declared_classes
            .contains(&contracts::LegacyERC20::HASH));

        let storage = states
            .state_updates
            .storage_updates
            .get(&DEFAULT_STRK_FEE_TOKEN_ADDRESS)
            .expect("STRK storage seeded");
        assert_eq!(
            storage.get(&ERC20_NAME_STORAGE_SLOT).copied(),
            Some(ShortString::from_ascii("Starknet Token").into()),
        );
        assert_eq!(
            storage.get(&ERC20_SYMBOL_STORAGE_SLOT).copied(),
            Some(ShortString::from_ascii("STRK").into()),
        );
        assert_eq!(storage.get(&ERC20_DECIMAL_STORAGE_SLOT).copied(), Some(Felt::from(18u8)));
        // Total supply slot must be present (exact value depends on allocations; just sanity-check
        // it is non-zero given the master account holds U256::MAX and the dev account is funded).
        assert!(storage.get(&ERC20_TOTAL_SUPPLY_STORAGE_SLOT).is_some());
    }

    #[test]
    fn transaction_order() {
        let chain_spec = chain_spec(1, true);
        let transactions = GenesisTransactionsBuilder::new(&chain_spec).build();

        let expected_order = vec![
            TxType::Declare,       // Master account class declare
            TxType::DeployAccount, // Master account
            TxType::Declare,       // UDC declare
            TxType::Invoke,        // UDC deploy
            TxType::Declare,       // Legacy UDC declare
            TxType::Invoke,        // Legacy UDC deploy
            // ERC20 declare/deploy intentionally absent — the STRK fee token is
            // pre-allocated to genesis state by ChainSpec::state_updates instead.
            TxType::Declare,       // Account class declare (V2)
            TxType::DeployAccount, // Dev account
            TxType::Invoke,        // Balance transfer
        ];

        assert_eq!(transactions.len(), expected_order.len());
        for (tx, expected) in transactions.iter().zip(expected_order) {
            assert_eq!(tx.transaction.r#type(), expected);
        }
    }

    /// Classes preloaded into the genesis (e.g. the Cartridge Controller account
    /// classes added by `--cartridge-controllers`) must be declared via a real
    /// genesis declare tx under their canonical hash — not merely left in
    /// `genesis.classes`, which a genesis.json round-trip can shift.
    #[test]
    fn preloaded_classes_declared_via_tx() {
        use katana_contracts::controller::ControllerLatest;
        use katana_primitives::transaction::{DeclareTx, ExecutableTx};

        let mut chain_spec = chain_spec(1, true);

        // Preload a controller class, the way `add_controller_classes` does.
        chain_spec
            .genesis
            .classes
            .insert(ControllerLatest::HASH, ControllerLatest::CLASS.clone().into());

        let txs = GenesisTransactionsBuilder::new(&chain_spec).build();
        let declared: Vec<ClassHash> = txs
            .iter()
            .filter_map(|tx| match &tx.transaction {
                ExecutableTx::Declare(d) => Some(match &d.transaction {
                    DeclareTx::V0(t) => t.class_hash,
                    DeclareTx::V1(t) => t.class_hash,
                    DeclareTx::V2(t) => t.class_hash,
                    DeclareTx::V3(t) => t.class_hash,
                }),
                _ => None,
            })
            .collect();

        // The preloaded controller class is declared, under its canonical hash.
        assert!(
            declared.contains(&ControllerLatest::HASH),
            "preloaded controller class must be declared via a genesis tx"
        );
        // The STRK fee token class stays pre-allocated to state — never declared.
        assert!(
            !declared.contains(&contracts::LegacyERC20::HASH),
            "fee token class must remain pre-allocated, not declared"
        );
    }

    /// The genesis.json round-trip (write → read) historically shifted the embedded
    /// class hashes, leaving a preloaded Controller class unusable by its canonical
    /// hash and forcing a runtime re-declare. Because the class is now declared via a
    /// genesis tx — whose hash is recomputed from the artifact, not the (shifted) map
    /// key — the round-tripped genesis still declares the Controller at its canonical
    /// hash.
    #[test]
    fn preloaded_class_declared_at_canonical_hash_after_roundtrip() {
        use katana_contracts::controller::ControllerLatest;
        use katana_genesis::json::GenesisJson;
        use katana_primitives::transaction::{DeclareTx, ExecutableTx};

        let mut chain_spec = chain_spec(1, true);
        chain_spec
            .genesis
            .classes
            .insert(ControllerLatest::HASH, ControllerLatest::CLASS.clone().into());

        // Round-trip the genesis through its on-disk JSON form (the step that used to
        // shift the embedded class hashes).
        let json = GenesisJson::try_from(chain_spec.genesis.clone()).unwrap();
        chain_spec.genesis = Genesis::try_from(json).unwrap();

        let txs = GenesisTransactionsBuilder::new(&chain_spec).build();
        let declared: Vec<ClassHash> = txs
            .iter()
            .filter_map(|tx| match &tx.transaction {
                ExecutableTx::Declare(d) => Some(match &d.transaction {
                    DeclareTx::V0(t) => t.class_hash,
                    DeclareTx::V1(t) => t.class_hash,
                    DeclareTx::V2(t) => t.class_hash,
                    DeclareTx::V3(t) => t.class_hash,
                }),
                _ => None,
            })
            .collect();

        assert!(
            declared.contains(&ControllerLatest::HASH),
            "controller class must declare at its canonical hash even after a genesis.json \
             round-trip"
        );
    }

    #[rstest::rstest]
    #[case::with_balance(true)]
    #[case::no_balance(false)]
    fn predeployed_acccounts(#[case] with_balance: bool) {
        fn inner(n_accounts: usize, with_balance: bool) {
            let mut chain_spec = chain_spec(0, with_balance);

            // add non-dev allocations
            for i in 0..n_accounts {
                const CLASS_HASH: ClassHash = contracts::Account::HASH;
                let salt = Felt::from(i);
                let pk = Felt::from(1337);

                let mut account = GenesisAccount::new_with_salt(pk, CLASS_HASH, salt);

                if with_balance {
                    account.balance = Some(U256::from(DEFAULT_PREFUNDED_ACCOUNT_BALANCE));
                }

                chain_spec.genesis.extend_allocations([(
                    account.address(),
                    GenesisAllocation::Account(GenesisAccountAlloc::Account(account)),
                )]);
            }

            let mut transactions = GenesisTransactionsBuilder::new(&chain_spec).build();

            // Skip the prefix txs (master account declare/deploy, UDC + legacy UDC declare/deploy,
            // Account class declare) so we're left with just the per-account work. The STRK fee
            // token is pre-allocated to state by ChainSpec::state_updates rather than declared and
            // deployed here, so the prefix is 7 txs instead of the original 9.
            let account_transactions = &transactions.split_off(7);

            if with_balance {
                assert_eq!(account_transactions.len(), n_accounts * 2);
                for txs in account_transactions.chunks(2) {
                    assert_eq!(txs[0].transaction.r#type(), TxType::Invoke); // deploy
                    assert_eq!(txs[1].transaction.r#type(), TxType::Invoke); // transfer
                }
            } else {
                assert_eq!(account_transactions.len(), n_accounts);
                for txs in account_transactions.chunks(2) {
                    assert_eq!(txs[0].transaction.r#type(), TxType::Invoke); // deploy
                }
            }
        }

        for i in 0..10 {
            inner(i, with_balance);
        }
    }

    #[rstest::rstest]
    #[case::with_balance(true)]
    #[case::no_balance(false)]
    fn dev_predeployed_acccounts(#[case] with_balance: bool) {
        fn inner(n_accounts: u16, with_balance: bool) {
            let chain_spec = chain_spec(n_accounts, with_balance);
            let mut transactions = GenesisTransactionsBuilder::new(&chain_spec).build();

            // Skip the prefix txs (see `predeployed_acccounts` for the rationale).
            let account_transactions = &transactions.split_off(7);

            if with_balance {
                assert_eq!(account_transactions.len(), n_accounts as usize * 2);
                for txs in account_transactions.chunks(2) {
                    assert_eq!(txs[0].transaction.r#type(), TxType::DeployAccount);
                    assert_eq!(txs[1].transaction.r#type(), TxType::Invoke); // transfer
                }
            } else {
                assert_eq!(account_transactions.len(), n_accounts as usize);
                for txs in account_transactions.chunks(2) {
                    assert_eq!(txs[0].transaction.r#type(), TxType::DeployAccount);
                }
            }
        }

        for i in 0..10 {
            inner(i, with_balance);
        }
    }
}
