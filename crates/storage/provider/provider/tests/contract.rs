mod fixtures;

use anyhow::Result;
use katana_primitives::block::{BlockHashOrNumber, BlockNumber};
use katana_primitives::class::ClassHash;
use katana_primitives::contract::{ContractAddress, Nonce};
use katana_provider::api::state::{StateFactoryProvider, StateProvider};
use rstest_reuse::{self, *};
use starknet::macros::felt;

use crate::fixtures::db_provider_with_states;

fn assert_state_provider_contract_info(
    state_provider: Box<dyn StateProvider>,
    expected_contract_info: Vec<(ContractAddress, Option<ClassHash>, Option<Nonce>)>,
) -> Result<()> {
    for (address, expected_class_hash, expected_nonce) in expected_contract_info {
        let actual_class_hash = state_provider.class_hash_of_contract(address)?;
        let actual_nonce = state_provider.nonce(address)?;

        assert_eq!(actual_class_hash, expected_class_hash);
        assert_eq!(actual_nonce, expected_nonce);
    }

    Ok(())
}

mod latest {
    use katana_provider::{DbProviderFactory, ProviderFactory};

    use super::*;

    fn assert_latest_contract_info(
        provider: impl StateFactoryProvider,
        expected_contract_info: Vec<(ContractAddress, Option<ClassHash>, Option<Nonce>)>,
    ) -> Result<()> {
        let state_provider = provider.latest()?;
        assert_state_provider_contract_info(state_provider, expected_contract_info)
    }

    #[template]
    #[rstest::rstest]
    #[case(
        vec![
            (ContractAddress::from(felt!("1337")), Some(felt!("22")), Some(felt!("3"))),
            (ContractAddress::from(felt!("80085")), Some(felt!("33")), Some(felt!("2"))),
        ]
    )]
    fn test_latest_contract_info_read(
        #[case] expected_contract_info: Vec<(ContractAddress, Option<ClassHash>, Option<Nonce>)>,
    ) {
    }

    mod fork {
        use fixtures::fork::fork_provider_with_spawned_fork_network_and_states;
        use katana_provider::ForkProviderFactory;

        use super::*;

        #[apply(test_latest_contract_info_read)]
        fn read_storage_from_fork_provider(
            #[from(fork_provider_with_spawned_fork_network_and_states)]
            provider_factory: ForkProviderFactory,
            #[case] expected_contract_info: Vec<(
                ContractAddress,
                Option<ClassHash>,
                Option<Nonce>,
            )>,
        ) -> Result<()> {
            let provider = provider_factory.provider();
            assert_latest_contract_info(provider, expected_contract_info)
        }
    }

    #[apply(test_latest_contract_info_read)]
    fn read_storage_from_db_provider(
        #[from(db_provider_with_states)] provider_factory: DbProviderFactory,
        #[case] expected_contract_info: Vec<(ContractAddress, Option<ClassHash>, Option<Nonce>)>,
    ) -> Result<()> {
        let provider = provider_factory.provider();
        assert_latest_contract_info(provider, expected_contract_info)
    }
}

mod historical {
    use katana_provider::{DbProviderFactory, ProviderFactory};

    use super::*;

    fn assert_historical_contract_info(
        provider: impl StateFactoryProvider,
        block_num: BlockNumber,
        expected_contract_info: Vec<(ContractAddress, Option<ClassHash>, Option<Nonce>)>,
    ) -> Result<()> {
        let state_provider = provider
            .historical(BlockHashOrNumber::Num(block_num))?
            .expect(ERROR_CREATE_HISTORICAL_PROVIDER);
        assert_state_provider_contract_info(state_provider, expected_contract_info)
    }

    const ERROR_CREATE_HISTORICAL_PROVIDER: &str = "Failed to create historical state provider.";

    #[template]
    #[rstest::rstest]
    #[case::storage_at_block_0(
        0,
        vec![
        (ContractAddress::from(felt!("1337")), None, None),
        (ContractAddress::from(felt!("80085")), None, None)
    ])
]
    #[case::storage_at_block_1(
    1,
    vec![
        (ContractAddress::from(felt!("1337")), Some(felt!("11")), Some(felt!("1"))),
        (ContractAddress::from(felt!("80085")), Some(felt!("11")), Some(felt!("1"))),
    ])
]
    #[case::storage_at_block_4(
    4,
    vec![
        (ContractAddress::from(felt!("1337")), Some(felt!("11")), Some(felt!("2"))),
        (ContractAddress::from(felt!("80085")), Some(felt!("22")), Some(felt!("1"))),
    ])
]
    #[case::storage_at_block_5(
    5,
    vec![
        (ContractAddress::from(felt!("1337")), Some(felt!("22")), Some(felt!("3"))),
        (ContractAddress::from(felt!("80085")), Some(felt!("33")), Some(felt!("2"))),
    ])
]
    fn test_historical_storage_read(
        #[case] block_num: BlockNumber,
        #[case] expected_contract_info: Vec<(ContractAddress, Option<ClassHash>, Option<Nonce>)>,
    ) {
    }

    mod fork {
        use fixtures::fork::fork_provider_with_spawned_fork_network_and_states;
        use katana_provider::ForkProviderFactory;

        use super::*;

        #[apply(test_historical_storage_read)]
        fn read_storage_from_fork_provider(
            #[from(fork_provider_with_spawned_fork_network_and_states)]
            provider_factory: ForkProviderFactory,
            #[case] block_num: BlockNumber,
            #[case] expected_contract_info: Vec<(
                ContractAddress,
                Option<ClassHash>,
                Option<Nonce>,
            )>,
        ) -> Result<()> {
            let provider = provider_factory.provider();
            assert_historical_contract_info(provider, block_num, expected_contract_info)
        }
    }

    #[apply(test_historical_storage_read)]
    fn read_storage_from_db_provider(
        #[from(db_provider_with_states)] provider_factory: DbProviderFactory,
        #[case] block_num: BlockNumber,
        #[case] expected_contract_info: Vec<(ContractAddress, Option<ClassHash>, Option<Nonce>)>,
    ) -> Result<()> {
        let provider = provider_factory.provider();
        assert_historical_contract_info(provider, block_num, expected_contract_info)
    }
}
