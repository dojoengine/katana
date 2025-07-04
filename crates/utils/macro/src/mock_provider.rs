use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::{Ident, Result, Token};

/// Parsed input for the mock_provider macro
pub struct MockProviderInput {
    struct_name: Ident,
    methods: Vec<MockMethod>,
}

/// A single method implementation in the mock
pub struct MockMethod {
    name: Ident,
    params: Vec<Ident>,
    body: TokenStream2,
}

impl Parse for MockProviderInput {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let struct_name: Ident = input.parse()?;
        input.parse::<Token![,]>()?;

        let mut methods = Vec::new();
        while !input.is_empty() {
            methods.push(input.parse()?);
            if input.peek(Token![,]) {
                input.parse::<Token![,]>()?;
            }
        }

        Ok(MockProviderInput { struct_name, methods })
    }
}

impl Parse for MockMethod {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        input.parse::<Token![fn]>()?;
        let name: Ident = input.parse()?;
        input.parse::<Token![:]>()?;

        let content;
        syn::parenthesized!(content in input);
        let params =
            Punctuated::<Ident, Token![,]>::parse_terminated(&content)?.into_iter().collect();

        input.parse::<Token![=>]>()?;
        let body;
        syn::braced!(body in input);
        let body: TokenStream2 = body.parse()?;

        Ok(MockMethod { name, params, body })
    }
}

/// Generate the complete mock provider implementation
pub fn generate_mock_provider(input: MockProviderInput) -> TokenStream2 {
    let struct_name = &input.struct_name;
    let methods = &input.methods;

    let struct_def = generate_struct_definition(struct_name);
    let provider_impl = generate_provider_impl(struct_name, methods);

    quote! {
        #struct_def
        #provider_impl
    }
}

/// Generate the struct definition
fn generate_struct_definition(struct_name: &Ident) -> TokenStream2 {
    quote! {
        #[derive(Debug, Clone)]
        pub struct #struct_name;

        impl #struct_name {
            pub fn new() -> Self {
                Self
            }
        }

        impl Default for #struct_name {
            fn default() -> Self {
                Self::new()
            }
        }
    }
}

/// Generate the Provider trait implementation
fn generate_provider_impl(struct_name: &Ident, methods: &[MockMethod]) -> TokenStream2 {
    let all_methods = get_all_provider_methods();
    let mut method_impls = Vec::new();

    for method in &all_methods {
        if let Some(user_method) = methods.iter().find(|m| m.name == method.name) {
            // Use user implementation
            method_impls.push(generate_user_method_impl(method, user_method));
        } else {
            // Use unimplemented!() for methods not provided by user
            method_impls.push(generate_unimplemented_method(method));
        }
    }

    quote! {
        #[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
        #[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
        impl starknet::providers::Provider for #struct_name {
            #(#method_impls)*
        }
    }
}

/// Generate a user-provided method implementation
fn generate_user_method_impl(method: &ProviderMethod, user_method: &MockMethod) -> TokenStream2 {
    let method_name = &method.name;
    let return_type = &method.return_type;
    let params = &method.params;
    let where_clause = &method.where_clause;
    let body = &user_method.body;

    quote! {
        async fn #method_name #params -> #return_type #where_clause {
            #body
        }
    }
}

