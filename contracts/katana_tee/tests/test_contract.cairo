use katana_tee::{IKatanaTeeDispatcher, IKatanaTeeDispatcherTrait};
use snforge_std::{ContractClassTrait, DeclareResultTrait, declare};
use starknet::ContractAddress;

fn deploy_contract(
    registry_address: ContractAddress, storage_commitment_registry: ContractAddress,
) -> ContractAddress {
    let contract = declare("KatanaTee").unwrap().contract_class();

    let mut calldata: Array<felt252> = array![];
    calldata.append(registry_address.into());
    calldata.append(storage_commitment_registry.into());

    let (contract_address, _) = contract.deploy(@calldata).unwrap();
    contract_address
}

#[test]
fn test_get_registry_address() {
    let registry_address: ContractAddress = 0x1234.try_into().unwrap();
    let storage_commitment_registry: ContractAddress = 0x5678.try_into().unwrap();

    let contract_address = deploy_contract(registry_address, storage_commitment_registry);
    let dispatcher = IKatanaTeeDispatcher { contract_address };

    let returned_registry = dispatcher.get_registry_address();
    assert(returned_registry == registry_address, 'Wrong registry address');
}
