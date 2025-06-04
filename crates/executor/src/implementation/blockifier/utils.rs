use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;
use std::sync::Arc;

use blockifier::blockifier_versioned_constants::VersionedConstants;
use blockifier::bouncer::{Bouncer, BouncerConfig};
use blockifier::context::{BlockContext, ChainInfo, FeeTokenAddresses};
use blockifier::execution::contract_class::{
    CompiledClassV0, CompiledClassV1, RunnableCompiledClass,
};
use blockifier::fee::fee_utils::get_fee_by_gas_vector;
use blockifier::state::cached_state::{self, TransactionalState};
use blockifier::state::state_api::{StateReader, UpdatableState};
use blockifier::transaction::account_transaction::{
    AccountTransaction, ExecutionFlags as BlockifierExecutionFlags,
};
use blockifier::transaction::objects::{HasRelatedFeeType, TransactionExecutionInfo};
use blockifier::transaction::transaction_execution::Transaction;
use blockifier::transaction::transactions::ExecutableTransaction;
use cairo_vm::types::errors::program_errors::ProgramError;
use katana_primitives::chain::NamedChainId;
use katana_primitives::env::{BlockEnv, CfgEnv};
use katana_primitives::fee::{FeeInfo, PriceUnit, ResourceBoundsMapping};
use katana_primitives::state::{StateUpdates, StateUpdatesWithClasses};
use katana_primitives::transaction::{
    DeclareTx, DeployAccountTx, ExecutableTx, ExecutableTxWithHash, InvokeTx,
};
use katana_primitives::{class, fee};
use katana_provider::traits::contract::ContractClassProvider;
use starknet::core::utils::parse_cairo_short_string;
use starknet_api::block::{
    BlockInfo, BlockNumber, BlockTimestamp, FeeType, GasPriceVector, GasPrices, NonzeroGasPrice,
    StarknetVersion,
};
use starknet_api::contract_class::{ClassInfo, SierraVersion};
use starknet_api::core::{
    self, ChainId, ClassHash, CompiledClassHash, ContractAddress, EntryPointSelector, Nonce,
};
use starknet_api::data_availability::DataAvailabilityMode;
use starknet_api::executable_transaction::{
    DeclareTransaction, DeployAccountTransaction, InvokeTransaction, L1HandlerTransaction,
};
use starknet_api::transaction::fields::{
    AccountDeploymentData, AllResourceBounds, Calldata, ContractAddressSalt, Fee, PaymasterData,
    ResourceBounds, Tip, TransactionSignature, ValidResourceBounds,
};
use starknet_api::transaction::{
    DeclareTransaction as ApiDeclareTransaction, DeclareTransactionV0V1, DeclareTransactionV2,
    DeclareTransactionV3, DeployAccountTransaction as ApiDeployAccountTransaction,
    DeployAccountTransactionV1, DeployAccountTransactionV3,
    InvokeTransaction as ApiInvokeTransaction, InvokeTransactionV3, TransactionHash,
    TransactionVersion,
};

use super::state::CachedState;
use crate::abstraction::ExecutionFlags;
use crate::utils::build_receipt;
use crate::{ExecutionError, ExecutionResult, ExecutorResult};

