use std::collections::BTreeMap;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;

use pons_core::{ErrorSpec, TypeCx};
use pons_ir::{
    AuthScheme, Diagnostic, Document, ErrorResponse, Info, Operation, TypeDef, TypeRef,
    ValidationError,
};
use tako::route::Route;
use tako_rs_core::handler::Handler;

use crate::describe::OperationCx;
use crate::document;
use crate::handler::DescribeHandler;
use crate::metadata::OperationMeta;
use crate::operation::{self, RouteDescription};
use crate::operation_cx::push_diagnostic;

pub struct Router {
    inner: tako::router::Router,
    info: Info,
    pub(crate) prefix: String,
    operations: Vec<Operation>,
    types: BTreeMap<String, TypeDef>,
    carriers: Vec<TypeRef>,
    auth_schemes: BTreeMap<String, AuthScheme>,
    common_errors: Vec<ErrorResponse>,
    diagnostics: Vec<Diagnostic>,
}

impl Router {
    pub fn new(info: Info) -> Self {
        Self {
            inner: tako::router::Router::new(),
            info,
            prefix: String::new(),
            operations: Vec::new(),
            types: BTreeMap::new(),
            carriers: Vec::new(),
            auth_schemes: BTreeMap::new(),
            common_errors: Vec::new(),
            diagnostics: Vec::new(),
        }
    }

    pub fn auth_scheme(&mut self, id: impl Into<String>, scheme: AuthScheme) -> &mut Self {
        self.insert_auth_scheme(id.into(), scheme);
        self
    }

    pub fn common_error<E: ErrorSpec>(&mut self) -> &mut Self {
        let mut types = TypeCx::new();
        self.common_errors.extend(E::errors(&mut types));
        let (definitions, diagnostics) = types.into_parts();
        self.types.extend(definitions);
        self.diagnostics.extend(diagnostics);
        self
    }

    fn insert_auth_scheme(&mut self, id: String, scheme: AuthScheme) {
        if self
            .auth_schemes
            .get(&id)
            .is_some_and(|existing| existing != &scheme)
        {
            push_diagnostic(
                &mut self.diagnostics,
                "PONS-E102",
                format!("auth scheme {id} is declared twice with different definitions"),
            );
        }
        self.auth_schemes.insert(id, scheme);
    }

    pub fn route<H, T>(&mut self, method: http::Method, path: &str, handler: H) -> Arc<Route>
    where
        H: Handler<T> + Clone + DescribeHandler<T> + 'static,
    {
        self.register(method, path, handler, None)
    }

    pub fn route_with<H, T>(
        &mut self,
        method: http::Method,
        path: &str,
        handler: H,
        meta: OperationMeta,
    ) -> Arc<Route>
    where
        H: Handler<T> + Clone + DescribeHandler<T> + 'static,
    {
        self.register(method, path, handler, Some(meta))
    }

    pub fn route_described<H, T, D>(
        &mut self,
        method: http::Method,
        path: &str,
        handler: H,
        describe: D,
    ) -> Arc<Route>
    where
        H: Handler<T> + Clone + 'static,
        D: FnOnce(&mut OperationCx),
    {
        self.register_described(method, path, handler, None, describe)
    }

    pub fn route_described_with<H, T, D>(
        &mut self,
        method: http::Method,
        path: &str,
        handler: H,
        meta: OperationMeta,
        describe: D,
    ) -> Arc<Route>
    where
        H: Handler<T> + Clone + 'static,
        D: FnOnce(&mut OperationCx),
    {
        self.register_described(method, path, handler, Some(meta), describe)
    }

    fn register<H, T>(
        &mut self,
        method: http::Method,
        path: &str,
        handler: H,
        meta: Option<OperationMeta>,
    ) -> Arc<Route>
    where
        H: Handler<T> + Clone + DescribeHandler<T> + 'static,
    {
        self.register_described(method, path, handler, meta, |cx| H::describe(cx))
    }

    fn register_described<H, T, D>(
        &mut self,
        method: http::Method,
        path: &str,
        handler: H,
        meta: Option<OperationMeta>,
        describe: D,
    ) -> Arc<Route>
    where
        H: Handler<T> + Clone + 'static,
        D: FnOnce(&mut OperationCx),
    {
        let mut cx = OperationCx::default();
        describe(&mut cx);
        let full_path = join_prefix(&self.prefix, path);
        let route = self.inner.route(method.clone(), &full_path, handler);
        let description = RouteDescription {
            method: operation::ir_method(&method),
            id: operation::inferred_id(&method, route.path.as_ref()),
            cx,
            meta,
        };
        let built = operation::build(route.path.as_ref(), description);
        self.types.extend(built.types);
        self.carriers.extend(built.carriers);
        self.operations.push(built.operation);
        route
    }

