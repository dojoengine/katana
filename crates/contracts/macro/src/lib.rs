use std::path::PathBuf;
use std::str::FromStr;

use katana_primitives::class::ContractClass;
use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, Ident, LitStr};

/// A proc macro that generates contract wrapper structs with compile-time computed hashes.
///
/// This is a helper macro to guarantees that the generated hashes (i.e., sierra and casm hashes)
/// are correct and can be accessed in a const context.
///
/// # Usage
///
/// ```rust
/// contract!(ContractName, "path/to/contract.json");
/// ```
///
/// # Path Resolution
///
/// The macro supports the `{CARGO_MANIFEST_DIR}` placeholder to ensure paths are resolved
/// correctly when the macro is invoked from downstream crates:
///
/// ```rust
/// contract!(LegacyERC20, "{CARGO_MANIFEST_DIR}/build/legacy/erc20.json");
/// ```
///
/// This will resolve to the directory containing your `Cargo.toml` file at compile time.
/// The `{CARGO_MANIFEST_DIR}` placeholder is particularly useful when this macro is used
/// in a crate that is a dependency of another crate, as it ensures the path is always
/// relative to the crate where the macro is used, not the downstream crate.
///
/// For example, if your project structure is:
///
/// ```
/// my-project/
/// ├── Cargo.toml
/// ├── src/
/// │   └── lib.rs
/// └── build/
///     └── legacy/
///         └── erc20.json
/// ```
///
/// And `CARGO_MANIFEST_DIR` is `/home/user/my-project`, then:
/// - `"{CARGO_MANIFEST_DIR}/build/legacy/erc20.json"` resolves to
///   `/home/user/my-project/build/legacy/erc20.json`
#[proc_macro]
pub fn contract(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as ContractInput);

    match generate_contract_impl(&input) {
        Ok(tokens) => tokens.into(),
        Err(err) => syn::Error::new(proc_macro2::Span::call_site(), err).to_compile_error().into(),
    }
}

/// Input structure for the contract! macro.
struct ContractInput {
    /// The name of the contract.
    ///
    /// This will be used as the identifier for the generated contract struct.
    name: Ident,
    /// The absolute path to the contract artifact.
    artifact_path: PathBuf,
}

impl syn::parse::Parse for ContractInput {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        // first argument - struct name
        let name = input.parse::<Ident>()?;
        input.parse::<syn::Token![,]>()?;

        // second argument - artifact path
        let str = input.parse::<LitStr>()?.value();

        // Check if the literal string path starts with {CARGO_MANIFEST_DIR}
        let artifact_path = if let Some(stripped_path) = str.strip_prefix("{CARGO_MANIFEST_DIR}") {
            let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").map_err(|_| {
                syn::Error::new(input.span(), "`CARGO_MANIFEST_DIR` environment variable not set")
            })?;

            PathBuf::from(manifest_dir).join(stripped_path.trim_start_matches('/'))
        } else {
            PathBuf::from(str)
        };

        let abs_artifact_path = artifact_path.canonicalize().map_err(|error| {
            syn::Error::new(
                input.span(),
                format!(
                    "failed to canonicalize artifact path {}: {error}",
                    artifact_path.display()
                ),
            )
        })?;

        Ok(ContractInput { name, artifact_path: abs_artifact_path })
    }
}

fn generate_contract_impl(input: &ContractInput) -> Result<proc_macro2::TokenStream, String> {
    let contract_content = std::fs::read_to_string(&input.artifact_path).map_err(|error| {
        format!("failed to read contract file '{}': {error}", input.artifact_path.display())
    })?;

    // Parse the contract class
    let contract_class = ContractClass::from_str(&contract_content)
        .map_err(|error| format!("failed to parse contract class: {error}"))?;

    // Compute class hash
    let class_hash =
        contract_class.class_hash().map_err(|e| format!("failed to compute class hash: {e}"))?;

    // Compile and compute compiled class hash
    let compiled_class =
        contract_class.compile().map_err(|e| format!("failed to compile contract class: {e}"))?;

    let compiled_class_hash =
        compiled_class.class_hash().map_err(|e| format!("failed to compute casm hash: {e}"))?;

    // Convert Felt values to string representation for const generation
    let class_hash_str = format!("{class_hash:#x}",);
    let compiled_class_hash_str = format!("{compiled_class_hash:#x}",);

    let contract_name = &input.name;
    let contract_path = input.artifact_path.to_string_lossy().to_string();
    let static_class_name = syn::parse_str::<Ident>(&format!("{contract_name}_CLASS")).unwrap();

    // Generate the contract implementation
    let expanded = quote! {
        pub struct #contract_name;

        impl #contract_name {
            #[doc = #class_hash_str]
            pub const HASH: ::katana_primitives::class::ClassHash = ::katana_primitives::felt!(#class_hash_str);
            #[doc = #compiled_class_hash_str]
            pub const CASM_HASH: ::katana_primitives::class::CompiledClassHash = ::katana_primitives::felt!(#compiled_class_hash_str);
            pub const CLASS: #static_class_name = #static_class_name { __private_field: () };
        }

        #[allow(missing_copy_implementations)]
        #[allow(non_camel_case_types)]
        #[allow(dead_code)]
        pub struct #static_class_name {
            __private_field: (),
        }
        impl ::lazy_static::__Deref for #static_class_name {
            type Target = ::katana_primitives::class::ContractClass;
            fn deref(&self) -> &::katana_primitives::class::ContractClass {
                #[inline(always)]
                fn __static_ref_initialize() -> ::katana_primitives::class::ContractClass {
                    use ::std::str::FromStr;
                    (::katana_primitives::class::ContractClass::from_str(include_str!(#contract_path)).unwrap())
                }
                #[inline(always)]
                fn __stability() -> &'static ::katana_primitives::class::ContractClass {
                    static LAZY: lazy_static::lazy::Lazy<::katana_primitives::class::ContractClass> = lazy_static::lazy::Lazy::INIT;
                    LAZY.get(__static_ref_initialize)
                }
                __stability()
            }
        }
        impl ::lazy_static::LazyStatic for #static_class_name {
            fn initialize(lazy: &Self) {
                let _ = &**lazy;
            }
        }
    };

    Ok(expanded)
}
