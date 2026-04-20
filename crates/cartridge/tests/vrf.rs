use cartridge::vrf::RequestContext;
use cartridge::{get_default_vrf_account, VrfServer, VrfServerConfig};
use katana_primitives::execution::Call;
use katana_primitives::{address, felt, ContractAddress, Felt};
use katana_rpc_types::{
    MessageHashRev1, OutsideExecution, OutsideExecutionV2, SignedOutsideExecution,
};
use starknet::macros::selector;
use starknet::signers::SigningKey;
use url::Url;

const TEST_CHAIN_ID: Felt = felt!("0x57505f4b4154414e41"); // WP_KATANA
const TEST_CALLER: ContractAddress = address!("0x414e595f43414c4c4552");

fn test_calls() -> Vec<Call> {
    vec![
        Call {
            contract_address: felt!("0x888").into(),
            entry_point_selector: selector!("request_random"),
            calldata: vec![felt!("0x111"), felt!("0x1"), felt!("0x222")],
        },
        Call {
            contract_address: felt!("0x111").into(),
            entry_point_selector: selector!("dice"),
            calldata: vec![],
        },
    ]
}

fn outside_execution_v2() -> OutsideExecution {
    OutsideExecution::V2(OutsideExecutionV2 {
        caller: TEST_CALLER,
        nonce: felt!("0x1"),
        execute_after: 0,
        execute_before: 3000000000,
        calls: test_calls(),
    })
}

#[ignore = "requires vrf-server binary"]
#[tokio::test]
async fn vrf_signed_outside_execution() {
    let vrf_creds = get_default_vrf_account().unwrap();

    let vrf_server = VrfServer::new(VrfServerConfig {
        vrf_account_address: vrf_creds.account_address,
        vrf_private_key: vrf_creds.private_key,
        secret_key: vrf_creds.secret_key,
        port: 3000,
    })
    .start()
    .await
    .unwrap();

    let signed_outside_execution = SignedOutsideExecution {
        signature: vec![],
        address: TEST_CALLER,
        outside_execution: outside_execution_v2(),
    };

    let vrf_signed_outside_execution = vrf_server
        .client()
        .outside_execution(
            &signed_outside_execution,
            &RequestContext {
                chain_id: "WP_KATANA".to_string(),
                rpc_url: Some(Url::parse("http://localhost:6000").unwrap()),
            },
        )
        .await
        .unwrap();

    let signing_key = SigningKey::from_secret_scalar(vrf_creds.private_key);
    let public_key = signing_key.verifying_key().scalar();

    let message_hash = vrf_signed_outside_execution
        .outside_execution
        .get_message_hash_rev_1(TEST_CHAIN_ID, vrf_creds.account_address);

    let vrf_signature = vrf_signed_outside_execution.signature;

    assert!(
        starknet_crypto::verify(&public_key, &message_hash, &vrf_signature[0], &vrf_signature[1])
            .unwrap(),
        "invalid vrf signature"
    );
}
