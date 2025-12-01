mod fixtures;

use anyhow::Result;
use katana_primitives::block::{BlockHashOrNumber, BlockNumber};
use katana_primitives::contract::{ContractAddress, StorageKey, StorageValue};
use katana_provider::api::state::{StateFactoryProvider, StateProvider};
use rstest_reuse::{self, *};
use starknet::macros::felt;

use crate::fixtures::db_provider_with_states;

fn assert_state_provider_storage(
    state_provider: Box<dyn StateProvider>,
    expected_storage_entry: Vec<(ContractAddress, StorageKey, Option<StorageValue>)>,
) -> Result<()> {
    for (address, key, expected_value) in expected_storage_entry {
        let actual_value = state_provider.storage(address, key)?;
        assert_eq!(actual_value, expected_value);
    }
    Ok(())
}

mod latest {
    use katana_provider::{DbProviderFactory, ProviderFactory};

    use super::*;

    fn assert_latest_storage_value(
        provider: impl StateFactoryProvider,
        expected_storage_entry: Vec<(ContractAddress, StorageKey, Option<StorageValue>)>,
    ) -> Result<()> {
        let state_provider = provider.latest()?;
        assert_state_provider_storage(state_provider, expected_storage_entry)
    }

    #[template]
    #[rstest::rstest]
    #[case(
        vec![
            (ContractAddress::from(felt!("1337")), felt!("1"), Some(felt!("111"))),
            (ContractAddress::from(felt!("1337")), felt!("2"), Some(felt!("222"))),
            (ContractAddress::from(felt!("1337")), felt!("3"), Some(felt!("77"))),
            (ContractAddress::from(felt!("80085")), felt!("1"), Some(felt!("12"))),
            (ContractAddress::from(felt!("80085")), felt!("2"), Some(felt!("13")))
        ]
    )]
    fn test_latest_storage_read(
        #[case] storage_entry: Vec<(ContractAddress, StorageKey, Option<StorageValue>)>,
    ) {
    }

    mod fork {
        use fixtures::fork::fork_provider_with_spawned_fork_network_and_states;
        use katana_provider::ForkProviderFactory;

        use super::*;

        #[apply(test_latest_storage_read)]
        fn read_storage_from_fork_provider_with_spawned_fork_network(
            #[from(fork_provider_with_spawned_fork_network_and_states)]
            provider_factory: ForkProviderFactory,
            #[case] expected_storage_entry: Vec<(
                ContractAddress,
                StorageKey,
                Option<StorageValue>,
            )>,
        ) -> Result<()> {
            let provider = provider_factory.provider();
            assert_latest_storage_value(provider, expected_storage_entry)
        }
    }

    #[apply(test_latest_storage_read)]
    fn read_storage_from_db_provider(
        #[from(db_provider_with_states)] provider_factory: DbProviderFactory,
        #[case] expected_storage_entry: Vec<(ContractAddress, StorageKey, Option<StorageValue>)>,
    ) -> Result<()> {
        let provider = provider_factory.provider();
        assert_latest_storage_value(provider, expected_storage_entry)
    }
}

mod historical {
    use katana_provider::{DbProviderFactory, ProviderFactory};

    use super::*;

    fn assert_historical_storage_value(
        provider: impl StateFactoryProvider,
        block_num: BlockNumber,
        expected_storage_entry: Vec<(ContractAddress, StorageKey, Option<StorageValue>)>,
    ) -> Result<()> {
        let state_provider = provider
            .historical(BlockHashOrNumber::Num(block_num))?
            .expect(ERROR_CREATE_HISTORICAL_PROVIDER);
        assert_state_provider_storage(state_provider, expected_storage_entry)
    }

    const ERROR_CREATE_HISTORICAL_PROVIDER: &str = "Failed to create historical state provider.";

    #[template]
    #[rstest::rstest]
    #[case::storage_at_block_0(
        0,
        vec![
            (ContractAddress::from(felt!("1337")), felt!("1"), None),
            (ContractAddress::from(felt!("1337")), felt!("2"), None),
            (ContractAddress::from(felt!("80085")), felt!("1"), None),
            (ContractAddress::from(felt!("80085")), felt!("2"), None)
        ])
    ]
    #[case::storage_at_block_1(
        1,
        vec![
            (ContractAddress::from(felt!("1337")), felt!("1"), Some(felt!("100"))),
            (ContractAddress::from(felt!("1337")), felt!("2"), Some(felt!("101"))),
            (ContractAddress::from(felt!("80085")), felt!("1"), Some(felt!("200"))),
            (ContractAddress::from(felt!("80085")), felt!("2"), Some(felt!("201"))),
        ])
    ]
    #[case::storage_at_block_4(
        4,
        vec![
            (ContractAddress::from(felt!("1337")), felt!("1"), Some(felt!("111"))),
            (ContractAddress::from(felt!("1337")), felt!("2"), Some(felt!("222"))),
            (ContractAddress::from(felt!("80085")), felt!("1"), Some(felt!("200"))),
            (ContractAddress::from(felt!("80085")), felt!("2"), Some(felt!("201"))),
        ])
    ]
    #[case::storage_at_block_5(
        5,
        vec![
            (ContractAddress::from(felt!("1337")), felt!("1"), Some(felt!("111"))),
            (ContractAddress::from(felt!("1337")), felt!("2"), Some(felt!("222"))),
            (ContractAddress::from(felt!("1337")), felt!("3"), Some(felt!("77"))),
            (ContractAddress::from(felt!("80085")), felt!("1"), Some(felt!("12"))),
            (ContractAddress::from(felt!("80085")), felt!("2"), Some(felt!("13"))),
        ])
    ]
    fn test_historical_storage_read(
        #[case] block_num: BlockNumber,
        #[case] storage_entry: Vec<(ContractAddress, StorageKey, Option<StorageValue>)>,
    ) {
    }

    mod fork {
        use fixtures::fork::fork_provider_with_spawned_fork_network_and_states;
        use katana_provider::ForkProviderFactory;

        use super::*;

        #[apply(test_historical_storage_read)]
        fn read_storage_from_fork_provider_with_spawned_fork_network(
            #[from(fork_provider_with_spawned_fork_network_and_states)]
            provider_factory: ForkProviderFactory,
            #[case] block_num: BlockNumber,
            #[case] expected_storage_entry: Vec<(
                ContractAddress,
                StorageKey,
                Option<StorageValue>,
            )>,
        ) -> Result<()> {
            let provider = provider_factory.provider();
            assert_historical_storage_value(provider, block_num, expected_storage_entry)
        }
    }

    #[apply(test_historical_storage_read)]
    fn read_storage_from_db_provider(
        #[from(db_provider_with_states)] provider_factory: DbProviderFactory,
        #[case] block_num: BlockNumber,
        #[case] expected_storage_entry: Vec<(ContractAddress, StorageKey, Option<StorageValue>)>,
    ) -> Result<()> {
        let provider = provider_factory.provider();
        assert_historical_storage_value(provider, block_num, expected_storage_entry)
    }
}