pub fn transact<S: StateReader>(
    state: &mut cached_state::CachedState<S>,
    block_context: &BlockContext,
    simulation_flags: &ExecutionFlags,
    tx: ExecutableTxWithHash,
    bouncer: Option<&mut Bouncer>,
) -> ExecutorResult<ExecutionResult> {
    fn transact_inner<U: UpdatableState>(
        state: &mut U,
        block_context: &BlockContext,
        tx: Transaction,
    ) -> Result<(TransactionExecutionInfo, FeeInfo), ExecutionError> {
        let execution_info = match &tx {
            Transaction::Account(tx) => tx.execute(state, block_context),
            Transaction::L1Handler(tx) => tx.execute(state, block_context),
        }?;

        let fee_type = get_fee_type_from_tx(&tx);

        // There are a few case where the `actual_fee` field of the transaction info is not set
        // where the fee is skipped and thus not charged for the transaction (e.g. when the
        // `skip_fee_transfer` is explicitly set, or when the transaction `max_fee` is set to 0). In
        // these cases, we still want to calculate the fee.
        let overall_fee = if execution_info.receipt.fee == Fee(0) {
            let tip = match &tx {
                Transaction::Account(tx) if tx.version() == TransactionVersion::THREE => tx.tip(),
                _ => Tip::ZERO,
            };

            get_fee_by_gas_vector(
                block_context.block_info(),
                execution_info.receipt.gas,
                &fee_type,
                tip,
            )
        } else {
            execution_info.receipt.fee
        };

        let prices = &block_context.block_info().gas_prices;

        let fee = match fee_type {
            FeeType::Eth => {
                let unit = PriceUnit::Wei;
                let overall_fee = overall_fee.0;
                let l1_gas_price = prices.eth_gas_prices.l1_gas_price.get().0;
                let l2_gas_price = prices.eth_gas_prices.l2_gas_price.get().0;
                let l1_data_gas_price = prices.eth_gas_prices.l1_data_gas_price.get().0;
                FeeInfo { unit, overall_fee, l1_gas_price, l2_gas_price, l1_data_gas_price }
            }
            FeeType::Strk => {
                let unit = PriceUnit::Fri;
                let overall_fee = overall_fee.0;
                let l1_gas_price = prices.strk_gas_prices.l1_gas_price.get().0;
                let l2_gas_price = prices.strk_gas_prices.l2_gas_price.get().0;
                let l1_data_gas_price = prices.strk_gas_prices.l1_data_gas_price.get().0;
                FeeInfo { unit, overall_fee, l1_gas_price, l2_gas_price, l1_data_gas_price }
            }
        };

        Ok((execution_info, fee))
    }

    let transaction = to_executor_tx(tx.clone(), simulation_flags.clone());
    let mut tx_state = TransactionalState::create_transactional(state);
    let result = transact_inner(&mut tx_state, block_context, transaction);

    match result {
        Ok((info, fee)) => {
            if let Some(bouncer) = bouncer {
                let tx_state_changes_keys = tx_state.to_state_diff().unwrap().state_maps.keys();
                let versioned_constants = block_context.versioned_constants();

                bouncer.try_update(
                    &tx_state,
                    &tx_state_changes_keys,
                    &info.summarize(versioned_constants),
                    &info.receipt.resources,
                    versioned_constants,
                )?;
            }

            tx_state.commit();

            let receipt = build_receipt(tx.tx_ref(), fee, &info);
            Ok(ExecutionResult::new_success(receipt, info))
        }

        Err(e) => {
            tx_state.commit();
            Ok(ExecutionResult::new_failed(e))
        }
    }
}

