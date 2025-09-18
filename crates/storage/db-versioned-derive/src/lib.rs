use proc_macro::TokenStream;
use quote::quote;
use syn::{self, Data, DeriveInput, ItemEnum, ItemStruct};

mod entry;
mod generate;
mod parse;
mod utils;

use generate::{generate_enum_versioned, generate_struct_versioned};
use parse::VersionedInput;

/// Attribute macro for generating versioned type implementations for database compatibility.
///
/// # Struct Example
/// ```ignore
/// #[versioned(current = "katana_primitives::transaction")]
/// pub struct InvokeTxV3 {
///     pub chain_id: ChainId,
///
///     #[version(
///         v6 = "v6::ResourceBoundsMapping",
///         v7 = "v7::ResourceBoundsMapping"
///     )]
///     pub resource_bounds: ResourceBoundsMapping,
/// }
/// ```
///
/// This will generate the struct along with version-specific modules and From implementations.
#[proc_macro_attribute]
pub fn versioned(_attr: TokenStream, input: TokenStream) -> TokenStream {
    // Parse the input as a DeriveInput first to check what kind of item it is
    let input_clone = input.clone();
    let derive_input = syn::parse_macro_input!(input_clone as DeriveInput);

    let result = match derive_input.data {
        Data::Struct(ref data_struct) => {
            match VersionedInput::from_struct(&derive_input, data_struct) {
                Ok(versioned) => generate_struct_versioned(versioned, &derive_input),
                Err(err) => {
                    let err_tokens = err.to_compile_error();
                    let original = syn::parse_macro_input!(input as ItemStruct);
                    quote! {
                        #original
                        #err_tokens
                    }
                }
            }
        }
        Data::Enum(ref data_enum) => match VersionedInput::from_enum(&derive_input, data_enum) {
            Ok(versioned) => generate_enum_versioned(versioned, &derive_input),
            Err(err) => {
                let err_tokens = err.to_compile_error();
                let original = syn::parse_macro_input!(input as ItemEnum);
                quote! {
                    #original
                    #err_tokens
                }
            }
        },
        Data::Union(_) => {
            let err = syn::Error::new_spanned(
                &derive_input,
                "Versioned can only be used on structs and enums",
            )
            .to_compile_error();
            quote! {
                #err
            }
        }
    };

    TokenStream::from(result)
}
