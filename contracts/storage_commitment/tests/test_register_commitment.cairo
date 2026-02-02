use snforge_std::{ContractClassTrait, DeclareResultTrait, declare};
use starknet::ContractAddress;
use storage_commitment::{IStorageCommitmentDispatcher, IStorageCommitmentDispatcherTrait};

fn deploy_storage_commitment() -> ContractAddress {
    let contract_class = declare("StorageCommitment").unwrap().contract_class();

    let mut calldata: Array<felt252> = ArrayTrait::new();
    let (contract_address, _) = contract_class.deploy(@calldata).unwrap();
    contract_address
}

#[test]
fn test_register_commitment() {
    let contract_address = deploy_storage_commitment();
    let dispatcher = IStorageCommitmentDispatcher { contract_address };

    let commitment: u256 = u256 {
        high: 0x11111111111111111111111111111111, low: 0x22222222222222222222222222222222,
    };

    let exists_before: bool = dispatcher.is_verified(commitment);
    assert(exists_before == false, 'Should not be registered');

    dispatcher.register_verified_commitment(commitment);

    let exists_after: bool = dispatcher.is_verified(commitment);
    assert(exists_after == true, 'Should be registered');
}

#[test]
fn test_multiple_commitments() {
    let contract_address = deploy_storage_commitment();
    let dispatcher = IStorageCommitmentDispatcher { contract_address };

    let mut commitments: Array<u256> = ArrayTrait::new();
    commitments.append(u256 { high: 1, low: 1 });
    commitments.append(u256 { high: 2, low: 2 });
    commitments.append(u256 { high: 3, low: 3 });

    let len = commitments.len();
    let mut i = 0;
    while i < len {
        let commitment: u256 = *commitments.at(i);
        let exists_before: bool = dispatcher.is_verified(commitment);
        assert(exists_before == false, 'Should not be registered');

        dispatcher.register_verified_commitment(commitment);

        let exists_after: bool = dispatcher.is_verified(commitment);
        assert(exists_after == true, 'Should be registered');
        i += 1;
    }
}
