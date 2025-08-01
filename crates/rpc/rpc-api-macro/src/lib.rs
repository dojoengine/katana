//! Procedural macros for simplifying RPC API trait definitions.
//!
//! This crate provides macros that wrap jsonrpsee's `rpc` macro to reduce boilerplate
//! when defining RPC traits, especially for versioned APIs like Starknet's.

use proc_macro::TokenStream;

mod entry;

/// A procedural macro that enhances RPC trait definitions with automatic version handling.
///
/// This macro wraps the jsonrpsee `rpc` macro and automatically generates:
/// - A `RPC_SPEC_VERSION` constant with the specified version
/// - A `spec_version` method implementation that returns the version
///
/// # Arguments
///
/// * `server` - Generate server implementation (optional)
/// * `client` - Generate client implementation (optional)
/// * `version` - The RPC specification version (required)
/// * `namespace` - The RPC namespace (required)
///
/// If neither `server` nor `client` is specified, both implementations are generated.
///
/// # Examples
///
/// ```ignore
/// use jsonrpsee::core::RpcResult;
/// use jsonrpsee::proc_macros::rpc;
/// use katana_rpc_api_macro::starknet_rpc;
///
/// // Generate both server and client
/// #[starknet_rpc(server, client, version = "0.8.1", namespace = "starknet")]
/// pub trait StarknetApi {
///     // Your RPC methods here
///     #[method(name = "blockNumber")]
///     async fn block_number(&self) -> RpcResult<u64>;
/// }
///
/// // Generate server only
/// #[starknet_rpc(server, version = "0.8.1", namespace = "starknet")]
/// pub trait ServerOnlyApi {
///     #[method(name = "serverMethod")]
///     async fn server_method(&self) -> RpcResult<String>;
/// }
///
/// // Generate client only
/// #[starknet_rpc(client, version = "0.8.1", namespace = "starknet")]
/// pub trait ClientOnlyApi {
///     #[method(name = "clientMethod")]
///     async fn client_method(&self) -> RpcResult<String>;
/// }
/// ```
///
/// This will generate:
/// - `pub const RPC_SPEC_VERSION: &str = "0.8.1";` (for StarknetApi trait)
/// - `pub const <TRAIT_NAME>_RPC_SPEC_VERSION: &str = "0.8.1";` (for other traits)
/// - An automatic implementation of `spec_version()` method that returns this version
/// - The appropriate `#[rpc(...)]` attributes
///
/// The generated code will look like:
///
/// ```ignore
/// pub const RPC_SPEC_VERSION: &str = "0.8.1";
///
/// #[rpc(server, client, namespace = "starknet")]
/// pub trait StarknetApi {
///     #[method(name = "specVersion")]
///     async fn spec_version(&self) -> RpcResult<String> {
///         Ok(RPC_SPEC_VERSION.into())
///     }
///
///     #[method(name = "blockNumber")]
///     async fn block_number(&self) -> RpcResult<u64>;
/// }
/// ```
#[proc_macro_attribute]
pub fn starknet_rpc(args: TokenStream, input: TokenStream) -> TokenStream {
    entry::starknet_rpc(args.into(), input.into()).into()
}
