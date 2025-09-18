use proc_macro2::{Span, TokenStream};
use syn::{Attribute, Ident, Path, Type};

/// Constructs a Type from a path and a type name
pub fn type_from_path(base_path: &Path, type_name: &str) -> Type {
    let mut path = base_path.clone();
    path.segments.push(syn::PathSegment {
        ident: Ident::new(type_name, Span::call_site()),
        arguments: syn::PathArguments::None,
    });

    Type::Path(syn::TypePath { qself: None, path })
}

pub fn find_attr<'a>(attrs: &'a [Attribute], ident: &str) -> Option<&'a Attribute> {
    attrs.iter().find(|a| a.path().is_ident(ident))
}

pub fn token_stream_with_error(mut tokens: TokenStream, error: syn::Error) -> TokenStream {
    tokens.extend(error.into_compile_error());
    tokens
}
