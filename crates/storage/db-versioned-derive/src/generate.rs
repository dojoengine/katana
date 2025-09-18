use proc_macro2::{Ident, Span, TokenStream};
use quote::{format_ident, quote};
use std::collections::HashMap;
use syn::{Fields, Path, Type};

use crate::parse::{VersionedInput, VersionedKind, VersionedStruct, VersionedEnum, VersionedField};
use crate::utils;

pub fn generate_struct_impls(input: VersionedInput) -> TokenStream {
    let VersionedKind::Struct(ref versioned_struct) = input.kind else {
        panic!("Expected struct");
    };
    
    let struct_name = &input.ident;
    let vis = &input.vis;
    
    // Generate the current/latest version struct (with derives)
    let current_struct = generate_current_struct(&input, &versioned_struct);
    
    // Generate version-specific modules
    let version_modules = input.versions.iter().map(|version| {
        generate_version_module(version, &input, versioned_struct)
    });
    
    quote! {
        #current_struct
        
        #(#version_modules)*
    }
}

pub fn generate_enum_impls(input: VersionedInput) -> TokenStream {
    let VersionedKind::Enum(ref versioned_enum) = input.kind else {
        panic!("Expected enum");
    };
    
    let enum_name = &input.ident;
    let vis = &input.vis;
    
    // For enums, generate the enum definition with derives
    let variants = versioned_enum.variants.iter().map(|v| {
        let ident = &v.ident;
        let fields = &v.fields;
        quote! { #ident #fields }
    });
    
    let current_enum = quote! {
        #[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
        #[cfg_attr(test, derive(::arbitrary::Arbitrary))]
        #vis enum #enum_name {
            #(#variants),*
        }
    };
    
    // Generate From implementation if current path is specified
    let from_impl = if let Some(ref current_path) = input.current_path {
        let target_type = utils::type_from_path(current_path, &enum_name.to_string());
        generate_enum_from_impl(&enum_name, &target_type, &versioned_enum)
    } else {
        quote! {}
    };
    
    quote! {
        #current_enum
        #from_impl
    }
}

fn generate_current_struct(input: &VersionedInput, versioned_struct: &VersionedStruct) -> TokenStream {
    let struct_name = &input.ident;
    let vis = &input.vis;
    
    let fields = versioned_struct.fields.iter().map(|f| {
        let field_name = &f.ident;
        let field_vis = &f.vis;
        let field_ty = &f.ty;
        
        quote! {
            #field_vis #field_name: #field_ty
        }
    });
    
    quote! {
        #[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
        #[cfg_attr(test, derive(::arbitrary::Arbitrary))]
        #vis struct #struct_name {
            #(#fields),*
        }
    }
}

fn generate_version_module(
    version: &str,
    input: &VersionedInput,
    versioned_struct: &VersionedStruct,
) -> TokenStream {
    let module_name = format_ident!("{}", version);
    let struct_name = &input.ident;
    let vis = &input.vis;
    
    // Generate fields for this version
    let fields: Vec<TokenStream> = versioned_struct.fields.iter().filter_map(|f| {
        // Skip fields added after this version
        if let Some(ref added) = f.added_in {
            if version_number(added) > version_number(version) {
                return None;
            }
        }
        
        // Skip fields removed before or at this version
        if let Some(ref removed) = f.removed_after {
            if version_number(version) > version_number(removed) {
                return None;
            }
        }
        
        let field_name = &f.ident;
        let field_vis = &f.vis;
        
        // Use version-specific type if specified
        let field_ty = if let Some(version_type) = f.versions.get(version) {
            let ty_path: Path = syn::parse_str(version_type).unwrap_or_else(|_| {
                panic!("Invalid type path: {}", version_type)
            });
            quote! { #ty_path }
        } else {
            let ty = &f.ty;
            quote! { #ty }
        };
        
        Some(quote! {
            #field_vis #field_name: #field_ty
        })
    }).collect();
    
    // If no fields changed for this version, skip generating the module
    let has_version_specific_types = versioned_struct.fields.iter().any(|f| {
        f.versions.contains_key(version) || 
        f.added_in.as_ref().map_or(false, |v| v == version) ||
        f.removed_after.as_ref().map_or(false, |v| v == version)
    });
    
    if !has_version_specific_types {
        return quote! {};
    }
    
    // Generate From implementation
    let from_impl = if let Some(ref current_path) = input.current_path {
        let target_type = utils::type_from_path(current_path, &struct_name.to_string());
        generate_struct_from_impl(&module_name, &struct_name, &target_type, versioned_struct, version)
    } else {
        quote! {}
    };
    
    quote! {
        pub mod #module_name {
            use super::*;
            
            #[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
            #[cfg_attr(test, derive(::arbitrary::Arbitrary))]
            #vis struct #struct_name {
                #(#fields),*
            }
            
            #from_impl
        }
    }
}

fn generate_struct_from_impl(
    module_name: &Ident,
    struct_name: &Ident,
    target_type: &Type,
    versioned_struct: &VersionedStruct,
    version: &str,
) -> TokenStream {
    let field_conversions: Vec<TokenStream> = versioned_struct.fields.iter().filter_map(|f| {
        // Skip fields not in this version
        if let Some(ref added) = f.added_in {
            if version_number(added) > version_number(version) {
                return None;
            }
        }
        if let Some(ref removed) = f.removed_after {
            if version_number(version) > version_number(removed) {
                return None;
            }
        }
        
        let field_name = &f.ident;
        Some(quote! {
            #field_name: versioned.#field_name.into()
        })
    }).collect();
    
    // Handle fields added in later versions (provide defaults)
    let default_fields: Vec<TokenStream> = versioned_struct.fields.iter().filter_map(|f| {
        if let Some(ref added) = f.added_in {
            if version_number(added) > version_number(version) {
                let field_name = &f.ident;
                // For now, use Default::default() - could be enhanced with custom defaults
                return Some(quote! {
                    #field_name: Default::default()
                });
            }
        }
        None
    }).collect();
    
    quote! {
        impl From<#struct_name> for #target_type {
            fn from(versioned: #struct_name) -> Self {
                Self {
                    #(#field_conversions,)*
                    #(#default_fields,)*
                }
            }
        }
    }
}

fn generate_enum_from_impl(
    enum_name: &Ident,
    target_type: &Type,
    versioned_enum: &VersionedEnum,
) -> TokenStream {
    let variant_conversions = versioned_enum.variants.iter().map(|v| {
        let variant_name = &v.ident;
        
        match &v.fields {
            Fields::Unit => quote! {
                #enum_name::#variant_name => #target_type::#variant_name
            },
            Fields::Unnamed(_) => quote! {
                #enum_name::#variant_name(inner) => #target_type::#variant_name(inner.into())
            },
            Fields::Named(_) => quote! {
                #enum_name::#variant_name { .. } => todo!("Named enum fields not yet supported")
            },
        }
    });
    
    quote! {
        impl From<#enum_name> for #target_type {
            fn from(versioned: #enum_name) -> Self {
                match versioned {
                    #(#variant_conversions),*
                }
            }
        }
    }
}

fn version_number(version: &str) -> u32 {
    version.trim_start_matches('v').parse().unwrap_or(0)
}