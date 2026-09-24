mod auth;
mod describe;
mod handler;
mod layer;
mod metadata;
mod ndjson;
mod operation;
mod raw_bytes;
mod response;
pub mod routing;
mod sse;

use std::collections::BTreeMap;

use pons_core::{ErrorSpec, TypeCx};

pub use auth::{Auth, AuthSpec, SessionAuth};
pub use axum::{body, extract, http};
pub use describe::{
    DescribeRequest, DescribeResponse, Header, HeaderRejection, HeaderSpec, NoContent, OperationCx,
    OptionalHeader, StatusResponse, WithHeader, WithOptionalHeader, WithStatus,
};
pub use handler::DescribeHandler;
pub use layer::DocumentedLayer;
pub use metadata::OperationMeta;
pub use ndjson::Ndjson;
use operation::build;
pub use pons_ir::Info;
use pons_ir::{AuthScheme, Diagnostic, Document, ErrorResponse, TypeDef, ValidationError};
pub use raw_bytes::RawBytes;
pub use response::{
    BytesResponse, JsonResponse, ProblemJsonResponse, ResponseSpec, TextResponse, WithResponse,
};
pub use routing::MethodRoute;
pub use sse::{Sse, SseEvent};

pub struct Router<S = ()> {
    inner: axum::Router<S>,
    info: Info,
    types: BTreeMap<String, TypeDef>,
    auth_schemes: BTreeMap<String, AuthScheme>,
    common_errors: Vec<ErrorResponse>,
    operations: Vec<pons_ir::Operation>,
    diagnostics: Vec<Diagnostic>,
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
            common_errors: Vec::new(),
            operations: Vec::new(),
            diagnostics: Vec::new(),
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

    pub fn common_error<E: ErrorSpec>(mut self) -> Self {
        let mut types = TypeCx::new();
        self.common_errors.extend(E::errors(&mut types));
        let (definitions, diagnostics) = types.into_parts();
        self.types.extend(definitions);
        self.diagnostics.extend(diagnostics);
        self
    }

    pub fn with_state<S2>(self, state: S) -> Router<S2> {
        Router {
            inner: self.inner.with_state(state),
            info: self.info,
            types: self.types,
            auth_schemes: self.auth_schemes,
            common_errors: self.common_errors,
            operations: self.operations,
            diagnostics: self.diagnostics,
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
            common_errors: self.common_errors.clone(),
            operations: self.operations.clone(),
            ext: BTreeMap::new(),
            diagnostics: self.diagnostics.clone(),
        }
    }

    pub fn try_lower_document(
        self,
        lower: impl FnOnce(Document) -> Result<Document, Vec<ValidationError>>,
    ) -> Result<Document, Vec<ValidationError>> {
        let mut document = lower(self.into_document_value())?;
        document.canonicalize();
        document.validate()?;
        Ok(document)
    }

    pub fn into_document(self) -> Result<Document, Vec<ValidationError>> {
        let mut document = self.into_document_value();
        document.canonicalize();
        document.validate()?;
        Ok(document)
    }

    fn into_document_value(self) -> Document {
        Document {
            ir_version: pons_ir::IR_VERSION.to_owned(),
            info: self.info,
            types: self.types,
            auth_schemes: self.auth_schemes,
            common_errors: self.common_errors,
            operations: self.operations,
            ext: BTreeMap::new(),
            diagnostics: self.diagnostics,
        }
    }
}

#[macro_export]
macro_rules! export_main {
    ($builder:path) => {
        $crate::export_main!($builder, |document| Ok(document));
    };
    ($builder:path, $lower:expr) => {
        fn main() -> ::std::result::Result<(), ::std::boxed::Box<dyn ::std::error::Error>> {
            let document = $builder().try_lower_document($lower).map_err(|errors| {
                ::std::io::Error::other(
                    errors
                        .into_iter()
                        .map(|error| error.to_string())
                        .collect::<::std::vec::Vec<_>>()
                        .join("\n"),
                )
            })?;
            let mut output = document.to_canonical_json()?;
            output.push('\n');
            let mut stdout = ::std::io::stdout();
            ::std::io::Write::write_all(&mut stdout, output.as_bytes())?;
            Ok(())
        }
    };
}