pub fn to_executor_tx(mut tx: ExecutableTxWithHash, mut flags: ExecutionFlags) -> Transaction {
    use starknet_api::executable_transaction::AccountTransaction as ExecTx;

    let hash = tx.hash;

    // We only do this if we're running in fee enabled mode. If fee is already disabled, then
    // there's no need to do anything.
    if flags.fee() {
        // Disable fee charge if the total tx execution gas is zero.
        //
        // Max fee == 0 for legacy txs, or the total resource bounds == zero for V3 transactions.
        //
        // This is to support genesis transactions where the fee token is not yet deployed.
        flags = flags.with_fee(!skip_fee_on_zero_gas(&tx));
    }

    if !flags.fee() {
        set_max_initial_sierra_gas(&mut tx);
    }

    match tx.transaction {
        ExecutableTx::Invoke(tx) => match tx {
            InvokeTx::V0(tx) => {
                let calldata = tx.calldata;
                let signature = tx.signature;

                let tx = InvokeTransaction {
                    tx: ApiInvokeTransaction::V0(starknet_api::transaction::InvokeTransactionV0 {
                        entry_point_selector: EntryPointSelector(tx.entry_point_selector),
                        contract_address: to_blk_address(tx.contract_address),
                        signature: TransactionSignature(signature),
                        calldata: Calldata(Arc::new(calldata)),
                        max_fee: Fee(tx.max_fee),
                    }),
                    tx_hash: TransactionHash(hash),
                };

                Transaction::Account(AccountTransaction {
                    tx: ExecTx::Invoke(tx),
                    execution_flags: flags.into(),
                })
            }

            InvokeTx::V1(tx) => {
                let calldata = tx.calldata;
                let signature = tx.signature;

                let tx = InvokeTransaction {
                    tx: ApiInvokeTransaction::V1(starknet_api::transaction::InvokeTransactionV1 {
                        max_fee: Fee(tx.max_fee),
                        nonce: Nonce(tx.nonce),
                        sender_address: to_blk_address(tx.sender_address),
                        signature: TransactionSignature(signature),
                        calldata: Calldata(Arc::new(calldata)),
                    }),
                    tx_hash: TransactionHash(hash),
                };

                Transaction::Account(AccountTransaction {
                    tx: ExecTx::Invoke(tx),
                    execution_flags: flags.into(),
                })
            }

            InvokeTx::V3(tx) => {
                let calldata = tx.calldata;
                let signature = tx.signature;

                let paymaster_data = tx.paymaster_data;
                let account_deploy_data = tx.account_deployment_data;
                let fee_data_availability_mode = to_api_da_mode(tx.fee_data_availability_mode);
                let nonce_data_availability_mode = to_api_da_mode(tx.nonce_data_availability_mode);

                let tx = InvokeTransaction {
                    tx: ApiInvokeTransaction::V3(InvokeTransactionV3 {
                        tip: Tip(tx.tip),
                        nonce: Nonce(tx.nonce),
                        sender_address: to_blk_address(tx.sender_address),
                        signature: TransactionSignature(signature),
                        calldata: Calldata(Arc::new(calldata)),
                        paymaster_data: PaymasterData(paymaster_data),
                        account_deployment_data: AccountDeploymentData(account_deploy_data),
                        fee_data_availability_mode,
                        nonce_data_availability_mode,
                        resource_bounds: to_api_resource_bounds(tx.resource_bounds),
                    }),

                    tx_hash: TransactionHash(hash),
                };

                Transaction::Account(AccountTransaction {
                    tx: ExecTx::Invoke(tx),
                    execution_flags: flags.into(),
                })
            }
        },

        ExecutableTx::DeployAccount(tx) => match tx {
            DeployAccountTx::V1(tx) => {
                let calldata = tx.constructor_calldata;
                let signature = tx.signature;
                let salt = ContractAddressSalt(tx.contract_address_salt);

                let tx = DeployAccountTransaction {
                    contract_address: to_blk_address(tx.contract_address),
                    tx: ApiDeployAccountTransaction::V1(DeployAccountTransactionV1 {
                        max_fee: Fee(tx.max_fee),
                        nonce: Nonce(tx.nonce),
                        signature: TransactionSignature(signature),
                        class_hash: ClassHash(tx.class_hash),
                        constructor_calldata: Calldata(Arc::new(calldata)),
                        contract_address_salt: salt,
                    }),
                    tx_hash: TransactionHash(hash),
                };

                Transaction::Account(AccountTransaction {
                    tx: ExecTx::DeployAccount(tx),
                    execution_flags: flags.into(),
                })
            }

            DeployAccountTx::V3(tx) => {
                let calldata = tx.constructor_calldata;
                let signature = tx.signature;
                let salt = ContractAddressSalt(tx.contract_address_salt);

                let paymaster_data = tx.paymaster_data;
                let fee_data_availability_mode = to_api_da_mode(tx.fee_data_availability_mode);
                let nonce_data_availability_mode = to_api_da_mode(tx.nonce_data_availability_mode);

                let tx = DeployAccountTransaction {
                    contract_address: to_blk_address(tx.contract_address),
                    tx: ApiDeployAccountTransaction::V3(DeployAccountTransactionV3 {
                        tip: Tip(tx.tip),
                        nonce: Nonce(tx.nonce),
                        signature: TransactionSignature(signature),
                        class_hash: ClassHash(tx.class_hash),
                        constructor_calldata: Calldata(Arc::new(calldata)),
                        contract_address_salt: salt,
                        paymaster_data: PaymasterData(paymaster_data),
                        fee_data_availability_mode,
                        nonce_data_availability_mode,
                        resource_bounds: to_api_resource_bounds(tx.resource_bounds),
                    }),
                    tx_hash: TransactionHash(hash),
                };

                Transaction::Account(AccountTransaction {
                    tx: ExecTx::DeployAccount(tx),
                    execution_flags: flags.into(),
                })
            }
        },

        ExecutableTx::Declare(tx) => {
            let compiled = tx.class.as_ref().clone().compile().expect("failed to compile");

            let tx = match tx.transaction {
                DeclareTx::V0(tx) => ApiDeclareTransaction::V0(DeclareTransactionV0V1 {
                    max_fee: Fee(tx.max_fee),
                    nonce: Nonce::default(),
                    sender_address: to_blk_address(tx.sender_address),
                    signature: TransactionSignature(tx.signature),
                    class_hash: ClassHash(tx.class_hash),
                }),

                DeclareTx::V1(tx) => ApiDeclareTransaction::V1(DeclareTransactionV0V1 {
                    max_fee: Fee(tx.max_fee),
                    nonce: Nonce(tx.nonce),
                    sender_address: to_blk_address(tx.sender_address),
                    signature: TransactionSignature(tx.signature),
                    class_hash: ClassHash(tx.class_hash),
                }),

                DeclareTx::V2(tx) => {
                    let signature = tx.signature;

                    ApiDeclareTransaction::V2(DeclareTransactionV2 {
                        max_fee: Fee(tx.max_fee),
                        nonce: Nonce(tx.nonce),
                        sender_address: to_blk_address(tx.sender_address),
                        signature: TransactionSignature(signature),
                        class_hash: ClassHash(tx.class_hash),
                        compiled_class_hash: CompiledClassHash(tx.compiled_class_hash),
                    })
                }

                DeclareTx::V3(tx) => {
                    let signature = tx.signature;

                    let paymaster_data = tx.paymaster_data;
                    let fee_data_availability_mode = to_api_da_mode(tx.fee_data_availability_mode);
                    let nonce_data_availability_mode =
                        to_api_da_mode(tx.nonce_data_availability_mode);
                    let account_deploy_data = tx.account_deployment_data;

                    ApiDeclareTransaction::V3(DeclareTransactionV3 {
                        tip: Tip(tx.tip),
                        nonce: Nonce(tx.nonce),
                        sender_address: to_blk_address(tx.sender_address),
                        signature: TransactionSignature(signature),
                        class_hash: ClassHash(tx.class_hash),
                        account_deployment_data: AccountDeploymentData(account_deploy_data),
                        compiled_class_hash: CompiledClassHash(tx.compiled_class_hash),
                        paymaster_data: PaymasterData(paymaster_data),
                        fee_data_availability_mode,
                        nonce_data_availability_mode,
                        resource_bounds: to_api_resource_bounds(tx.resource_bounds),
                    })
                }
            };

            let tx_hash = TransactionHash(hash);
            let class_info = to_class_info(compiled).unwrap();
            Transaction::Account(AccountTransaction {
                tx: ExecTx::Declare(DeclareTransaction { class_info, tx_hash, tx }),
                execution_flags: flags.into(),
            })
        }

        ExecutableTx::L1Handler(tx) => Transaction::L1Handler(L1HandlerTransaction {
            paid_fee_on_l1: Fee(tx.paid_fee_on_l1),
            tx: starknet_api::transaction::L1HandlerTransaction {
                nonce: core::Nonce(tx.nonce),
                calldata: Calldata(Arc::new(tx.calldata)),
                version: TransactionVersion(1u128.into()),
                contract_address: to_blk_address(tx.contract_address),
                entry_point_selector: core::EntryPointSelector(tx.entry_point_selector),
            },
            tx_hash: TransactionHash(hash),
        }),
    }
}

