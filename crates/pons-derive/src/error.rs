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
        let media_type = match error_media_type(&variant.original.attrs, variant_ident) {
            Ok(value) => value,
            Err(error) => return error.into_compile_error(),
        };
        let headers = match error_headers(&variant.original.attrs, variant_ident) {
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
                media_type: #media_type,
                body: #body_field,
                headers: #headers,
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

fn error_media_type(attrs: &[syn::Attribute], variant: &syn::Ident) -> syn::Result<TokenStream> {
    let Some(value) = attribute_value(attrs, "media_type") else {
        return Ok(quote!(None));
    };
    let literal: syn::LitStr = syn::parse2(value).map_err(|_| {
        syn::Error::new_spanned(variant, "ApiError media_type must be a string literal")
    })?;
    Ok(quote!(Some(#literal.to_owned())))
}

fn error_headers(attrs: &[syn::Attribute], variant: &syn::Ident) -> syn::Result<TokenStream> {
    let mut headers = Vec::new();
    for meta in pons_metas(attrs) {
        let presence = if meta.path().is_ident("header") {
            quote!(::pons_core::ir::Presence::Required)
        } else if meta.path().is_ident("optional_header") {
            quote!(::pons_core::ir::Presence::Optional)
        } else {
            continue;
        };
        let syn::Meta::NameValue(value) = &meta else {
            return Err(header_refusal(variant));
        };
        let syn::Expr::Path(spec) = &value.value else {
            return Err(header_refusal(variant));
        };
        headers.push(quote!(::pons_axum::error_header::<#spec>(cx, #presence)));
    }
    Ok(quote!(vec![#(#headers),*]))
}

fn header_refusal(variant: &syn::Ident) -> syn::Error {
    syn::Error::new_spanned(
        variant,
        "ApiError headers name the type that implements HeaderSpec, as in #[pons(header = TraceId)]",
    )
}

fn pons_metas(attrs: &[syn::Attribute]) -> Vec<syn::Meta> {
    let parser = syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated;
    attrs
        .iter()
        .filter(|attribute| attribute.path().is_ident("pons"))
        .filter_map(|attribute| match &attribute.meta {
            syn::Meta::List(list) => syn::parse::Parser::parse2(parser, list.tokens.clone()).ok(),
            _ => None,
        })
        .flatten()
        .collect()
}

fn attribute_value(attrs: &[syn::Attribute], key: &str) -> Option<TokenStream> {
    pons_metas(attrs).into_iter().find_map(|meta| {
        if !meta.path().is_ident(key) {
            return None;
        }
        match meta {
            syn::Meta::NameValue(value) => {
                let expression = value.value;
                Some(quote!(#expression))
            }
            syn::Meta::Path(_) if key == "retryable" => Some(quote!(true)),
            _ => None,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use syn::parse_quote;

    fn variant(attrs: Vec<syn::Attribute>) -> syn::Ident {
        let variant: syn::Variant = parse_quote! {
            #(#attrs)*
            Limited
        };
        variant.ident
    }

    fn expanded(tokens: TokenStream) -> String {
        tokens.to_string().replace(' ', "")
    }

    #[test]
    fn a_media_type_is_the_string_the_variant_names() {
        let media_type = error_media_type(
            &[parse_quote!(#[pons(status = 429, media_type = "application/problem+json")])],
            &variant(Vec::new()),
        )
        .unwrap();
        assert_eq!(
            expanded(media_type),
            expanded(quote!(Some("application/problem+json".to_owned())))
        );
        assert_eq!(
            expanded(error_media_type(&[], &variant(Vec::new())).unwrap()),
            expanded(quote!(None))
        );
    }

    #[test]
    fn a_media_type_that_is_no_string_literal_is_a_diagnostic() {
        let error = error_media_type(
            &[parse_quote!(#[pons(media_type = 42)])],
            &variant(Vec::new()),
        )
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            "ApiError media_type must be a string literal"
        );
    }

    #[test]
    fn a_header_names_the_spec_and_the_presence_it_is_written_with() {
        let headers = error_headers(
            &[
                parse_quote!(#[pons(header = TraceId)]),
                parse_quote!(#[pons(optional_header = RetryAfter)]),
            ],
            &variant(Vec::new()),
        )
        .unwrap();
        assert_eq!(
            expanded(headers),
            expanded(quote! {
                vec![
                    ::pons_axum::error_header::<TraceId>(cx, ::pons_core::ir::Presence::Required),
                    ::pons_axum::error_header::<RetryAfter>(cx, ::pons_core::ir::Presence::Optional)
                ]
            })
        );
        assert_eq!(
            expanded(error_headers(&[], &variant(Vec::new())).unwrap()),
            expanded(quote!(vec![]))
        );
    }

    #[test]
    fn a_header_that_names_no_type_is_a_diagnostic() {
        for attribute in [
            parse_quote!(#[pons(header = "Retry-After")]),
            parse_quote!(#[pons(header)]),
        ] {
            let error = error_headers(&[attribute], &variant(Vec::new())).unwrap_err();
            assert_eq!(
                error.to_string(),
                "ApiError headers name the type that implements HeaderSpec, as in #[pons(header = TraceId)]"
            );
        }
    }
}
