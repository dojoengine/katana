use amd_tee_registry::cert_cache::CertCacheComponent::{
    ICertCacheDispatcher, ICertCacheDispatcherTrait,
};
use amd_tee_registry::tee_types::ProcessorType;
use snforge_std::{ContractClassTrait, DeclareResultTrait, declare};
use starknet::ContractAddress;

fn deploy_contract() -> ContractAddress {
    let contract = declare("AMDTEERegistry").unwrap().contract_class();

    // Prepare constructor arguments
    let trusted_cert: u256 = 0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef;
    let root_cert: u256 = 0xabcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890;

    let verifier_class_hash: felt252 = 0x1;
    let sp1_program_id: u256 = 0x2;
    let max_time_diff: u64 = 3600;

    let mut calldata: Array<felt252> = array![];

    // verifier_class_hash
    calldata.append(verifier_class_hash);

    // sp1_program_id
    calldata.append(sp1_program_id.low.into());
    calldata.append(sp1_program_id.high.into());

    // max_time_diff
    calldata.append(max_time_diff.into());

    // trusted_certs array (1 element)
    calldata.append(1); // array length
    calldata.append(trusted_cert.low.into());
    calldata.append(trusted_cert.high.into());

    // processor_models array (1 element)
    calldata.append(1); // array length
    calldata.append(0); // ProcessorType::Milan

    // root_certs array (1 element)
    calldata.append(1); // array length
    calldata.append(root_cert.low.into());
    calldata.append(root_cert.high.into());

    // storage_commitment_proxy (0 = disabled)
    calldata.append(0);

    let (contract_address, _) = contract.deploy(@calldata).unwrap();
    contract_address
}

#[test]
fn test_is_trusted_intermediate_cert() {
    let contract_address = deploy_contract();
    let dispatcher = ICertCacheDispatcher { contract_address };

    let trusted_cert: u256 = 0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef;
    let untrusted_cert: u256 = 0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef;

    // The trusted cert should be marked as trusted
    assert(dispatcher.is_trusted_intermediate_cert(trusted_cert), 'Cert should be trusted');

    // An unknown cert should not be trusted
    assert(!dispatcher.is_trusted_intermediate_cert(untrusted_cert), 'Cert should not be trusted');
}

#[test]
fn test_get_root_cert() {
    let contract_address = deploy_contract();
    let dispatcher = ICertCacheDispatcher { contract_address };

    let expected_root_cert: u256 =
        0xabcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890;

    // Check that the root cert was set correctly for Milan processor
    let root_cert = dispatcher.get_root_cert(ProcessorType::Milan);
    assert(root_cert == expected_root_cert, 'Wrong root cert for Milan');

    // Check that other processor types have no root cert set (should be 0)
    let genoa_root = dispatcher.get_root_cert(ProcessorType::Genoa);
    assert(genoa_root == 0, 'Genoa should have no root cert');
}
