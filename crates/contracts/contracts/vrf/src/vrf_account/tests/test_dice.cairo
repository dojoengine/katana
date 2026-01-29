use openzeppelin::account::extensions::src9::OutsideExecution;
use openzeppelin::account::interface::{ISRC6Dispatcher, ISRC6DispatcherTrait};
use snforge_std::{
    CheatSpan, cheat_signature, cheat_transaction_hash, start_cheat_caller_address,
    start_cheat_max_fee,
};
use starknet::account::Call;
use super::common::{ANY_CALLER, CONSUMER, CONSUMER_ACCOUNT, VRF_ACCOUNT, ZERO_ADDRESS, setup};


#[test]
fn test_multicall() {
    let setup = setup();

    let call0 = Call {
        to: CONSUMER, selector: selector!("get_dice_value"), calldata: array![].span(),
    };
    let call1 = Call {
        to: CONSUMER, selector: selector!("get_dice_value"), calldata: array![].span(),
    };

    let calls = array![call0, call1];

    start_cheat_caller_address(setup.consumer_account.contract_address, ZERO_ADDRESS);
    let disp = ISRC6Dispatcher { contract_address: setup.consumer_account.contract_address };

    disp.__execute__(calls);
}


#[test]
fn test_vrf() {
    let setup = setup();

    let request_random = Call {
        to: VRF_ACCOUNT,
        selector: selector!("request_random"),
        calldata: array![CONSUMER.into(), 0x0, // Source::Nonce
        CONSUMER_ACCOUNT.into()].span(),
    };
    let dice_call = Call { to: CONSUMER, selector: selector!("dice"), calldata: array![].span() };

    let consumer_account_calls = array![request_random, dice_call];

    let outside_execution = OutsideExecution {
        caller: ANY_CALLER,
        nonce: 0,
        execute_after: 0,
        execute_before: 999,
        calls: consumer_account_calls.span(),
    };
    let signature = array![
        3526767899891781004414137567219167473278784810269869008283119685065145083722,
        353496959307647856729870737612651536400118077590332991504830665655429751219,
    ]
        .span();

    let sumbit_random = Call {
        to: VRF_ACCOUNT,
        selector: selector!("submit_random"),
        calldata: array![
            0x5db4e1c9bd8b0898674bf96f79e8fbffa3fe6d70a4597683c4dba2f0930dc45, // seed
            // proof
            0x16aec715f329872b75ca9beb77557ca6c9d3a67a01a3363df86496d2c3a261d,
            0x22c44f72eccd63fb28a0e7bf3205f45130c57db28f9c35777e90ae5e414c246,
            0x28bfff7440a8dcd8e86d260eff5aea305db55b9183109eff9f81a99e150e8ea,
            0x16d49559522712102d3459c7999b0fabf24f2525b1a5e3f91d148a7a7256576,
            0x5c87f6e05d61823e0646ff56675fab2e3c01b5b09deee479390d5a50ce34b83,
        ]
            .span(),
    };

    //     curl -X POST -H "Content-Type: application/json" -d '{"seed":
    // ["0x5db4e1c9bd8b0898674bf96f79e8fbffa3fe6d70a4597683c4dba2f0930dc45"]}'
    // http://0.0.0.0:3000/stark_vrf

    let mut outside_execution_calldata = array![];
    outside_execution.serialize(ref outside_execution_calldata);
    signature.serialize(ref outside_execution_calldata);
    let outside_execution_call = Call {
        to: CONSUMER_ACCOUNT,
        selector: selector!("execute_from_outside_v2"),
        calldata: outside_execution_calldata.span(),
    };

    let vrf_account_calls = array![sumbit_random, outside_execution_call];

    start_cheat_caller_address(setup.vrf_account.contract_address, ZERO_ADDRESS);
    cheat_transaction_hash(setup.vrf_account.contract_address, 0x123, CheatSpan::TargetCalls(1));
    cheat_signature(
        setup.vrf_account.contract_address,
        array![
            248467116562972322059502545450223135318561830597727224855446137733023338075,
            791470283963215574516589790678480760757926615323666747565268653048092433018,
        ]
            .span(),
        CheatSpan::TargetCalls(1),
    );
    start_cheat_max_fee(setup.vrf_account.contract_address, 1);

    let disp = ISRC6Dispatcher { contract_address: setup.vrf_account.contract_address };

    disp.__validate__(vrf_account_calls.clone());
    disp.__execute__(vrf_account_calls);
}