// The reason why we do this, ie bcs in blockifier, if all the ValidResourceBounds::AllResources are
// given, then it will use the max l2 gas as the initial gas for the transaction execution.
// So when we do fee estimates, usually the resource bounds are all set to zero. so executing them
// without calling this function will result in an out of gas error - because the initial gas
// will end up being zero.
//
// Similarly, if the node is running in fee disabled mode, we want to completely ignore the resource
// bounds specified in the tx. So we artifically set the max initial gas in case the tx resource
// bounds isn't zero or not enough to run the tx to completion, bcs then blockifier will use the max
// l2 gas as the initial gas for the transaction execution.
fn set_max_initial_sierra_gas(tx: &mut ExecutableTxWithHash) {
    match &mut tx.transaction {
        ExecutableTx::Invoke(InvokeTx::V3(ref mut tx)) => {
            tx.resource_bounds.l2_gas.max_amount = u64::MAX;
        }
        ExecutableTx::DeployAccount(DeployAccountTx::V3(ref mut tx)) => {
            tx.resource_bounds.l2_gas.max_amount = u64::MAX;
        }
        ExecutableTx::Declare(tx) => match tx.transaction {
            DeclareTx::V3(ref mut tx) => {
                tx.resource_bounds.l2_gas.max_amount = u64::MAX;
            }
            _ => {}
        },
        _ => {}
    }
}

