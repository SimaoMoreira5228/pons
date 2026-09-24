use std::any::type_name;
use std::convert::Infallible;

use axum::handler::Handler;
use axum::routing::MethodRouter;
use pons_ir::{Method, Name};

use crate::OperationCx;
use crate::handler::DescribeHandler;
use crate::metadata::OperationMeta;
use crate::operation::RouteDescription;

pub struct MethodRoute<S = ()> {
    pub(crate) inner: MethodRouter<S, Infallible>,
    pub(crate) descriptions: Vec<RouteDescription>,
}

impl<S> MethodRoute<S>
where
    S: Clone + Send + Sync + 'static,
{
    pub fn with_meta(mut self, meta: OperationMeta) -> Self {
        self.descriptions
            .last_mut()
            .expect("method route must contain a handler")
            .meta = Some(meta);
        self
    }

    pub fn head<H, T>(mut self, handler: H) -> Self
    where
        H: Handler<T, S> + DescribeHandler<T, S>,
        T: 'static,
    {
        self.inner = self.inner.head(handler);
        self.descriptions
            .push(describe_handler::<H, T, S>(Method::Head));
        self
    }

    pub fn post<H, T>(mut self, handler: H) -> Self
    where
        H: Handler<T, S> + DescribeHandler<T, S>,
        T: 'static,
    {
        self.inner = self.inner.post(handler);
        self.descriptions
            .push(describe_handler::<H, T, S>(Method::Post));
        self
    }

    pub fn put<H, T>(mut self, handler: H) -> Self
    where
        H: Handler<T, S> + DescribeHandler<T, S>,
        T: 'static,
    {
        self.inner = self.inner.put(handler);
        self.descriptions
            .push(describe_handler::<H, T, S>(Method::Put));
        self
    }

    pub fn patch<H, T>(mut self, handler: H) -> Self
    where
        H: Handler<T, S> + DescribeHandler<T, S>,
        T: 'static,
    {
        self.inner = self.inner.patch(handler);
        self.descriptions
            .push(describe_handler::<H, T, S>(Method::Patch));
        self
    }

    pub fn delete<H, T>(mut self, handler: H) -> Self
    where
        H: Handler<T, S> + DescribeHandler<T, S>,
        T: 'static,
    {
        self.inner = self.inner.delete(handler);
        self.descriptions
            .push(describe_handler::<H, T, S>(Method::Delete));
        self
    }

    pub fn options<H, T>(mut self, handler: H) -> Self
    where
        H: Handler<T, S> + DescribeHandler<T, S>,
        T: 'static,
    {
        self.inner = self.inner.options(handler);
        self.descriptions
            .push(describe_handler::<H, T, S>(Method::Options));
        self
    }
}

macro_rules! method_function {
    ($function:ident, $method:ident, $axum_function:ident) => {
        pub fn $function<H, T, S>(handler: H) -> MethodRoute<S>
        where
            H: Handler<T, S> + DescribeHandler<T, S>,
            T: 'static,
            S: Clone + Send + Sync + 'static,
        {
            MethodRoute {
                inner: axum::routing::$axum_function(handler),
                descriptions: vec![describe_handler::<H, T, S>(Method::$method)],
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
method_function!(options, Options, options);

fn describe_handler<H, T, S>(method: Method) -> RouteDescription
where
    H: DescribeHandler<T, S>,
{
    let mut cx = OperationCx::default();
    <H as DescribeHandler<T, S>>::describe(&mut cx);
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
