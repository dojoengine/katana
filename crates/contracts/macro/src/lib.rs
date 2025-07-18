use std::path::PathBuf;
use std::str::FromStr;

use katana_primitives::class::ContractClass;
use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, Ident, LitStr};

/// A proc macro that generates contract wrapper structs with compile-time computed hashes.
///
/// # Usage
///
/// ```rust
/// contract!(ContractName, "path/to/contract.json");
/// ```
#[proc_macro]
pub fn contract(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as ContractInput);

    match generate_contract_impl(&input) {
        Ok(tokens) => tokens.into(),
        Err(err) => syn::Error::new(proc_macro2::Span::call_site(), err).to_compile_error().into(),
    }
}

/// Input structure for the contract macro
struct ContractInput {
    name: Ident,
    artifact_path: PathBuf,
    crate_path: syn::Path,
}

impl syn::parse::Parse for ContractInput {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        // first argument - struct name
        let name = input.parse::<Ident>()?;
        input.parse::<syn::Token![,]>()?;

        // second argument - artifact path
        let str = input.parse::<LitStr>()?.value();
        let artifact_path = PathBuf::from(str);

        // third argument (optional) - crate path
        let crate_path = if input.peek(syn::Token![,]) {
            input.parse::<syn::Token![,]>()?;
            input.parse::<syn::Path>()?
        } else {
            syn::parse_str::<syn::Path>(crate_path())?
        };

        Ok(ContractInput { crate_path, name, artifact_path })
    }
}

fn generate_contract_impl(input: &ContractInput) -> Result<proc_macro2::TokenStream, String> {
    let contract_content = std::fs::read_to_string(&input.artifact_path).map_err(|error| {
        format!("Failed to read contract file '{}': {error}", input.artifact_path.display())
    })?;

    // Parse the contract class
    let contract_class = ContractClass::from_str(&contract_content)
        .map_err(|error| format!("Failed to parse contract class: {error}"))?;

    // Compute class hash
    let class_hash =
        contract_class.class_hash().map_err(|e| format!("Failed to compute class hash: {}", e))?;

    // Compile and compute compiled class hash
    let compiled_class =
        contract_class.compile().map_err(|e| format!("Failed to compile contract class: {}", e))?;

    let compiled_class_hash = compiled_class
        .class_hash()
        .map_err(|e| format!("Failed to compute compiled class hash: {}", e))?;

    // Convert Felt values to string representation for const generation
    let class_hash_str = format!("{class_hash:#x}",);
    let compiled_class_hash_str = format!("{compiled_class_hash:#x}",);

    let crate_path = &input.crate_path;
    let contract_name = &input.name;
    let contract_path = input.artifact_path.to_string_lossy().to_string();

    // Generate the contract implementation
    let expanded = quote! {
        pub struct #contract_name;

        impl #contract_name {
            /// The contract class hash as a compile-time constant.
            pub const HASH: ::katana_primitives::class::ClassHash = ::katana_primitives::felt!(#class_hash_str);
            /// The compiled class hash as a compile-time constant.
            pub const CASM_HASH: ::katana_primitives::class::CompiledClassHash = ::katana_primitives::felt!(#compiled_class_hash_str);

            /// Returns the Sierra class artifact.
            pub fn class() -> katana_primitives::class::ContractClass {
                #crate_path::ClassArtifact::file(::std::path::PathBuf::from(#contract_path)).class().unwrap()
            }

            /// Returns the compiled CASM class.
            pub fn casm() -> katana_primitives::class::CompiledClass {
                #crate_path::ClassArtifact::file(::std::path::PathBuf::from(#contract_path)).casm().unwrap()
            }
        }
    };

    Ok(expanded)
}

fn crate_path() -> &'static str {
    "::katana_contracts"
}

#[cfg(test)]
mod tests {
    use syn::parse_quote;

    use super::*;

    #[test]
    fn test_contract_input_parsing() {
        let input: ContractInput = parse_quote! {
            TestContract, "path/to/contract.json"
        };

        assert_eq!(input.name.to_string(), "TestContract");
        assert_eq!(input.artifact_path, PathBuf::from("path/to/contract.json"));
    }
}
