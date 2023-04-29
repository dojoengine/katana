use anyhow::Result;
use blockifier::state::state_api::State;
use starknet_api::{
    core::{ClassHash, ContractAddress, PatriciaKey},
    hash::{StarkFelt, StarkHash},
    patricia_key, stark_felt,
};

use crate::{
    accounts::PredeployedAccounts,
    constants::{
        ACCOUNT_CONTRACT_CLASS_HASH, ACCOUNT_CONTRACT_PATH, ERC20_CONTRACT_CLASS_HASH,
        ERC20_CONTRACT_PATH, FEE_ERC20_CONTRACT_ADDRESS, UNIVERSAL_DEPLOYER_CLASS_HASH,
        UNIVERSAL_DEPLOYER_CONTRACT_ADDRESS, UNIVERSAL_DEPLOYER_CONTRACT_PATH,
    },
    util::get_contract_class,
};

pub struct KatanaDefaultState;

impl KatanaDefaultState {
    pub fn initialize_state(state: &mut dyn State) -> Result<()> {
        Self::deploy_fee_contract(state)?;
        Self::deploy_default_account_contract(state)?;
        Self::deploy_universal_deployer_contract(state)?;
        Self::deploy_rich_accounts(state, &PredeployedAccounts::default())?;
        Ok(())
    }

    fn deploy_fee_contract(state: &mut dyn State) -> Result<()> {
        let erc20_class_hash = ClassHash(stark_felt!(ERC20_CONTRACT_CLASS_HASH));

        state.set_contract_class(&erc20_class_hash, get_contract_class(ERC20_CONTRACT_PATH))?;
        state.set_class_hash_at(
            ContractAddress(patricia_key!(FEE_ERC20_CONTRACT_ADDRESS)),
            erc20_class_hash,
        )?;

        Ok(())
    }

    fn deploy_universal_deployer_contract(state: &mut dyn State) -> Result<()> {
        let universal_deployer_class_hash = ClassHash(stark_felt!(UNIVERSAL_DEPLOYER_CLASS_HASH));

        state.set_contract_class(
            &universal_deployer_class_hash,
            get_contract_class(UNIVERSAL_DEPLOYER_CONTRACT_PATH),
        )?;
        state.set_class_hash_at(
            ContractAddress(patricia_key!(UNIVERSAL_DEPLOYER_CONTRACT_ADDRESS)),
            universal_deployer_class_hash,
        )?;

        Ok(())
    }

    fn deploy_default_account_contract(state: &mut dyn State) -> Result<()> {
        let account_class_hash = ClassHash(stark_felt!(ACCOUNT_CONTRACT_CLASS_HASH));

        state.set_contract_class(
            &account_class_hash,
            get_contract_class(ACCOUNT_CONTRACT_PATH),
        )?;

        Ok(())
    }

    fn deploy_rich_accounts(state: &mut dyn State, accounts: &PredeployedAccounts) -> Result<()> {
        accounts.deploy_accounts(state)?;
        Ok(())
    }
}
