use proc_macro2::Span;
use syn::{Ident, Path, Type};

/// Constructs a Type from a path and a type name
pub fn type_from_path(base_path: &Path, type_name: &str) -> Type {
    let mut path = base_path.clone();
    path.segments.push(syn::PathSegment {
        ident: Ident::new(type_name, Span::call_site()),
        arguments: syn::PathArguments::None,
    });
    
    Type::Path(syn::TypePath {
        qself: None,
        path,
    })
}

/// Parses a string into a Path
pub fn parse_path(s: &str) -> syn::Result<Path> {
    syn::parse_str::<Path>(s)
}