/// Create a block context from the chain environment values.
pub fn block_context_from_envs(block_env: &BlockEnv, cfg_env: &CfgEnv) -> BlockContext {
    let fee_token_addresses = FeeTokenAddresses {
        eth_fee_token_address: to_blk_address(cfg_env.fee_token_addresses.eth),
        strk_fee_token_address: to_blk_address(cfg_env.fee_token_addresses.strk),
    };

    let eth_l1_gas_price = NonzeroGasPrice::new(block_env.l1_gas_prices.eth.get().into())
        .unwrap_or(NonzeroGasPrice::MIN);
    let strk_l1_gas_price = NonzeroGasPrice::new(block_env.l1_gas_prices.strk.get().into())
        .unwrap_or(NonzeroGasPrice::MIN);
    let eth_l1_data_gas_price = NonzeroGasPrice::new(block_env.l1_data_gas_prices.eth.get().into())
        .unwrap_or(NonzeroGasPrice::MIN);
    let strk_l1_data_gas_price =
        NonzeroGasPrice::new(block_env.l1_data_gas_prices.strk.get().into())
            .unwrap_or(NonzeroGasPrice::MIN);

    let gas_prices = GasPrices {
        eth_gas_prices: GasPriceVector {
            l1_gas_price: eth_l1_gas_price,
            l1_data_gas_price: eth_l1_data_gas_price,
            // TODO: update to use the correct value
            l2_gas_price: eth_l1_gas_price,
        },
        strk_gas_prices: GasPriceVector {
            l1_gas_price: strk_l1_gas_price,
            l1_data_gas_price: strk_l1_data_gas_price,
            // TODO: update to use the correct value
            l2_gas_price: strk_l1_gas_price,
        },
    };

    let block_info = BlockInfo {
        block_number: BlockNumber(block_env.number),
        block_timestamp: BlockTimestamp(block_env.timestamp),
        sequencer_address: to_blk_address(block_env.sequencer_address),
        gas_prices,
        use_kzg_da: false,
    };

    let chain_info = ChainInfo { fee_token_addresses, chain_id: to_blk_chain_id(cfg_env.chain_id) };

    // IMPORTANT:
    //
    // The versioned constants that we use here must match the version that is used in `snos`.
    // Otherwise, there might be a mismatch between the calculated fees.
    //
    // The version of `snos` we're using is still limited up to Starknet version `0.13.3`.
    const SN_VERSION: StarknetVersion = StarknetVersion::V0_13_4;
    let mut versioned_constants = VersionedConstants::get(&SN_VERSION).unwrap().clone();

    // NOTE:
    // These overrides would potentially make the `snos` run be invalid as it doesn't know about the
    // new overridden values.
    versioned_constants.max_recursion_depth = cfg_env.max_recursion_depth;
    versioned_constants.validate_max_n_steps = cfg_env.validate_max_n_steps;
    versioned_constants.invoke_tx_max_n_steps = cfg_env.invoke_tx_max_n_steps;

    BlockContext::new(block_info, chain_info, versioned_constants, BouncerConfig::max())
}

