use proc_macro2::TokenStream;
use quote::quote;
use serde_derive_internals::{Ctxt, Derive, ast};
use syn::{DeriveInput, parse_quote};

use crate::schema;

pub(super) fn expand(input: &DeriveInput) -> TokenStream {
    let cx = Ctxt::new();
    let Some(container) = ast::Container::from_ast(&cx, input, Derive::Serialize, &input.ident)
    else {
        return cx
            .check()
            .err()
            .map(|error| error.into_compile_error())
            .unwrap_or_default();
    };
    let ast::Data::Enum(variants) = &container.data else {
        return syn::Error::new_spanned(input, "ApiError can only be derived for enums")
            .into_compile_error();
    };
    if let Some(errors) = cx.check().err() {
        return errors.into_compile_error();
    }

    let ident = &container.ident;
    let mut generics = container.generics.clone();
    let type_params: Vec<_> = generics
        .type_params()
        .map(|param| param.ident.clone())
        .collect();
    for ty in type_params {
        generics
            .make_where_clause()
            .predicates
            .push(parse_quote!(#ty: ::pons_core::Schema));
    }
    let (impl_generics, type_generics, where_clause) = generics.split_for_impl();
    let definitions = variants.iter().map(|variant| {
        let variant_ident = &variant.ident;
        let wire = variant.attrs.name().serialize_name().to_string();
        let error_name = schema::words(&variant.ident.to_string());
        let variant_wire = syn::LitStr::new(&wire, variant.ident.span());
        let doc = schema::doc_value(&variant.original.attrs);
        let retryable = attribute_value(&variant.original.attrs, "retryable");
        let status = match error_status(&variant.original.attrs, variant_ident) {
            Ok(status) => status,
            Err(error) => return error.into_compile_error(),
        };
        let retryable = match retryable {
            Some(value) => quote!(Some(#value)),
            None => quote!(None),
        };
        let (content, body_field) = match error_content(&variant.original.attrs, variant_ident) {
            Ok(value) => value,
            Err(error) => return error.into_compile_error(),
        };
        quote!({
            let mut body = reference.clone();
            if let ::pons_core::ir::TypeRef::Named(named) = &mut body {
                named.variant = Some(#variant_wire.to_owned());
            }
            ::pons_core::ir::ErrorResponse {
                status: #status,
                name: #error_name,
                content: #content,
                media_type: None,
                body: #body_field,
                headers: Vec::new(),
                retryable: #retryable,
                doc: #doc,
                ext: Default::default(),
            }
        })
    });
    quote! {
        impl #impl_generics ::pons_core::ErrorSpec for #ident #type_generics #where_clause {
            fn errors(cx: &mut ::pons_core::TypeCx) -> Vec<::pons_core::ir::ErrorResponse> {
                let reference = <Self as ::pons_core::Schema>::schema(cx, ::pons_core::ir::Direction::Output);
                vec![#(#definitions),*]
            }
        }
    }
}

fn error_status(attrs: &[syn::Attribute], variant: &syn::Ident) -> syn::Result<TokenStream> {
    let Some(value) = attribute_value(attrs, "status") else {
        return Err(syn::Error::new_spanned(
            variant,
            "ApiError variants require #[pons(status = 400)]",
        ));
    };
    if let Ok(status) = syn::parse2::<syn::LitInt>(value.clone()) {
        return Ok(quote!(::pons_core::ir::ErrorStatus::Exact(#status)));
    }
    let literal: syn::LitStr = syn::parse2(value).map_err(|_| {
        syn::Error::new_spanned(
            variant,
            "ApiError status must be an integer or status class",
        )
    })?;
    match literal.value().as_str() {
        "default" => Ok(quote!(::pons_core::ir::ErrorStatus::Default)),
        value if value.len() == 3 && value.ends_with("xx") => {
            let class = value.as_bytes()[0] - b'0';
            if (1..=5).contains(&class) {
                Ok(quote!(::pons_core::ir::ErrorStatus::Class(#class)))
            } else {
                Err(syn::Error::new_spanned(
                    variant,
                    "ApiError status class must be 1xx through 5xx",
                ))
            }
        }
        _ => Err(syn::Error::new_spanned(
            variant,
            "ApiError status must be an integer, 1xx through 5xx, or default",
        )),
    }
}

fn error_content(
    attrs: &[syn::Attribute],
    variant: &syn::Ident,
) -> syn::Result<(TokenStream, TokenStream)> {
    let Some(value) = attribute_value(attrs, "content") else {
        return Ok((
            quote!(::pons_core::ir::ErrorContent::Json),
            quote!(Some(body)),
        ));
    };
    let literal: syn::LitStr = syn::parse2(value).map_err(|_| {
        syn::Error::new_spanned(variant, "ApiError content must be a string literal")
    })?;
    let (content, has_body) = match literal.value().as_str() {
        "json" => (quote!(::pons_core::ir::ErrorContent::Json), true),
        "problem_json" => (quote!(::pons_core::ir::ErrorContent::ProblemJson), true),
        "text" => (quote!(::pons_core::ir::ErrorContent::Text), true),
        "empty" => (quote!(::pons_core::ir::ErrorContent::Empty), false),
        _ => {
            return Err(syn::Error::new_spanned(
                variant,
                "ApiError content must be json, problem_json, text, or empty",
            ));
        }
    };
    let body = if has_body {
        quote!(Some(body))
    } else {
        quote!(None)
    };
    Ok((content, body))
}

fn attribute_value(attrs: &[syn::Attribute], key: &str) -> Option<TokenStream> {
    for attribute in attrs
        .iter()
        .filter(|attribute| attribute.path().is_ident("pons"))
    {
        let syn::Meta::List(list) = &attribute.meta else {
            continue;
        };
        let parser = syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated;
        let parsed = syn::parse::Parser::parse2(parser, list.tokens.clone()).ok()?;
        for meta in parsed {
            if !meta.path().is_ident(key) {
                continue;
            }
            return match meta {
                syn::Meta::NameValue(value) => {
                    let expression = value.value;
                    Some(quote!(#expression))
                }
                syn::Meta::Path(_) if key == "retryable" => Some(quote!(true)),
                _ => None,
            };
        }
    }
    None
}
