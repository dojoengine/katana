use std::collections::BTreeMap;

use katana_contracts::contracts;
use katana_primitives::address;
use katana_primitives::block::{
    BlockHashOrNumber, FinalityStatus, Header, SealedBlock, SealedBlockWithStatus,
};
use katana_primitives::class::{ContractClass, SierraContractClass};
use katana_primitives::contract::ContractAddress;
use katana_primitives::state::{StateUpdates, StateUpdatesWithClasses};
use katana_provider::api::block::BlockWriter;
use katana_provider::api::state::StateFactoryProvider;
use katana_provider::providers::db::DbProvider;
use katana_provider::BlockchainProvider;
use lazy_static::lazy_static;
use starknet::macros::felt;

lazy_static! {
    pub static ref DOJO_WORLD_SIERRA_CLASS: SierraContractClass =
        serde_json::from_str(include_str!("./fixtures/dojo_world_240.json")).unwrap();
}

pub mod fork {

    use katana_provider::providers::fork::ForkedProvider;
    use katana_rpc_client::starknet::Client as StarknetClient;
    use katana_rpc_client::HttpClientBuilder;
    use katana_runner::KatanaRunner;
    use lazy_static::lazy_static;

    use super::*;

    lazy_static! {
        pub static ref FORKED_PROVIDER: (KatanaRunner, StarknetClient) = {
            let runner = katana_runner::KatanaRunner::new().unwrap();
            let url = runner.url();
            (runner, StarknetClient::new(HttpClientBuilder::new().build(url).unwrap()))
        };
    }

    #[rstest::fixture]
    pub fn fork_provider(
        #[default("https://api.cartridge.gg/x/starknet/sepolia")] rpc: &str,
        #[default(2888618)] block_num: u64,
    ) -> BlockchainProvider<ForkedProvider> {
        let provider = StarknetClient::new(HttpClientBuilder::new().build(rpc).unwrap());
        let provider = ForkedProvider::new_ephemeral(block_num.into(), provider);
        BlockchainProvider::new(provider)
    }

    #[rstest::fixture]
    pub fn fork_provider_with_spawned_fork_network(
        #[default(0)] block_num: u64,
    ) -> BlockchainProvider<ForkedProvider> {
        let provider = ForkedProvider::new_ephemeral(block_num.into(), FORKED_PROVIDER.1.clone());
        BlockchainProvider::new(provider)
    }
}

#[rstest::fixture]
pub fn db_provider() -> BlockchainProvider<DbProvider> {
    BlockchainProvider::new(DbProvider::new_in_memory())
}

#[rstest::fixture]
pub fn mock_state_updates() -> [StateUpdatesWithClasses; 3] {
    let address_1 = address!("1337");
    let address_2 = address!("80085");

    let class_hash_1 = felt!("11");
    let compiled_class_hash_1 = felt!("1000");

    let class_hash_2 = felt!("22");
    let compiled_class_hash_2 = felt!("2000");

    let class_hash_3 = felt!("33");
    let compiled_class_hash_3 = felt!("3000");

    let state_update_1 = StateUpdatesWithClasses {
        state_updates: StateUpdates {
            nonce_updates: BTreeMap::from([(address_1, 1u8.into()), (address_2, 1u8.into())]),
            storage_updates: BTreeMap::from([
                (
                    address_1,
                    BTreeMap::from([(1u8.into(), 100u32.into()), (2u8.into(), 101u32.into())]),
                ),
                (
                    address_2,
                    BTreeMap::from([(1u8.into(), 200u32.into()), (2u8.into(), 201u32.into())]),
                ),
            ]),
            declared_classes: BTreeMap::from([(class_hash_1, compiled_class_hash_1)]),
            deployed_contracts: BTreeMap::from([
                (address_1, class_hash_1),
                (address_2, class_hash_1),
            ]),
            ..Default::default()
        },
        classes: BTreeMap::from([(class_hash_1, contracts::LegacyERC20::CLASS.clone())]),
    };

    let state_update_2 = StateUpdatesWithClasses {
        state_updates: StateUpdates {
            nonce_updates: BTreeMap::from([(address_1, 2u8.into())]),
            storage_updates: BTreeMap::from([(
                address_1,
                BTreeMap::from([(felt!("1"), felt!("111")), (felt!("2"), felt!("222"))]),
            )]),
            declared_classes: BTreeMap::from([(class_hash_2, compiled_class_hash_2)]),
            deployed_contracts: BTreeMap::from([(address_2, class_hash_2)]),
            ..Default::default()
        },
        classes: BTreeMap::from([(class_hash_2, contracts::UniversalDeployer::CLASS.clone())]),
    };

    let state_update_3 = StateUpdatesWithClasses {
        state_updates: StateUpdates {
            nonce_updates: BTreeMap::from([(address_1, 3u8.into()), (address_2, 2u8.into())]),
            storage_updates: BTreeMap::from([
                (address_1, BTreeMap::from([(3u8.into(), 77u32.into())])),
                (
                    address_2,
                    BTreeMap::from([(1u8.into(), 12u32.into()), (2u8.into(), 13u32.into())]),
                ),
            ]),
            deployed_contracts: BTreeMap::from([
                (address_1, class_hash_2),
                (address_2, class_hash_3),
            ]),
            declared_classes: BTreeMap::from([(class_hash_3, compiled_class_hash_3)]),
            ..Default::default()
        },
        classes: BTreeMap::from([(
            class_hash_3,
            ContractClass::Class((*DOJO_WORLD_SIERRA_CLASS).clone()),
        )]),
    };

    [state_update_1, state_update_2, state_update_3]
}

#[rstest::fixture]
#[default(BlockchainProvider<DbProvider>)]
pub fn provider_with_states<Db>(
    #[default(db_provider())] provider: BlockchainProvider<Db>,
    #[from(mock_state_updates)] state_updates: [StateUpdatesWithClasses; 3],
) -> BlockchainProvider<Db>
where
    Db: BlockWriter + StateFactoryProvider,
{
    for i in 0..=5 {
        let block_id = BlockHashOrNumber::from(i);

        let state_update = match block_id {
            BlockHashOrNumber::Num(1) => state_updates[0].clone(),
            BlockHashOrNumber::Num(2) => state_updates[1].clone(),
            BlockHashOrNumber::Num(5) => state_updates[2].clone(),
            _ => StateUpdatesWithClasses::default(),
        };

        provider
            .insert_block_with_states_and_receipts(
                SealedBlockWithStatus {
                    status: FinalityStatus::AcceptedOnL2,
                    block: SealedBlock {
                        hash: i.into(),
                        header: Header { number: i, ..Default::default() },
                        body: Default::default(),
                    },
                },
                state_update,
                Default::default(),
                Default::default(),
            )
            .unwrap();
    }

    provider
}
