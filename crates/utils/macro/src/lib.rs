#![cfg_attr(not(test), warn(unused_crate_dependencies))]

//! Procedural macros for creating mock implementations of traits for testing.

mod mock_provider;

/// A procedural macro for creating mock implementations of the Provider trait.
///
/// The macro allows you to create mock implementations of the Starknet
/// [`Provider`](starknet::providers::Provider) trait with minimal boilerplate.
///
/// # Usage
///
/// ```ignore
/// use katana_utils::mock_provider;
/// use starknet::core::types::{BlockId, Felt};
/// use starknet::providers::Provider;
///
/// mock_provider! {
///     MyMockProvider,
///
///     fn get_storage_at: (contract_address, key, block_id) => {
///         Ok(Felt::from(42u32))
///     },
///
///     fn chain_id: () => {
///         Ok(Felt::from(1u32))
///     }
/// }
///
/// #[tokio::test]
/// async fn test_mock_provider() {
///     let provider = MyMockProvider::new();
///     let storage = provider
///         .get_storage_at(
///             Felt::from(1u32),
///             Felt::from(2u32),
///             BlockId::Tag(starknet::core::types::BlockTag::Latest),
///         )
///         .await
///         .unwrap();
///     assert_eq!(storage, Felt::from(42u32));
/// }
/// ```
///
/// This will generate:
/// - A struct with the specified name
/// - A Provider trait implementation with your custom methods
/// - `unimplemented!()` for all other Provider methods
#[proc_macro]
pub fn mock_provider(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    mock_provider::mock_provider_impl(input.into()).into()
}
