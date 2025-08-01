use proc_macro2::TokenStream;
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::{parse_quote, Expr, ItemTrait};

pub fn starknet_rpc(args: TokenStream, input: TokenStream) -> TokenStream {
    // If any of the steps for this macro fail, we still want to expand to an item that is as close
    // to the expected output as possible. This helps out IDEs such that completions and other
    // related features keep working.

    let mut item_trait: ItemTrait = match syn::parse2(input.clone()) {
        Ok(item_trait) => item_trait,
        Err(err) => return token_stream_with_error(input, err),
    };

    let args: Arguments = match syn::parse2(args.clone()) {
        Ok(args) => args,
        Err(err) => return token_stream_with_error(input, err),
    };

    // Generate the jsonrpsee rpc attributes based on specified modes
    let namespace = &args.namespace;
    let rpc_attrs = match (args.need_server, args.need_client) {
        (true, true) | (false, false) => quote! {
            #[rpc(client, server, namespace = #namespace)]
        },

        (true, _) => quote! {
            #[rpc(server, namespace = #namespace)]
        },

        (_, true) => quote! {
            #[rpc(client, namespace = #namespace)]
        },
    };

    // Generate the output with version constant and modified trait
    let version = &args.spec_version;

    // Add the spec_version method to the trait if it doesn't exist
    if !has_spec_version_method(&item_trait) {
        let spec_version_method = parse_quote! {
            /// Returns the version of the Starknet JSON-RPC specification being used.
            #[method(name = "specVersion")]
            async fn spec_version(&self) -> RpcResult<String> {
                Ok(#version.into())
            }
        };

        item_trait.items.push(spec_version_method);
    }

    quote! {
        #rpc_attrs
        #item_trait
    }
}

/// Arguments for the starknet_rpc proc macro
struct Arguments {
    /// The namespace of the RPC API
    namespace: Expr,
    /// Whether to generate a server implementation
    need_server: bool,
    /// Whether to generate a client implementation
    need_client: bool,
    /// The version of the JSON-RPC specification being used
    ///
    /// This will be the version returned by the `namespace_specVersion` method.
    spec_version: Expr,
}

impl Parse for Arguments {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        // Because syn::AttributeArgs does not implement syn::Parse
        type AttributeArgs = syn::punctuated::Punctuated<syn::Meta, syn::Token![,]>;
        // parse the attribute arguments
        let args = AttributeArgs::parse_terminated(input)?;

        let mut need_server = false;
        let mut need_client = false;
        let mut spec_version = None;
        let mut namespace = None;

        const VALID_ARGS: [&str; 4] = ["server", "client", "version", "namespace"];

        for arg in args {
            match arg {
                syn::Meta::List(list) => {
                    let ident = list.path.get_ident().map(|ident| ident.to_string().to_lowercase());
                    let Some(ident) = ident else {
                        return Err(syn::Error::new_spanned(&list, "must have specified ident"));
                    };

                    let ident = ident.as_str();

                    if !VALID_ARGS.contains(&ident) {}

                    match ident {
                        "server" => {
                            if need_server {
                                return Err(syn::Error::new_spanned(
                                    &list,
                                    "`server` set multiple times",
                                ));
                            }

                            need_server = true;
                        }

                        "client" => {
                            if need_client {
                                return Err(syn::Error::new_spanned(
                                    &list,
                                    "`client` set multiple times",
                                ));
                            }

                            need_client = true;
                        }

                        "version" => {}

                        "namespace" => {}

                        _ => {
                            return Err(syn::Error::new_spanned(
                                &list,
                                format!(
                                    "unknown attribute `{}` is specified; expected one of: {}",
                                    ident,
                                    VALID_ARGS.join(", ")
                                ),
                            ));
                        }
                    }
                }

                syn::Meta::NameValue(nv) => {
                    let ident = nv.path.get_ident().map(|ident| ident.to_string().to_lowercase());
                    let Some(ident) = ident else {
                        return Err(syn::Error::new_spanned(&nv, "must have specified ident"));
                    };

                    let ident = ident.as_str();

                    match ident {
                        "server" => {
                            // Handle server attribute
                        }

                        "client" => {
                            // Handle client attribute
                        }

                        "version" => {
                            if spec_version.is_some() {
                                return Err(syn::Error::new_spanned(
                                    &nv,
                                    "`version` set multiple times",
                                ));
                            }

                            spec_version = Some(nv.value);
                        }

                        "namespace" => {
                            if namespace.is_some() {
                                return Err(syn::Error::new_spanned(
                                    &nv,
                                    "`namespace` set multiple times",
                                ));
                            }

                            namespace = Some(nv.value);
                        }

                        _ => {
                            return Err(syn::Error::new_spanned(
                                &nv,
                                format!(
                                    "unknown attribute `{}` is specified; expected one of: {}",
                                    ident,
                                    VALID_ARGS.join(", ")
                                ),
                            ));
                        }
                    }
                }

                other => {
                    return Err(syn::Error::new_spanned(
                        other,
                        format!(
                            "unknown attribute specified; expected one of: {}",
                            VALID_ARGS.join(", ")
                        ),
                    ));
                }
            }
        }

        Ok(Self {
            need_client,
            need_server,
            namespace: namespace.unwrap(),
            spec_version: spec_version.unwrap(),
        })
    }
}

/// Check if the trait already has a spec_version method
fn has_spec_version_method(trait_item: &ItemTrait) -> bool {
    trait_item.items.iter().any(|item| {
        if let syn::TraitItem::Fn(method) = item {
            method.sig.ident == "spec_version"
        } else {
            false
        }
    })
}

fn token_stream_with_error(mut tokens: TokenStream, error: syn::Error) -> TokenStream {
    tokens.extend(error.into_compile_error());
    tokens
}