pub(super) fn state_update_from_cached_state(state: &CachedState<'_>) -> StateUpdatesWithClasses {
    let state_diff = state.inner.lock().cached_state.to_state_diff().unwrap();

    let mut declared_contract_classes: BTreeMap<
        katana_primitives::class::ClassHash,
        katana_primitives::class::ContractClass,
    > = BTreeMap::new();

    let mut declared_classes = BTreeMap::new();
    let mut deprecated_declared_classes = BTreeSet::new();

    // TODO: Legacy class shouldn't have a compiled class hash. This is a hack we added
    // in our fork of `blockifier. Check if it's possible to remove it now.
    for (class_hash, compiled_hash) in state_diff.state_maps.compiled_class_hashes {
        let hash = class_hash.0;
        let class = state.class(hash).unwrap().expect("must exist if declared");

        if class.is_legacy() {
            deprecated_declared_classes.insert(hash);
        } else {
            declared_classes.insert(hash, compiled_hash.0);
        }

        declared_contract_classes.insert(hash, class);
    }

    let nonce_updates =
        state_diff
            .state_maps
            .nonces
            .into_iter()
            .map(|(key, value)| (to_address(key), value.0))
            .collect::<BTreeMap<
                katana_primitives::contract::ContractAddress,
                katana_primitives::contract::Nonce,
            >>();

    let storage_updates = state_diff.state_maps.storage.into_iter().fold(
        BTreeMap::new(),
        |mut storage, ((addr, key), value)| {
            let entry: &mut BTreeMap<
                katana_primitives::contract::StorageKey,
                katana_primitives::contract::StorageValue,
            > = storage.entry(to_address(addr)).or_default();
            entry.insert(*key.0.key(), value);
            storage
        },
    );

    let deployed_contracts =
        state_diff
            .state_maps
            .class_hashes
            .into_iter()
            .map(|(key, value)| (to_address(key), value.0))
            .collect::<BTreeMap<
                katana_primitives::contract::ContractAddress,
                katana_primitives::class::ClassHash,
            >>();

    StateUpdatesWithClasses {
        classes: declared_contract_classes,
        state_updates: StateUpdates {
            nonce_updates,
            storage_updates,
            declared_classes,
            deployed_contracts,
            deprecated_declared_classes,
            replaced_classes: BTreeMap::default(),
        },
    }
}

fn to_api_da_mode(mode: katana_primitives::da::DataAvailabilityMode) -> DataAvailabilityMode {
    match mode {
        katana_primitives::da::DataAvailabilityMode::L1 => DataAvailabilityMode::L1,
        katana_primitives::da::DataAvailabilityMode::L2 => DataAvailabilityMode::L2,
    }
}

// The protocol version we want to support depends on the returned `ValidResourceBounds`. Returning
// the wrong variant without the right values will result in execution error.
//
// Ref: https://community.starknet.io/t/starknet-v0-13-1-pre-release-notes/113664#sdkswallets-how-to-use-the-new-fee-estimates-7
fn to_api_resource_bounds(resource_bounds: fee::ResourceBoundsMapping) -> ValidResourceBounds {
    // ValidResourceBounds::L1Gas(ResourceBounds {
    //     max_amount: resource_bounds.l1_gas.max_amount.into(),
    //     max_price_per_unit: resource_bounds.l1_gas.max_price_per_unit.into(),
    // })

    let l1_gas = ResourceBounds {
        max_amount: resource_bounds.l1_gas.max_amount.into(),
        max_price_per_unit: resource_bounds.l1_gas.max_price_per_unit.into(),
    };

    let l2_gas = ResourceBounds {
        max_amount: resource_bounds.l2_gas.max_amount.into(),
        max_price_per_unit: resource_bounds.l2_gas.max_price_per_unit.into(),
    };

    let l1_data_gas = ResourceBounds {
        max_amount: resource_bounds.l1_data_gas.max_amount.into(),
        max_price_per_unit: resource_bounds.l1_data_gas.max_price_per_unit.into(),
    };

    ValidResourceBounds::AllResources(AllResourceBounds { l1_gas, l2_gas, l1_data_gas })
}

