mod describe;
mod handler;
mod operation;
pub mod routing;
mod sse;

use std::collections::BTreeMap;

pub use axum::{body, extract, http};
pub use describe::{
    DescribeRequest, DescribeResponse, Header, HeaderRejection, HeaderSpec, OperationCx,
    OptionalHeader, StatusResponse, WithHeader, WithOptionalHeader, WithStatus,
};
pub use handler::DescribeHandler;
use operation::build;
use pons_ir::{AuthScheme, Document, Info, TypeDef, ValidationError};
pub use routing::MethodRoute;
pub use sse::{Sse, SseEvent};

pub struct Router<S = ()> {
    inner: axum::Router<S>,
    info: Info,
    types: BTreeMap<String, TypeDef>,
    auth_schemes: BTreeMap<String, AuthScheme>,
    operations: Vec<pons_ir::Operation>,
}

impl<S> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    pub fn new(info: Info) -> Self {
        Self {
            inner: axum::Router::new(),
            info,
            types: BTreeMap::new(),
            auth_schemes: BTreeMap::new(),
            operations: Vec::new(),
        }
    }

    pub fn route(mut self, path: &str, route: MethodRoute<S>) -> Self {
        self.inner = self.inner.route(path, route.inner);
        for description in route.descriptions {
            let built = build(path, description);
            self.types.extend(built.types);
            self.operations.push(built.operation);
        }
        self
    }

    pub fn auth_scheme(mut self, id: impl Into<String>, scheme: AuthScheme) -> Self {
        self.auth_schemes.insert(id.into(), scheme);
        self
    }

    pub fn with_state<S2>(self, state: S) -> Router<S2> {
        Router {
            inner: self.inner.with_state(state),
            info: self.info,
            types: self.types,
            auth_schemes: self.auth_schemes,
            operations: self.operations,
        }
    }

    pub fn into_axum(self) -> axum::Router<S> {
        self.inner
    }

    pub fn document(&self) -> Document {
        Document {
            ir_version: pons_ir::IR_VERSION.to_owned(),
            info: self.info.clone(),
            types: self.types.clone(),
            auth_schemes: self.auth_schemes.clone(),
            common_errors: Vec::new(),
            operations: self.operations.clone(),
            ext: BTreeMap::new(),
            diagnostics: Vec::new(),
        }
    }

    pub fn into_document(self) -> Result<Document, Vec<ValidationError>> {
        let mut document = Document {
            ir_version: pons_ir::IR_VERSION.to_owned(),
            info: self.info,
            types: self.types,
            auth_schemes: self.auth_schemes,
            common_errors: Vec::new(),
            operations: self.operations,
            ext: BTreeMap::new(),
            diagnostics: Vec::new(),
        };
        document.canonicalize();
        document.validate()?;
        Ok(document)
    }
}
