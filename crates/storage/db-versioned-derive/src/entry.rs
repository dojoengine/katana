use proc_macro2::TokenStream;

use crate::utils::token_stream_with_error;

// Because syn::AttributeArgs does not implement syn::Parse
pub type AttributeArgs = syn::punctuated::Punctuated<syn::Meta, syn::Token![,]>;

pub(crate) fn versioned(args: TokenStream, item: TokenStream) -> TokenStream {
    let input: syn::ItemTrait = match syn::parse2(item.clone()) {
        Ok(it) => it,
        Err(e) => return token_stream_with_error(item, e),
    };

    todo!()
}
