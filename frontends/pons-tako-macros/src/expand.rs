use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};
use syn::{FnArg, Ident, ItemFn, ReturnType, Type};

use crate::parse::{Method, RouteArgs, ShortcutArgs};

pub(crate) fn shortcut(
    method: Method,
    args: ShortcutArgs,
    function: ItemFn,
) -> syn::Result<TokenStream> {
    let ShortcutArgs {
        path,
        name,
        response,
    } = args;
    let handler = function.sig.ident.clone();
    let call = match response {
        Some(response) => {
            let describe = declaration(&function, &response)?;
            match name {
                Some(name) => {
                    let register = format_ident!("{}_described_with", method.router_method());
                    quote!(router.#register(#path, #handler, ::pons_tako::OperationMeta::new(#name), #describe))
                }
                None => {
                    let register = format_ident!("{}_described", method.router_method());
                    quote!(router.#register(#path, #handler, #describe))
                }
            }
        }
        None => {
            require_concrete_return(&function)?;
            match name {
                Some(name) => {
                    let register = format_ident!("{}_with", method.router_method());
                    quote!(router.#register(#path, #handler, ::pons_tako::OperationMeta::new(#name)))
                }
                None => {
                    let register = format_ident!("{}", method.router_method());
                    quote!(router.#register(#path, #handler))
                }
            }
        }
    };
    Ok(registration(handler, call, function))
}

pub(crate) fn route(args: RouteArgs, function: ItemFn) -> syn::Result<TokenStream> {
    let RouteArgs {
        method,
        path,
        name,
        response,
    } = args;
    let handler = function.sig.ident.clone();
    let http_method = Ident::new(method.http_name(), Span::call_site());
    let call = match response {
        Some(response) => {
            let describe = declaration(&function, &response)?;
            match name {
                Some(name) => quote! {
                    router.route_described_with(
                        ::pons_tako::Method::#http_method,
                        #path,
                        #handler,
                        ::pons_tako::OperationMeta::new(#name),
                        #describe,
                    )
                },
                None => quote! {
                    router.route_described(
                        ::pons_tako::Method::#http_method,
                        #path,
                        #handler,
                        #describe,
                    )
                },
            }
        }
        None => {
            require_concrete_return(&function)?;
            match name {
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
            }
        }
    };
    Ok(registration(handler, call, function))
}

fn declaration(function: &ItemFn, response: &Type) -> syn::Result<TokenStream> {
    let arguments = function
        .sig
        .inputs
        .iter()
        .map(|argument| match argument {
            FnArg::Typed(argument) => {
                let ty = &argument.ty;
                Ok(quote!(<#ty as ::pons_tako::DescribeRequest>::describe(cx);))
            }
            FnArg::Receiver(receiver) => Err(syn::Error::new_spanned(
                receiver,
                "a Pons route handler cannot take `self`",
            )),
        })
        .collect::<syn::Result<Vec<_>>>()?;
    Ok(quote! {
        |cx: &mut ::pons_tako::OperationCx| {
            #(#arguments)*
            <#response as ::pons_tako::DescribeResponse>::describe(cx);
        }
    })
}

fn require_concrete_return(function: &ItemFn) -> syn::Result<()> {
    let ReturnType::Type(_, ty) = &function.sig.output else {
        return Ok(());
    };
    if matches!(ty.as_ref(), Type::ImplTrait(_)) {
        return Err(syn::Error::new_spanned(
            ty,
            "a handler returning `impl Responder` needs `response = Type` so Pons can describe its response",
        ));
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    fn function(input: TokenStream) -> ItemFn {
        syn::parse2(input).unwrap()
    }

    fn args(input: TokenStream) -> ShortcutArgs {
        syn::parse2(input).unwrap()
    }

    #[test]
    fn an_opaque_return_needs_a_declared_response() {
        let function = function(quote! {
            async fn handler() -> impl ::tako::responder::Responder {
                "Ada"
            }
        });
        let Err(error) = shortcut(Method::Get, args(quote!("/users")), function) else {
            panic!("an opaque return without a response was accepted");
        };
        assert_eq!(
            error.to_string(),
            "a handler returning `impl Responder` needs `response = Type` so Pons can describe its response"
        );
    }

    #[test]
    fn a_declared_response_describes_every_argument() {
        let function = function(quote! {
            async fn handler(
                Query(query): Query<Listing>,
                Path(user): Path<UserId>,
                Json(input): Json<Update>,
            ) -> impl ::tako::responder::Responder {
                Json(input)
            }
        });
        let expanded = shortcut(
            Method::Post,
            args(quote!("/users/{id}", response = Json<User>, name = "users.update")),
            function,
        )
        .unwrap()
        .to_string();
        assert!(
            expanded.contains(
                "< Query < Listing > as :: pons_tako :: DescribeRequest > :: describe (cx)"
            )
        );
        assert!(
            expanded.contains(
                "< Path < UserId > as :: pons_tako :: DescribeRequest > :: describe (cx)"
            )
        );
        assert!(
            expanded.contains(
                "< Json < Update > as :: pons_tako :: DescribeRequest > :: describe (cx)"
            )
        );
        assert!(
            expanded
                .contains("< Json < User > as :: pons_tako :: DescribeResponse > :: describe (cx)")
        );
        assert!(expanded.contains("post_described_with"));
    }

    #[test]
    fn a_concrete_return_without_a_response_keeps_the_typed_registration() {
        let function = function(quote! {
            async fn handler() -> Json<User> {
                Json(User::default())
            }
        });
        let expanded = shortcut(Method::Get, args(quote!("/users")), function)
            .unwrap()
            .to_string();
        assert!(expanded.contains("router . get"));
        assert!(!expanded.contains("_described"));
    }
}
