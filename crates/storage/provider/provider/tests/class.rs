mod fixtures;

use anyhow::Result;
use fixtures::{provider_with_states, DOJO_WORLD_SIERRA_CLASS};
use katana_primitives::block::{BlockHashOrNumber, BlockNumber};
use katana_primitives::class::{ClassHash, CompiledClassHash, ContractClass};
use katana_provider::api::contract::ContractClassProviderExt;
use katana_provider::api::state::{StateFactoryProvider, StateProvider};
use rstest_reuse::{self, *};
use starknet::macros::felt;

type ClassHashAndClasses = (ClassHash, Option<CompiledClassHash>, Option<ContractClass>);

fn assert_state_provider_class(
    state_provider: Box<dyn StateProvider>,
    expected_class: Vec<ClassHashAndClasses>,
) -> Result<()> {
    for (class_hash, expected_compiled_hash, expected_class) in expected_class {
        let actual_class = state_provider.class(class_hash)?;
        let actual_compiled_class = state_provider.compiled_class(class_hash)?;
        let actual_compiled_hash = state_provider.compiled_class_hash_of_class_hash(class_hash)?;

        if let Some(expected_class) = expected_class {
            let expected_compiled = expected_class.clone().compile().expect("fail to compile");

            assert_eq!(actual_class, Some(expected_class));
            assert_eq!(actual_compiled_hash, expected_compiled_hash);
            assert_eq!(actual_compiled_class, Some(expected_compiled));
        }
    }
    Ok(())
}

mod latest {
    use katana_contracts::contracts;
    use katana_provider::{DbProviderFactory, ProviderFactory};

    use super::*;
    use crate::fixtures::db_provider;

    fn assert_latest_class(
        provider: impl StateFactoryProvider,
        expected_class: Vec<ClassHashAndClasses>,
    ) -> Result<()> {
        let state_provider = provider.latest()?;
        assert_state_provider_class(state_provider, expected_class)
    }

    #[template]
    #[rstest::rstest]
    #[case(
        vec![
            (felt!("11"), Some(felt!("1000")), Some(contracts::LegacyERC20::CLASS.clone())),
            (felt!("22"), Some(felt!("2000")), Some(contracts::UniversalDeployer::CLASS.clone())),
            (felt!("33"), Some(felt!("3000")), Some(ContractClass::Class((*DOJO_WORLD_SIERRA_CLASS).clone()))),
        ]
    )]
    fn test_latest_class_read(
        #[from(provider_with_states)] provider_factory: impl ProviderFactory,
        #[case] expected_class: Vec<ClassHashAndClasses>,
    ) {
    }

    mod fork {
        use fixtures::fork::fork_provider_with_spawned_fork_network;
        use katana_provider::{ForkProviderFactory, ProviderFactory};

        use super::*;

        #[apply(test_latest_class_read)]
        fn read_class_from_fork_provider(
            #[with(fork_provider_with_spawned_fork_network::default())]
            provider_factory: ForkProviderFactory,
            #[case] expected_classes: Vec<ClassHashAndClasses>,
        ) -> Result<()> {
            let provider = provider_factory.provider();
            assert_latest_class(provider, expected_classes)
        }
    }

    #[apply(test_latest_class_read)]
    fn read_class_from_db_provider(
        #[with(db_provider())] provider_factory: DbProviderFactory,
        #[case] expected_classes: Vec<ClassHashAndClasses>,
    ) -> Result<()> {
        let provider = provider_factory.provider();
        assert_latest_class(provider, expected_classes)
    }
}

mod historical {
    use katana_contracts::contracts;
    use katana_provider::{DbProviderFactory, ProviderFactory};

    use super::*;
    use crate::fixtures::db_provider;

    fn assert_historical_class(
        provider: impl StateFactoryProvider,
        block_num: BlockNumber,
        expected_class: Vec<ClassHashAndClasses>,
    ) -> Result<()> {
        let state_provider = provider
            .historical(BlockHashOrNumber::Num(block_num))?
            .expect(ERROR_CREATE_HISTORICAL_PROVIDER);
        assert_state_provider_class(state_provider, expected_class)
    }

    const ERROR_CREATE_HISTORICAL_PROVIDER: &str = "Failed to create historical state provider.";

    #[template]
    #[rstest::rstest]
    #[case::class_hash_at_block_0(
        0,
        vec![
            (felt!("11"), None, None),
            (felt!("22"), None, None),
            (felt!("33"), None, None)
        ])
    ]
    #[case::class_hash_at_block_1(
        1,
        vec![
            (felt!("11"), Some(felt!("1000")), Some(contracts::LegacyERC20::CLASS.clone())),
            (felt!("22"), None, None),
            (felt!("33"), None, None),
        ])
    ]
    #[case::class_hash_at_block_4(
        4,
        vec![
            (felt!("11"), Some(felt!("1000")), Some(contracts::LegacyERC20::CLASS.clone())),
            (felt!("22"), Some(felt!("2000")), Some(contracts::UniversalDeployer::CLASS.clone())),
            (felt!("33"), None, None),
        ])
    ]
    #[case::class_hash_at_block_5(
        5,
        vec![
            (felt!("11"), Some(felt!("1000")), Some(contracts::LegacyERC20::CLASS.clone())),
            (felt!("22"), Some(felt!("2000")), Some(contracts::UniversalDeployer::CLASS.clone())),
            (felt!("33"), Some(felt!("3000")), Some(ContractClass::Class((*DOJO_WORLD_SIERRA_CLASS).clone()))),
        ])
    ]
    fn test_historical_class_read(
        #[from(provider_with_states)] provider_factory: impl ProviderFactory,
        #[case] block_num: BlockNumber,
        #[case] expected_class: Vec<ClassHashAndClasses>,
    ) {
    }

    mod fork {
        use fixtures::fork::fork_provider_with_spawned_fork_network;
        use katana_provider::{ForkProviderFactory, ProviderFactory};

        use super::*;

        #[apply(test_historical_class_read)]
        fn read_class_from_fork_provider(
            #[with(fork_provider_with_spawned_fork_network::default())]
            provider_factory: ForkProviderFactory,
            #[case] block_num: BlockNumber,
            #[case] expected_classes: Vec<ClassHashAndClasses>,
        ) -> Result<()> {
            let provider = provider_factory.provider();
            assert_historical_class(provider, block_num, expected_classes)
        }
    }

    #[apply(test_historical_class_read)]
    fn read_class_from_db_provider(
        #[with(db_provider())] provider_factory: DbProviderFactory,
        #[case] block_num: BlockNumber,
        #[case] expected_classes: Vec<ClassHashAndClasses>,
    ) -> Result<()> {
        let provider = provider_factory.provider();
        assert_historical_class(provider, block_num, expected_classes)
    }
}
