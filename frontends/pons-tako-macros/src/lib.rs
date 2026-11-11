mod expand;
mod parse;

use proc_macro::TokenStream;
use syn::{ItemFn, parse_macro_input};

use crate::parse::{Method, RouteArgs, ShortcutArgs};

/// Registers the handler for `Router::mount_all` under an explicit method.
///
/// ```ignore
/// #[pons_tako::route(GET, "/users/{id}", name = "users.get")]
/// async fn get_user() -> &'static str { "Ada" }
///
/// #[pons_tako::route(GET, "/users/{id}", response = Json<User>)]
/// async fn get_opaque_user() -> impl Responder { Json(user) }
/// ```
#[proc_macro_attribute]
pub fn route(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as RouteArgs);
    let function = parse_macro_input!(item as ItemFn);
    expand::route(args, function)
        .unwrap_or_else(|error| error.to_compile_error())
        .into()
}

/// Registers a `GET` handler for `Router::mount_all`.
///
/// ```ignore
/// #[pons_tako::get("/users/{id}")]
/// async fn get_user() -> &'static str { "Ada" }
///
/// #[pons_tako::get("/users/{id}", response = Json<User>)]
/// async fn get_opaque_user() -> impl Responder { Json(user) }
/// ```
#[proc_macro_attribute]
pub fn get(attr: TokenStream, item: TokenStream) -> TokenStream {
    shortcut(Method::Get, attr, item)
}

/// Registers a `POST` handler for `Router::mount_all`.
#[proc_macro_attribute]
pub fn post(attr: TokenStream, item: TokenStream) -> TokenStream {
    shortcut(Method::Post, attr, item)
}

/// Registers a `PUT` handler for `Router::mount_all`.
#[proc_macro_attribute]
pub fn put(attr: TokenStream, item: TokenStream) -> TokenStream {
    shortcut(Method::Put, attr, item)
}

/// Registers a `DELETE` handler for `Router::mount_all`.
#[proc_macro_attribute]
pub fn delete(attr: TokenStream, item: TokenStream) -> TokenStream {
    shortcut(Method::Delete, attr, item)
}

/// Registers a `PATCH` handler for `Router::mount_all`.
#[proc_macro_attribute]
pub fn patch(attr: TokenStream, item: TokenStream) -> TokenStream {
    shortcut(Method::Patch, attr, item)
}

/// Registers a `HEAD` handler for `Router::mount_all`.
#[proc_macro_attribute]
pub fn head(attr: TokenStream, item: TokenStream) -> TokenStream {
    shortcut(Method::Head, attr, item)
}

/// Registers an `OPTIONS` handler for `Router::mount_all`.
#[proc_macro_attribute]
pub fn options(attr: TokenStream, item: TokenStream) -> TokenStream {
    shortcut(Method::Options, attr, item)
}

fn shortcut(method: Method, attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr as ShortcutArgs);
    let function = parse_macro_input!(item as ItemFn);
    expand::shortcut(method, args, function)
        .unwrap_or_else(|error| error.to_compile_error())
        .into()
}
