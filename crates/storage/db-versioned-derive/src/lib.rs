use proc_macro::TokenStream;
use quote::quote;
use syn::{self, Data, DeriveInput};

mod parse;
mod generate;
mod utils;

use parse::VersionedInput;
use generate::{generate_struct_impls, generate_enum_impls};

/// Derives versioned type implementations for database compatibility.
///
/// # Struct Example
/// ```ignore
/// #[derive(Versioned)]
/// #[versioned(current = "katana_primitives::transaction")]
/// pub struct InvokeTxV3 {
///     pub chain_id: ChainId,
///     
///     #[versioned(
///         v6 = "v6::ResourceBoundsMapping",
///         v7 = "v7::ResourceBoundsMapping"
///     )]
///     pub resource_bounds: ResourceBoundsMapping,
/// }
/// ```
///
/// This will generate version-specific modules with structs and From implementations.
#[proc_macro_derive(Versioned, attributes(versioned))]
pub fn derive_versioned(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as DeriveInput);
    
    let result = match input.data {
        Data::Struct(ref data_struct) => {
            match VersionedInput::from_struct(&input, data_struct) {
                Ok(versioned) => generate_struct_impls(versioned),
                Err(err) => err.to_compile_error(),
            }
        }
        Data::Enum(ref data_enum) => {
            match VersionedInput::from_enum(&input, data_enum) {
                Ok(versioned) => generate_enum_impls(versioned),
                Err(err) => err.to_compile_error(),
            }
        }
        Data::Union(_) => {
            syn::Error::new_spanned(&input, "Versioned can only be derived for structs and enums")
                .to_compile_error()
        }
    };
    
    TokenStream::from(result)
}