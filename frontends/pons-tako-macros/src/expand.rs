use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};
use syn::{Ident, ItemFn};

use crate::parse::{Method, RouteArgs, ShortcutArgs};

pub(crate) fn shortcut(method: Method, args: ShortcutArgs, function: ItemFn) -> TokenStream {
    let ShortcutArgs { path, name } = args;
    let handler = function.sig.ident.clone();
    let call = match name {
        Some(name) => {
            let register = format_ident!("{}_with", method.router_method());
            quote!(router.#register(#path, #handler, ::pons_tako::OperationMeta::new(#name)))
        }
        None => {
            let register = format_ident!("{}", method.router_method());
            quote!(router.#register(#path, #handler))
        }
    };
    registration(handler, call, function)
}

pub(crate) fn route(args: RouteArgs, function: ItemFn) -> TokenStream {
    let RouteArgs { method, path, name } = args;
    let handler = function.sig.ident.clone();
    let http_method = Ident::new(method.http_name(), Span::call_site());
    let call = match name {
        Some(name) => quote! {
            router.route_with(
                ::pons_tako::Method::#http_method,
                #path,
                #handler,
                ::pons_tako::OperationMeta::new(#name),
            )
        },
        None => quote! {
            router.route(::pons_tako::Method::#http_method, #path, #handler)
        },
    };
    registration(handler, call, function)
}

fn registration(handler: Ident, call: TokenStream, function: ItemFn) -> TokenStream {
    let name = handler.to_string();
    let name = name
        .strip_prefix("r#")
        .unwrap_or(&name)
        .to_ascii_uppercase();
    let registrar = format_ident!("__PONS_TAKO_REG_{name}", span = handler.span());
    quote! {
        #[::pons_tako::linkme::distributed_slice(::pons_tako::PONS_TAKO_ROUTES)]
        #[linkme(crate = ::pons_tako::linkme)]
        static #registrar: fn(&mut ::pons_tako::Router) = |router| {
            #call;
        };

        #function
    }
}