    pub fn scope<F>(&mut self, prefix: &str, build: F) -> &mut Self
    where
        F: FnOnce(&mut Self),
    {
        let next = join_prefix(&self.prefix, prefix);
        let saved = std::mem::replace(&mut self.prefix, next);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| build(self)));
        self.prefix = saved;
        if let Err(payload) = result {
            std::panic::resume_unwind(payload);
        }
        self
    }

    pub fn nest(&mut self, prefix: &str, other: Router) -> &mut Self {
        let prefix_segments = match operation::literal_prefix(prefix) {
            Ok(segments) => segments,
            Err(message) => {
                push_diagnostic(&mut self.diagnostics, "PONS-E102", message);
                return self;
            }
        };
        let Router {
            inner,
            mut operations,
            types,
            carriers,
            auth_schemes,
            common_errors,
            diagnostics,
            ..
        } = other;
        self.inner.nest(prefix, inner);
        operations.iter_mut().for_each(|operation| {
            operation.path = operation::nested_path(&prefix_segments, &operation.path);
        });
        self.types.extend(types);
        self.carriers.extend(carriers);
        for (id, scheme) in auth_schemes {
            self.insert_auth_scheme(id, scheme);
        }
        self.common_errors.extend(common_errors);
        self.diagnostics.extend(diagnostics);
        self.operations.extend(operations);
        self
    }

    pub fn merge(&mut self, other: Router) -> &mut Self {
        self.nest("", other)
    }

    pub fn document(&self) -> Document {
        document::assemble(
            self.info.clone(),
            self.types.clone(),
            self.carriers.clone(),
            self.common_errors.clone(),
            self.auth_schemes.clone(),
            self.operations.clone(),
            self.diagnostics.clone(),
        )
    }

    pub fn into_document(self) -> Result<Document, Vec<ValidationError>> {
        document::checked(self.into_document_value())
    }

    pub fn into_router(mut self) -> tako::router::Router {
        let inner = std::mem::replace(&mut self.inner, tako::router::Router::new());
        let document = Arc::new(self.into_document().unwrap_or_else(|errors| {
            panic!(
                "the described document is invalid:\n{}",
                errors
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("\n")
            )
        }));
        inner.middleware(move |mut request, next| {
            request.extensions_mut().insert(Arc::clone(&document));
            next.run(request)
        });
        inner
    }

    fn into_document_value(self) -> Document {
        document::assemble(
            self.info,
            self.types,
            self.carriers,
            self.common_errors,
            self.auth_schemes,
            self.operations,
            self.diagnostics,
        )
    }
}

impl Deref for Router {
    type Target = tako::router::Router;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl DerefMut for Router {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

macro_rules! route_methods {
    ($method:ident, $method_with:ident, $method_described:ident, $method_described_with:ident, $http_method:expr) => {
        pub fn $method<H, T>(&mut self, path: &str, handler: H) -> Arc<Route>
        where
            H: Handler<T> + Clone + DescribeHandler<T> + 'static,
        {
            self.register($http_method, path, handler, None)
        }

        pub fn $method_with<H, T>(
            &mut self,
            path: &str,
            handler: H,
            meta: OperationMeta,
        ) -> Arc<Route>
        where
            H: Handler<T> + Clone + DescribeHandler<T> + 'static,
        {
            self.register($http_method, path, handler, Some(meta))
        }

        pub fn $method_described<H, T, D>(
            &mut self,
            path: &str,
            handler: H,
            describe: D,
        ) -> Arc<Route>
        where
            H: Handler<T> + Clone + 'static,
            D: FnOnce(&mut OperationCx),
        {
            self.register_described($http_method, path, handler, None, describe)
        }

        pub fn $method_described_with<H, T, D>(
            &mut self,
            path: &str,
            handler: H,
            meta: OperationMeta,
            describe: D,
        ) -> Arc<Route>
        where
            H: Handler<T> + Clone + 'static,
            D: FnOnce(&mut OperationCx),
        {
            self.register_described($http_method, path, handler, Some(meta), describe)
        }
    };
}

impl Router {
    route_methods!(
        get,
        get_with,
        get_described,
        get_described_with,
        http::Method::GET
    );
    route_methods!(
        post,
        post_with,
        post_described,
        post_described_with,
        http::Method::POST
    );
    route_methods!(
        put,
        put_with,
        put_described,
        put_described_with,
        http::Method::PUT
    );
    route_methods!(
        delete,
        delete_with,
        delete_described,
        delete_described_with,
        http::Method::DELETE
    );
    route_methods!(
        patch,
        patch_with,
        patch_described,
        patch_described_with,
        http::Method::PATCH
    );
    route_methods!(
        head,
        head_with,
        head_described,
        head_described_with,
        http::Method::HEAD
    );
    route_methods!(
        options,
        options_with,
        options_described,
        options_described_with,
        http::Method::OPTIONS
    );
}

pub(crate) fn join_prefix(prefix: &str, path: &str) -> String {
    if prefix.is_empty() {
        return path.to_owned();
    }
    let prefix = prefix.trim_end_matches('/');
    if path.is_empty() || path == "/" {
        if prefix.is_empty() {
            "/".to_owned()
        } else {
            prefix.to_owned()
        }
    } else if path.starts_with('/') {
        format!("{prefix}{path}")
    } else {
        format!("{prefix}/{path}")
    }
}
