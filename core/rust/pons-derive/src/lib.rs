mod error;
mod schema;

use proc_macro::TokenStream;
use serde_derive_internals::{Ctxt, Derive, ast};
use syn::parse_macro_input;

#[proc_macro_derive(Schema, attributes(serde, pons))]
pub fn derive_schema(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as syn::DeriveInput);
    let cx = Ctxt::new();
    let parsed = ast::Container::from_ast(&cx, &input, Derive::Serialize, &input.ident);
    let errors = cx.check().err().map(|error| error.into_compile_error());
    match (parsed, errors) {
        (Some(parsed), None) => schema::expand(&parsed).into(),
        (_, Some(errors)) => errors.into(),
        (None, None) => TokenStream::new(),
    }
}

#[proc_macro_derive(ApiError, attributes(serde, pons))]
pub fn derive_api_error(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as syn::DeriveInput);
    error::expand(&input).into()
}
