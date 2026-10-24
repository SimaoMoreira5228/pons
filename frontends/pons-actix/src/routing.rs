use std::any::type_name;

use actix_web::Handler;
use actix_web::Route;
use pons_ir::{Method, Name};

use crate::OperationCx;
use crate::handler::DescribeHandler;
use crate::metadata::OperationMeta;
use crate::operation::RouteDescription;

pub struct MethodRoute {
    pub(crate) routes: Vec<Route>,
    pub(crate) descriptions: Vec<RouteDescription>,
}

impl MethodRoute {
    pub fn with_meta(mut self, meta: OperationMeta) -> Self {
        self.descriptions
            .last_mut()
            .expect("method route must contain a handler")
            .meta = Some(meta);
        self
    }

    pub fn head<H, T>(mut self, handler: H) -> Self
    where
        H: Handler<T> + DescribeHandler<T>,
        T: actix_web::FromRequest + 'static,
        H::Output: actix_web::Responder + 'static,
    {
        self.routes.push(actix_web::web::head().to(handler));
        self.descriptions
            .push(describe_handler::<H, T>(Method::Head));
        self
    }

    pub fn get<H, T>(mut self, handler: H) -> Self
    where
        H: Handler<T> + DescribeHandler<T>,
        T: actix_web::FromRequest + 'static,
        H::Output: actix_web::Responder + 'static,
    {
        self.routes.push(actix_web::web::get().to(handler));
        self.descriptions
            .push(describe_handler::<H, T>(Method::Get));
        self
    }

    pub fn post<H, T>(mut self, handler: H) -> Self
    where
        H: Handler<T> + DescribeHandler<T>,
        T: actix_web::FromRequest + 'static,
        H::Output: actix_web::Responder + 'static,
    {
        self.routes.push(actix_web::web::post().to(handler));
        self.descriptions
            .push(describe_handler::<H, T>(Method::Post));
        self
    }

    pub fn put<H, T>(mut self, handler: H) -> Self
    where
        H: Handler<T> + DescribeHandler<T>,
        T: actix_web::FromRequest + 'static,
        H::Output: actix_web::Responder + 'static,
    {
        self.routes.push(actix_web::web::put().to(handler));
        self.descriptions
            .push(describe_handler::<H, T>(Method::Put));
        self
    }

    pub fn patch<H, T>(mut self, handler: H) -> Self
    where
        H: Handler<T> + DescribeHandler<T>,
        T: actix_web::FromRequest + 'static,
        H::Output: actix_web::Responder + 'static,
    {
        self.routes.push(actix_web::web::patch().to(handler));
        self.descriptions
            .push(describe_handler::<H, T>(Method::Patch));
        self
    }

    pub fn delete<H, T>(mut self, handler: H) -> Self
    where
        H: Handler<T> + DescribeHandler<T>,
        T: actix_web::FromRequest + 'static,
        H::Output: actix_web::Responder + 'static,
    {
        self.routes.push(actix_web::web::delete().to(handler));
        self.descriptions
            .push(describe_handler::<H, T>(Method::Delete));
        self
    }

    pub fn options<H, T>(mut self, handler: H) -> Self
    where
        H: Handler<T> + DescribeHandler<T>,
        T: actix_web::FromRequest + 'static,
        H::Output: actix_web::Responder + 'static,
    {
        self.routes.push(
            actix_web::web::route()
                .method(actix_web::http::Method::OPTIONS)
                .to(handler),
        );
        self.descriptions
            .push(describe_handler::<H, T>(Method::Options));
        self
    }
}

macro_rules! method_function {
    ($function:ident, $method:ident, $actix_function:ident) => {
        pub fn $function<H, T>(handler: H) -> MethodRoute
        where
            H: Handler<T> + DescribeHandler<T>,
            T: actix_web::FromRequest + 'static,
            H::Output: actix_web::Responder + 'static,
        {
            MethodRoute {
                routes: vec![actix_web::web::$actix_function().to(handler)],
                descriptions: vec![describe_handler::<H, T>(Method::$method)],
            }
        }
    };
}

method_function!(get, Get, get);
method_function!(post, Post, post);
method_function!(put, Put, put);
method_function!(patch, Patch, patch);
method_function!(delete, Delete, delete);
method_function!(head, Head, head);

pub fn options<H, T>(handler: H) -> MethodRoute
where
    H: Handler<T> + DescribeHandler<T>,
    T: actix_web::FromRequest + 'static,
    H::Output: actix_web::Responder + 'static,
{
    MethodRoute {
        routes: vec![
            actix_web::web::route()
                .method(actix_web::http::Method::OPTIONS)
                .to(handler),
        ],
        descriptions: vec![describe_handler::<H, T>(Method::Options)],
    }
}

macro_rules! method_with_function {
    ($function:ident, $base:ident) => {
        pub fn $function<H, T>(handler: H, meta: OperationMeta) -> MethodRoute
        where
            H: Handler<T> + DescribeHandler<T>,
            T: actix_web::FromRequest + 'static,
            H::Output: actix_web::Responder + 'static,
        {
            $base::<H, T>(handler).with_meta(meta)
        }
    };
}

method_with_function!(get_with, get);
method_with_function!(post_with, post);
method_with_function!(put_with, put);
method_with_function!(patch_with, patch);
method_with_function!(delete_with, delete);
method_with_function!(head_with, head);
method_with_function!(options_with, options);

fn describe_handler<H, T>(method: Method) -> RouteDescription
where
    H: DescribeHandler<T>,
{
    let mut cx = OperationCx::default();
    <H as DescribeHandler<T>>::describe(&mut cx);
    RouteDescription {
        method,
        id: handler_id::<H>(),
        cx,
        meta: None,
    }
}

fn handler_id<H>() -> Vec<Name> {
    let mut parts: Vec<_> = type_name::<H>().split("::").map(str::to_owned).collect();
    if parts.len() > 1 {
        parts.remove(0);
    }
    if parts.is_empty() {
        parts.push("operation".into());
    }
    parts.iter().map(|part| name_words(part)).collect()
}

fn name_words(source: &str) -> Name {
    let mut words = Vec::new();
    let mut current = String::new();
    let chars: Vec<char> = source.trim_start_matches("r#").chars().collect();
    for (index, character) in chars.iter().copied().enumerate() {
        if character == '_' {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
        } else {
            if character.is_uppercase() && !current.is_empty() {
                let previous = chars
                    .get(index.wrapping_sub(1))
                    .copied()
                    .unwrap_or_default();
                let next = chars.get(index + 1).copied().unwrap_or_default();
                if previous.is_lowercase() || next.is_lowercase() && previous.is_uppercase() {
                    words.push(std::mem::take(&mut current));
                }
            }
            current.extend(character.to_lowercase());
        }
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}