/// Get the fee type of a transaction. The fee type determines the token used to pay for the
/// transaction.
fn get_fee_type_from_tx(transaction: &Transaction) -> FeeType {
    match transaction {
        Transaction::Account(tx) => tx.fee_type(),
        Transaction::L1Handler(tx) => tx.fee_type(),
    }
}

pub fn to_blk_address(address: katana_primitives::contract::ContractAddress) -> ContractAddress {
    address.0.try_into().expect("valid address")
}

pub fn to_address(address: ContractAddress) -> katana_primitives::contract::ContractAddress {
    katana_primitives::contract::ContractAddress(*address.0.key())
}

pub fn to_blk_chain_id(chain_id: katana_primitives::chain::ChainId) -> ChainId {
    match chain_id {
        katana_primitives::chain::ChainId::Named(NamedChainId::Mainnet) => ChainId::Mainnet,
        katana_primitives::chain::ChainId::Named(NamedChainId::Sepolia) => ChainId::Sepolia,
        katana_primitives::chain::ChainId::Named(named) => ChainId::Other(named.to_string()),
        katana_primitives::chain::ChainId::Id(id) => {
            let id = parse_cairo_short_string(&id).expect("valid cairo string");
            ChainId::Other(id)
        }
    }
}

pub fn to_class_info(class: class::CompiledClass) -> Result<ClassInfo, ProgramError> {
    use starknet_api::contract_class::ContractClass;

    // TODO: @kariy not sure of the variant that must be used in this case. Should we change the
    // return type to include this case of error for contract class conversions?
    match class {
        class::CompiledClass::Legacy(legacy) => {
            // For cairo 0, the sierra_program_length must be 0.
            Ok(ClassInfo::new(&ContractClass::V0(legacy), 0, 0, SierraVersion::DEPRECATED).unwrap())
        }

        class::CompiledClass::Class(sierra) => {
            // NOTE:
            //
            // Right now, we're using dummy values for the sierra class info (ie
            // sierra_program_length, and abi_length). This value affects the fee
            // calculation so we should use the correct values based on the sierra class itself.
            //
            // Make sure these values are the same over on `snos` when it re-executes the
            // transactions as otherwise the fees would be different.

            let version = SierraVersion::from_str(&sierra.compiler_version).unwrap();
            let class = ContractClass::V1((sierra, version.clone()));
            let sierra_program_length = 1;
            let abi_length = 0;

            Ok(ClassInfo::new(&class, sierra_program_length, abi_length, version).unwrap())
        }
    }
}

/// Convert katana-primitives compiled class to blockfiier's contract class.
pub fn to_class(class: class::CompiledClass) -> Result<RunnableCompiledClass, ProgramError> {
    // TODO: @kariy not sure of the variant that must be used in this case. Should we change the
    // return type to include this case of error for contract class conversions?
    match class {
        class::CompiledClass::Legacy(class) => {
            Ok(RunnableCompiledClass::V0(CompiledClassV0::try_from(class)?))
        }
        class::CompiledClass::Class(casm) => {
            let version = SierraVersion::from_str(&casm.compiler_version).unwrap();
            let versioned_casm = (casm, version);
            Ok(RunnableCompiledClass::V1(CompiledClassV1::try_from(versioned_casm)?))
        }
    }
}

impl From<ExecutionFlags> for BlockifierExecutionFlags {
    fn from(value: ExecutionFlags) -> Self {
        Self {
            only_query: false,
            charge_fee: value.fee(),
            validate: value.account_validation(),
            strict_nonce_check: value.nonce_check(),
        }
    }
}