/// Generate an unimplemented method
fn generate_unimplemented_method(method: &ProviderMethod) -> TokenStream2 {
    let method_name = &method.name;
    let return_type = &method.return_type;
    let params = &method.params;
    let where_clause = &method.where_clause;

    quote! {
        async fn #method_name #params -> #return_type #where_clause {
            unimplemented!("Method {} not implemented in mock", stringify!(#method_name))
        }
    }
}

/// Represents a single method in the Provider trait
struct ProviderMethod {
    name: Ident,
    params: TokenStream2,
    return_type: TokenStream2,
    where_clause: TokenStream2,
}

/// Get all Provider trait methods with their signatures
fn get_all_provider_methods() -> Vec<ProviderMethod> {
    vec![
        ProviderMethod {
            name: syn::parse_str("spec_version").unwrap(),
            params: quote! { (&self) },
            return_type: quote! { Result<String, starknet::providers::ProviderError> },
            where_clause: quote! {},
        },
        ProviderMethod {
            name: syn::parse_str("get_block_with_tx_hashes").unwrap(),
            params: quote! { <B>(&self, block_id: B) },
            return_type: quote! { Result<starknet::core::types::MaybePendingBlockWithTxHashes, starknet::providers::ProviderError> },
            where_clause: quote! { where B: AsRef<starknet::core::types::BlockId> + Send + Sync },
        },
        ProviderMethod {
            name: syn::parse_str("get_block_with_txs").unwrap(),
            params: quote! { <B>(&self, block_id: B) },
            return_type: quote! { Result<starknet::core::types::MaybePendingBlockWithTxs, starknet::providers::ProviderError> },
            where_clause: quote! { where B: AsRef<starknet::core::types::BlockId> + Send + Sync },
        },
        ProviderMethod {
            name: syn::parse_str("get_block_with_receipts").unwrap(),
            params: quote! { <B>(&self, block_id: B) },
            return_type: quote! { Result<starknet::core::types::MaybePendingBlockWithReceipts, starknet::providers::ProviderError> },
            where_clause: quote! { where B: AsRef<starknet::core::types::BlockId> + Send + Sync },
        },
        ProviderMethod {
            name: syn::parse_str("get_state_update").unwrap(),
            params: quote! { <B>(&self, block_id: B) },
            return_type: quote! { Result<starknet::core::types::MaybePendingStateUpdate, starknet::providers::ProviderError> },
            where_clause: quote! { where B: AsRef<starknet::core::types::BlockId> + Send + Sync },
        },
        ProviderMethod {
            name: syn::parse_str("get_storage_at").unwrap(),
            params: quote! { <A, K, B>(&self, contract_address: A, key: K, block_id: B) },
            return_type: quote! { Result<starknet::core::types::Felt, starknet::providers::ProviderError> },
            where_clause: quote! { where A: AsRef<starknet::core::types::Felt> + Send + Sync, K: AsRef<starknet::core::types::Felt> + Send + Sync, B: AsRef<starknet::core::types::BlockId> + Send + Sync },
        },
        ProviderMethod {
            name: syn::parse_str("get_messages_status").unwrap(),
            params: quote! { (&self, transaction_hash: starknet::core::types::Hash256) },
            return_type: quote! { Result<Vec<starknet::core::types::MessageWithStatus>, starknet::providers::ProviderError> },
            where_clause: quote! {},
        },
        ProviderMethod {
            name: syn::parse_str("get_transaction_status").unwrap(),
            params: quote! { <H>(&self, transaction_hash: H) },
            return_type: quote! { Result<starknet::core::types::TransactionStatus, starknet::providers::ProviderError> },
            where_clause: quote! { where H: AsRef<starknet::core::types::Felt> + Send + Sync },
        },
        ProviderMethod {
            name: syn::parse_str("get_transaction_by_hash").unwrap(),
            params: quote! { <H>(&self, transaction_hash: H) },
            return_type: quote! { Result<starknet::core::types::Transaction, starknet::providers::ProviderError> },
            where_clause: quote! { where H: AsRef<starknet::core::types::Felt> + Send + Sync },
        },
        ProviderMethod {
            name: syn::parse_str("get_transaction_by_block_id_and_index").unwrap(),
            params: quote! { <B>(&self, block_id: B, index: u64) },
            return_type: quote! { Result<starknet::core::types::Transaction, starknet::providers::ProviderError> },
            where_clause: quote! { where B: AsRef<starknet::core::types::BlockId> + Send + Sync },
        },
        ProviderMethod {
            name: syn::parse_str("get_transaction_receipt").unwrap(),
            params: quote! { <H>(&self, transaction_hash: H) },
            return_type: quote! { Result<starknet::core::types::TransactionReceiptWithBlockInfo, starknet::providers::ProviderError> },
            where_clause: quote! { where H: AsRef<starknet::core::types::Felt> + Send + Sync },
        },
        ProviderMethod {
            name: syn::parse_str("get_class").unwrap(),
            params: quote! { <B, H>(&self, block_id: B, class_hash: H) },
            return_type: quote! { Result<starknet::core::types::ContractClass, starknet::providers::ProviderError> },
            where_clause: quote! { where B: AsRef<starknet::core::types::BlockId> + Send + Sync, H: AsRef<starknet::core::types::Felt> + Send + Sync },
        },
        ProviderMethod {
            name: syn::parse_str("get_class_hash_at").unwrap(),
            params: quote! { <B, A>(&self, block_id: B, contract_address: A) },
            return_type: quote! { Result<starknet::core::types::Felt, starknet::providers::ProviderError> },
            where_clause: quote! { where B: AsRef<starknet::core::types::BlockId> + Send + Sync, A: AsRef<starknet::core::types::Felt> + Send + Sync },
        },
        ProviderMethod {
            name: syn::parse_str("get_class_at").unwrap(),
            params: quote! { <B, A>(&self, block_id: B, contract_address: A) },
            return_type: quote! { Result<starknet::core::types::ContractClass, starknet::providers::ProviderError> },
            where_clause: quote! { where B: AsRef<starknet::core::types::BlockId> + Send + Sync, A: AsRef<starknet::core::types::Felt> + Send + Sync },
        },
        ProviderMethod {
            name: syn::parse_str("get_block_transaction_count").unwrap(),
            params: quote! { <B>(&self, block_id: B) },
            return_type: quote! { Result<u64, starknet::providers::ProviderError> },
            where_clause: quote! { where B: AsRef<starknet::core::types::BlockId> + Send + Sync },
        },
        ProviderMethod {
            name: syn::parse_str("call").unwrap(),
            params: quote! { <R, B>(&self, request: R, block_id: B) },
            return_type: quote! { Result<Vec<starknet::core::types::Felt>, starknet::providers::ProviderError> },
            where_clause: quote! { where R: AsRef<starknet::core::types::FunctionCall> + Send + Sync, B: AsRef<starknet::core::types::BlockId> + Send + Sync },
        },
        ProviderMethod {
            name: syn::parse_str("estimate_fee").unwrap(),
            params: quote! { <R, S, B>(&self, request: R, simulation_flags: S, block_id: B) },
            return_type: quote! { Result<Vec<starknet::core::types::FeeEstimate>, starknet::providers::ProviderError> },
            where_clause: quote! { where R: AsRef<[starknet::core::types::BroadcastedTransaction]> + Send + Sync, S: AsRef<[starknet::core::types::SimulationFlagForEstimateFee]> + Send + Sync, B: AsRef<starknet::core::types::BlockId> + Send + Sync },
        },
        ProviderMethod {
            name: syn::parse_str("estimate_message_fee").unwrap(),
            params: quote! { <M, B>(&self, message: M, block_id: B) },
            return_type: quote! { Result<starknet::core::types::FeeEstimate, starknet::providers::ProviderError> },
            where_clause: quote! { where M: AsRef<starknet::core::types::MsgFromL1> + Send + Sync, B: AsRef<starknet::core::types::BlockId> + Send + Sync },
        },
        ProviderMethod {
            name: syn::parse_str("block_number").unwrap(),
            params: quote! { (&self) },
            return_type: quote! { Result<u64, starknet::providers::ProviderError> },
            where_clause: quote! {},
        },
        ProviderMethod {
            name: syn::parse_str("block_hash_and_number").unwrap(),
            params: quote! { (&self) },
            return_type: quote! { Result<starknet::core::types::BlockHashAndNumber, starknet::providers::ProviderError> },
            where_clause: quote! {},
        },
        ProviderMethod {
            name: syn::parse_str("chain_id").unwrap(),
            params: quote! { (&self) },
            return_type: quote! { Result<starknet::core::types::Felt, starknet::providers::ProviderError> },
            where_clause: quote! {},
        },
        ProviderMethod {
            name: syn::parse_str("syncing").unwrap(),
            params: quote! { (&self) },
            return_type: quote! { Result<starknet::core::types::SyncStatusType, starknet::providers::ProviderError> },
            where_clause: quote! {},
        },
        ProviderMethod {
            name: syn::parse_str("get_events").unwrap(),
            params: quote! { (&self, filter: starknet::core::types::EventFilter, continuation_token: Option<String>, chunk_size: u64) },
            return_type: quote! { Result<starknet::core::types::EventsPage, starknet::providers::ProviderError> },
            where_clause: quote! {},
        },
        ProviderMethod {
            name: syn::parse_str("get_nonce").unwrap(),
            params: quote! { <B, A>(&self, block_id: B, contract_address: A) },
            return_type: quote! { Result<starknet::core::types::Felt, starknet::providers::ProviderError> },
            where_clause: quote! { where B: AsRef<starknet::core::types::BlockId> + Send + Sync, A: AsRef<starknet::core::types::Felt> + Send + Sync },
        },
        ProviderMethod {
            name: syn::parse_str("get_storage_proof").unwrap(),
            params: quote! { <B, H, A, K>(&self, block_id: B, class_hashes: H, contract_addresses: A, contracts_storage_keys: K) },
            return_type: quote! { Result<starknet::core::types::StorageProof, starknet::providers::ProviderError> },
            where_clause: quote! { where B: AsRef<starknet::core::types::ConfirmedBlockId> + Send + Sync, H: AsRef<[starknet::core::types::Felt]> + Send + Sync, A: AsRef<[starknet::core::types::Felt]> + Send + Sync, K: AsRef<[starknet::core::types::ContractStorageKeys]> + Send + Sync },
        },
        ProviderMethod {
            name: syn::parse_str("add_invoke_transaction").unwrap(),
            params: quote! { <I>(&self, invoke_transaction: I) },
            return_type: quote! { Result<starknet::core::types::InvokeTransactionResult, starknet::providers::ProviderError> },
            where_clause: quote! { where I: AsRef<starknet::core::types::BroadcastedInvokeTransaction> + Send + Sync },
        },
        ProviderMethod {
            name: syn::parse_str("add_declare_transaction").unwrap(),
            params: quote! { <D>(&self, declare_transaction: D) },
            return_type: quote! { Result<starknet::core::types::DeclareTransactionResult, starknet::providers::ProviderError> },
            where_clause: quote! { where D: AsRef<starknet::core::types::BroadcastedDeclareTransaction> + Send + Sync },
        },
        ProviderMethod {
            name: syn::parse_str("add_deploy_account_transaction").unwrap(),
            params: quote! { <D>(&self, deploy_account_transaction: D) },
            return_type: quote! { Result<starknet::core::types::DeployAccountTransactionResult, starknet::providers::ProviderError> },
            where_clause: quote! { where D: AsRef<starknet::core::types::BroadcastedDeployAccountTransaction> + Send + Sync },
        },
        ProviderMethod {
            name: syn::parse_str("trace_transaction").unwrap(),
            params: quote! { <H>(&self, transaction_hash: H) },
            return_type: quote! { Result<starknet::core::types::TransactionTrace, starknet::providers::ProviderError> },
            where_clause: quote! { where H: AsRef<starknet::core::types::Felt> + Send + Sync },
        },
        ProviderMethod {
            name: syn::parse_str("simulate_transactions").unwrap(),
            params: quote! { <B, T, S>(&self, block_id: B, transactions: T, simulation_flags: S) },
            return_type: quote! { Result<Vec<starknet::core::types::SimulatedTransaction>, starknet::providers::ProviderError> },
            where_clause: quote! { where B: AsRef<starknet::core::types::BlockId> + Send + Sync, T: AsRef<[starknet::core::types::BroadcastedTransaction]> + Send + Sync, S: AsRef<[starknet::core::types::SimulationFlag]> + Send + Sync },
        },
        ProviderMethod {
            name: syn::parse_str("trace_block_transactions").unwrap(),
            params: quote! { <B>(&self, block_id: B) },
            return_type: quote! { Result<Vec<starknet::core::types::TransactionTraceWithHash>, starknet::providers::ProviderError> },
            where_clause: quote! { where B: AsRef<starknet::core::types::BlockId> + Send + Sync },
        },
        ProviderMethod {
            name: syn::parse_str("batch_requests").unwrap(),
            params: quote! { <R>(&self, requests: R) },
            return_type: quote! { Result<Vec<starknet::providers::ProviderResponseData>, starknet::providers::ProviderError> },
            where_clause: quote! { where R: AsRef<[starknet::providers::ProviderRequestData]> + Send + Sync },
        },
        ProviderMethod {
            name: syn::parse_str("estimate_fee_single").unwrap(),
            params: quote! { <R, S, B>(&self, request: R, simulation_flags: S, block_id: B) },
            return_type: quote! { Result<starknet::core::types::FeeEstimate, starknet::providers::ProviderError> },
            where_clause: quote! { where R: AsRef<starknet::core::types::BroadcastedTransaction> + Send + Sync, S: AsRef<[starknet::core::types::SimulationFlagForEstimateFee]> + Send + Sync, B: AsRef<starknet::core::types::BlockId> + Send + Sync },
        },
        ProviderMethod {
            name: syn::parse_str("simulate_transaction").unwrap(),
            params: quote! { <B, T, S>(&self, block_id: B, transaction: T, simulation_flags: S) },
            return_type: quote! { Result<starknet::core::types::SimulatedTransaction, starknet::providers::ProviderError> },
            where_clause: quote! { where B: AsRef<starknet::core::types::BlockId> + Send + Sync, T: AsRef<starknet::core::types::BroadcastedTransaction> + Send + Sync, S: AsRef<[starknet::core::types::SimulationFlag]> + Send + Sync },
        },
    ]
}
