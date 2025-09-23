use katana_chain_spec::ChainSpec;
use katana_primitives::chain::ChainId;
use katana_primitives::contract::{ContractAddress, Nonce};
use katana_primitives::da::DataAvailabilityMode;
use katana_primitives::env::CfgEnv;
use katana_primitives::fee::{AllResourceBoundsMapping, ResourceBounds, ResourceBoundsMapping};
use katana_genesis::allocation::GenesisAllocation;
use katana_genesis::constant::DEFAULT_ETH_FEE_TOKEN_ADDRESS;
use katana_primitives::transaction::ExecutableTxWithHash;
use katana_primitives::utils::transaction::compute_invoke_v3_tx_hash;
use katana_primitives::Felt;
use katana_rpc_types::broadcasted::BroadcastedInvokeTx;
use starknet::accounts::{Account, ExecutionEncoder, ExecutionEncoding, SingleOwnerAccount};
use starknet::core::types::{BlockId, BlockTag, Call};
use starknet::macros::{felt, selector};
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::{JsonRpcClient, Url};
use starknet::signers::{LocalWallet, Signer, SigningKey};

use super::{cfg, chain};

#[allow(unused)]
pub fn invoke_executable_tx(
    address: ContractAddress,
    private_key: Felt,
    chain_id: ChainId,
    nonce: Nonce,
    max_fee: Felt,
    signed: bool,
) -> ExecutableTxWithHash {
    let url = "http://localhost:5050";
    let provider = JsonRpcClient::new(HttpTransport::new(Url::try_from(url).unwrap()));
    let signer = LocalWallet::from_signing_key(SigningKey::from_secret_scalar(private_key));

    let mut account = SingleOwnerAccount::new(
        &provider,
        &signer,
        address.into(),
        chain_id.into(),
        ExecutionEncoding::New,
    );

    account.set_block_id(BlockId::Tag(BlockTag::PreConfirmed));

    let calls = vec![Call {
        to: DEFAULT_ETH_FEE_TOKEN_ADDRESS.into(),
        selector: selector!("transfer"),
        calldata: vec![felt!("0x1"), felt!("0x99"), felt!("0x0")],
    }];

    let tip = 0;
    let l1_gas = ResourceBounds { max_amount: 0, max_price_per_unit: 0 };
    let l2_gas = ResourceBounds { max_amount: 0, max_price_per_unit: 0 };
    let l1_data_gas = ResourceBounds { max_amount: 0, max_price_per_unit: 0 };

    let nonce_da_mode = DataAvailabilityMode::L1;
    let fee_da_mode = DataAvailabilityMode::L1;
    let calldata = account.encode_calls(&calls);

    let hash = compute_invoke_v3_tx_hash(
        account.address(),
        &calldata,
        tip,
        &l1_gas,
        &l2_gas,
        Some(&l1_data_gas),
        &[],
        chain_id.into(),
        nonce,
        &nonce_da_mode,
        &fee_da_mode,
        &[],
        false,
    );

    let signature = if signed {
        let signature = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(signer.sign_hash(&hash))
            .unwrap();

        vec![signature.r, signature.s]
    } else {
        vec![]
    };

    let resource_bounds = ResourceBoundsMapping::All(AllResourceBoundsMapping {
        l1_gas: ResourceBounds {
            max_amount: l1_gas.max_amount,
            max_price_per_unit: l1_gas.max_price_per_unit,
        },
        l1_data_gas: ResourceBounds {
            max_amount: l1_data_gas.max_amount,
            max_price_per_unit: l1_data_gas.max_price_per_unit,
        },
        l2_gas: ResourceBounds {
            max_amount: l2_gas.max_amount,
            max_price_per_unit: l2_gas.max_price_per_unit,
        },
    });

    let mut broadcasted_tx = BroadcastedInvokeTx {
        is_query: false,
        sender_address: account.address().into(),
        calldata,
        signature,
        nonce,
        resource_bounds: resource_bounds.into(),
        tip: tip.into(),
        paymaster_data: vec![],
        account_deployment_data: vec![],
        nonce_data_availability_mode: nonce_da_mode,
        fee_data_availability_mode: fee_da_mode,
    };

    if !signed {
        broadcasted_tx.signature = vec![]
    }

    ExecutableTxWithHash::new(broadcasted_tx.into_inner(chain_id).into())
}

#[rstest::fixture]
fn signed() -> bool {
    true
}

#[rstest::fixture]
pub fn executable_tx(signed: bool, chain: &ChainSpec, cfg: CfgEnv) -> ExecutableTxWithHash {
    let (addr, alloc) = chain.genesis().allocations.first_key_value().expect("should have account");

    let GenesisAllocation::Account(account) = alloc else {
        panic!("should be account");
    };

    invoke_executable_tx(
        *addr,
        account.private_key().unwrap(),
        cfg.chain_id,
        Felt::ZERO,
        // this is an arbitrary large fee so that it doesn't fail
        felt!("0x999999999999999"),
        signed,
    )
}

#[rstest::fixture]
pub fn executable_tx_without_max_fee(
    signed: bool,
    chain: &ChainSpec,
    cfg: CfgEnv,
) -> ExecutableTxWithHash {
    let (addr, alloc) = chain.genesis().allocations.first_key_value().expect("should have account");

    let GenesisAllocation::Account(account) = alloc else {
        panic!("should be account");
    };

    invoke_executable_tx(
        *addr,
        account.private_key().unwrap(),
        cfg.chain_id,
        Felt::ZERO,
        Felt::ZERO,
        signed,
    )
}