/// Check if the tx max fee is 0, if yes, this function returns `true` - signalling that the
/// transaction should be executed without fee checks.
///
/// This is to support the old behaviour of blockifier where tx with 0 max fee can still be
/// executed. This flow is not integrated in the transaction execution flow anymore in the new
/// blockifer rev. So, we handle it here manually to mimic that behaviour.
/// Reference: https://github.com/dojoengine/sequencer/blob/07f473f9385f1bce4cbd7d0d64b5396f6784bbf1/crates/blockifier/src/transaction/objects.rs#L103-L113
///
/// Transaction with 0 max fee is mainly used for genesis block.
fn skip_fee_on_zero_gas(tx: &ExecutableTxWithHash) -> bool {
    match &tx.transaction {
        ExecutableTx::Invoke(tx_inner) => match tx_inner {
            InvokeTx::V0(tx) => tx.max_fee == 0,
            InvokeTx::V1(tx) => tx.max_fee == 0,
            InvokeTx::V3(tx) => is_zero_resource_bounds(&tx.resource_bounds),
        },
        ExecutableTx::DeployAccount(tx_inner) => match tx_inner {
            DeployAccountTx::V1(tx) => tx.max_fee == 0,
            DeployAccountTx::V3(tx) => is_zero_resource_bounds(&tx.resource_bounds),
        },
        ExecutableTx::Declare(tx_inner) => match &tx_inner.transaction {
            DeclareTx::V0(tx) => tx.max_fee == 0,
            DeclareTx::V1(tx) => tx.max_fee == 0,
            DeclareTx::V2(tx) => tx.max_fee == 0,
            DeclareTx::V3(tx) => is_zero_resource_bounds(&tx.resource_bounds),
        },
        ExecutableTx::L1Handler(..) => true,
    }
}

pub fn is_zero_resource_bounds(resource_bounds: &ResourceBoundsMapping) -> bool {
    let l1_bounds = &resource_bounds.l1_gas;
    let l2_bounds = &resource_bounds.l2_gas;
    let l1_data_bounds = &resource_bounds.l1_data_gas;

    let l1_max_amount: u128 = l1_bounds.max_amount.into();
    let l2_max_amount: u128 = l2_bounds.max_amount.into();
    let l1_data_max_amount: u128 = l1_data_bounds.max_amount.into();

    ((l1_max_amount * l1_bounds.max_price_per_unit)
        + (l2_max_amount * l2_bounds.max_price_per_unit)
        + (l1_data_max_amount * l1_data_bounds.max_price_per_unit))
        == 0
}

#[cfg(test)]
mod tests {

    use katana_primitives::Felt;
    use starknet_api::felt;

    use super::*;

    #[test]
    fn convert_chain_id() {
        let katana_mainnet = katana_primitives::chain::ChainId::MAINNET;
        let katana_sepolia = katana_primitives::chain::ChainId::SEPOLIA;
        let katana_id = katana_primitives::chain::ChainId::Id(felt!("0x1337"));

        let blockifier_mainnet = to_blk_chain_id(katana_mainnet);
        let blockifier_sepolia = to_blk_chain_id(katana_sepolia);
        let blockifier_id = to_blk_chain_id(katana_id);

        assert_eq!(blockifier_mainnet, ChainId::Mainnet);
        assert_eq!(blockifier_sepolia, ChainId::Sepolia);
        assert_eq!(blockifier_id.as_hex(), katana_id.to_string());
    }

    /// Test to ensure that when Blockifier pass the chain id to the contract ( thru a syscall eg,
    /// get_tx_inbox().unbox().chain_id ), the value is exactly the same as Katana chain id.
    ///
    /// Issue: <https://github.com/dojoengine/dojo/issues/1595>
    #[test]
    fn blockifier_chain_id_invariant() {
        let id = felt!("0x1337");

        let katana_id = katana_primitives::chain::ChainId::Id(id);
        let blockifier_id = to_blk_chain_id(katana_id);

        // Mimic how blockifier convert from ChainId to FieldElement.
        //
        // This is how blockifier pass the chain id to the contract through a syscall.
        // https://github.com/dojoengine/blockifier/blob/f2246ce2862d043e4efe2ecf149a4cb7bee689cd/crates/blockifier/src/execution/syscalls/hint_processor.rs#L600-L602
        let actual_id = Felt::from_hex(blockifier_id.as_hex().as_str()).unwrap();

        assert_eq!(actual_id, id)
    }
}
