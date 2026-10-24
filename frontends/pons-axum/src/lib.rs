mod auth;
mod describe;
mod document;
mod error;
pub mod extract;
mod handler;
mod layer;
mod metadata;
mod ndjson;
mod operation;
mod operation_cx;
mod raw_bytes;
mod request;
mod response;
pub mod routing;
mod sse;
mod validate;
mod websocket;

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::Extension;
use pons_core::{ErrorSpec, TypeCx};

pub use auth::{Auth, AuthSpec, SessionAuth};
pub use axum::{body, http};
pub use describe::{
    DescribeRequest, DescribeResponse, Header, HeaderRejection, HeaderSpec, NoContent, OperationCx,
    OptionalHeader, StatusResponse, WithHeader, WithOptionalHeader, WithStatus,
};
pub use error::{
    Violation, ViolationReport, WithErrorHeader, WithOptionalErrorHeader, error_header,
};
pub use handler::DescribeHandler;
pub use layer::DocumentedLayer;
pub use metadata::OperationMeta;
pub use ndjson::Ndjson;
use operation::build;
pub use pons_ir::Info;
use pons_ir::{
    AuthScheme, ContentKind, Diagnostic, Document, ErrorResponse, Method, TypeDef, TypeRef,
    ValidationError,
};
pub use raw_bytes::RawBytes;
pub use request::{Form, Json, Path, Query};
pub use response::{
    BytesResponse, JsonResponse, ProblemJsonResponse, ResponseSpec, TextResponse, WithResponse,
    WithUnknownStatus,
};
pub use routing::MethodRoute;
pub use sse::{Sse, SseEvent};
pub use websocket::{WebSocketResponse, WebSocketUpgrade};

pub struct Router<S = ()> {
    inner: axum::Router<S>,
    info: Info,
    types: BTreeMap<String, TypeDef>,
    carriers: Vec<TypeRef>,
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
            carriers: Vec::new(),
            auth_schemes: BTreeMap::new(),
            common_errors: Vec::new(),
            operations: Vec::new(),
            diagnostics: Vec::new(),
        }
    }

    pub fn route(mut self, path: &str, route: MethodRoute<S>) -> Self {
        self.inner = self.inner.route(path, route.inner);
        for description in route.descriptions {
            let mut built = build(path, description);
            resolve_request_body(&mut built.operation);
            self.types.extend(built.types);
            self.carriers.extend(built.carriers);
            self.operations.push(built.operation);
        }
        self
    }

    pub fn merge(mut self, other: Router<S>) -> Self {
        let Router {
            inner,
            types,
            carriers,
            auth_schemes,
            common_errors,
            operations,
            diagnostics,
            ..
        } = other;
        self.inner = self.inner.merge(inner);
        self.absorb(
            types,
            carriers,
            auth_schemes,
            common_errors,
            operations,
            diagnostics,
        );
        self
    }

    pub fn nest(mut self, prefix: &str, other: Router<S>) -> Self {
        let prefix_segments = match operation::literal_prefix(prefix) {
            Ok(segments) => segments,
            Err(message) => {
                operation_cx::push_diagnostic(&mut self.diagnostics, "PONS-E102", message);
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
        for operation in &mut operations {
            operation.path = operation::nested_path(&prefix_segments, &operation.path);
        }
        self.inner = self.inner.nest(prefix, inner);
        self.absorb(
            types,
            carriers,
            auth_schemes,
            common_errors,
            operations,
            diagnostics,
        );
        self
    }

    pub fn auth_scheme(mut self, id: impl Into<String>, scheme: AuthScheme) -> Self {
        let id = id.into();
        if self
            .auth_schemes
            .get(&id)
            .is_some_and(|existing| existing != &scheme)
        {
            operation_cx::push_diagnostic(
                &mut self.diagnostics,
                "PONS-E102",
                format!("auth scheme {id} is declared twice with different definitions"),
            );
        }
        self.auth_schemes.insert(id, scheme);
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
            carriers: self.carriers,
            auth_schemes: self.auth_schemes,
            common_errors: self.common_errors,
            operations: self.operations,
            diagnostics: self.diagnostics,
        }
    }

    pub fn into_axum(mut self) -> Result<axum::Router<S>, Vec<ValidationError>> {
        let inner = std::mem::replace(&mut self.inner, axum::Router::new());
        let document = document::checked(self.into_document_value())?;
        Ok(inner.layer(Extension(Arc::new(document))))
    }

    pub fn document(&self) -> Document {
        let mut operations = self.operations.clone();
        self.settle(&mut operations);
        let mut document = Document {
            ir_version: pons_ir::IR_VERSION.to_owned(),
            info: self.info.clone(),
            types: self.types.clone(),
            auth_schemes: self.auth_schemes.clone(),
            common_errors: self.common_errors.clone(),
            operations,
            ext: BTreeMap::new(),
            diagnostics: self.diagnostics.clone(),
        };
        document::prune_unreferenced_types(&mut document, &self.carriers);
        document
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
        document::checked(self.into_document_value())
    }

    fn into_document_value(mut self) -> Document {
        let mut operations = std::mem::take(&mut self.operations);
        self.settle(&mut operations);
        let mut document = Document {
            ir_version: pons_ir::IR_VERSION.to_owned(),
            info: self.info,
            types: self.types,
            auth_schemes: self.auth_schemes,
            common_errors: self.common_errors,
            operations,
            ext: BTreeMap::new(),
            diagnostics: self.diagnostics,
        };
        document::prune_unreferenced_types(&mut document, &self.carriers);
        document
    }

    fn absorb(
        &mut self,
        types: BTreeMap<String, TypeDef>,
        carriers: Vec<TypeRef>,
        auth_schemes: BTreeMap<String, AuthScheme>,
        common_errors: Vec<ErrorResponse>,
        operations: Vec<pons_ir::Operation>,
        diagnostics: Vec<Diagnostic>,
    ) {
        for (id, definition) in types {
            if self
                .types
                .get(&id)
                .is_some_and(|existing| existing != &definition)
            {
                operation_cx::push_diagnostic(
                    &mut self.diagnostics,
                    "PONS-E102",
                    format!("type {id} is described twice with different definitions"),
                );
            }
            self.types.insert(id, definition);
        }
        self.carriers.extend(carriers);
        for (id, scheme) in auth_schemes {
            if self
                .auth_schemes
                .get(&id)
                .is_some_and(|existing| existing != &scheme)
            {
                operation_cx::push_diagnostic(
                    &mut self.diagnostics,
                    "PONS-E102",
                    format!("auth scheme {id} is declared twice with different definitions"),
                );
            }
            self.auth_schemes.insert(id, scheme);
        }
        self.common_errors.extend(common_errors);
        self.operations.extend(operations);
        self.diagnostics.extend(diagnostics);
    }

    fn settle(&self, operations: &mut [pons_ir::Operation]) {
        document::settle(&self.common_errors, operations);
    }
}

fn resolve_request_body(operation: &mut pons_ir::Operation) {
    let Some(body) = &operation.body else {
        return;
    };
    match (operation.method, body.content) {
        (Method::Get | Method::Head, ContentKind::Form) if matches!(body.ty, TypeRef::Named(_)) => {
            let form = operation
                .body
                .take()
                .expect("the operation publishes a body");
            if operation.query.replace(form.ty).is_some() {
                operation_cx::push_diagnostic(
                    &mut operation.diagnostics,
                    "PONS-W101",
                    "a form on a GET or a HEAD is read from the query, which the operation already publishes a type for",
                );
            }
        }
        (Method::Get | Method::Head, ContentKind::Form) => operation_cx::push_diagnostic(
            &mut operation.diagnostics,
            "PONS-E102",
            "a form on a GET or a HEAD is read from the query, which this contract publishes as typed fields, so the form type must be a struct; use a struct type, or read the query with Query",
        ),
        (Method::Get | Method::Head, ContentKind::Multipart) => operation_cx::push_diagnostic(
            &mut operation.diagnostics,
            "PONS-E102",
            "a multipart body is read from a request body, and the client of a GET or HEAD operation sends none, so axum answers 400 for the boundary it never receives; the operation cannot be published",
        ),
        _ if framework_reads_the_body(body) => declare_framework_rejections(operation),
        _ => {}
    }
}

fn framework_reads_the_body(body: &pons_ir::Body) -> bool {
    match body.content {
        ContentKind::Multipart | ContentKind::Text | ContentKind::Bytes => true,
        ContentKind::Form => !matches!(body.ty, TypeRef::Named(_)),
        ContentKind::Json | ContentKind::Stream | ContentKind::ProblemJson | ContentKind::Empty => {
            false
        }
    }
}

fn declare_framework_rejections(operation: &mut pons_ir::Operation) {
    let undeclared: Vec<String> = [400, 413]
        .into_iter()
        .filter(|status| !document::declares_status(operation, *status))
        .map(|status| status.to_string())
        .collect();
    if undeclared.is_empty() {
        return;
    }
    operation_cx::push_diagnostic(
        &mut operation.diagnostics,
        "PONS-W101",
        format!(
            "the framework reads this request body itself, so it answers a body it cannot read as 400 and a body over the limit as 413 before the handler runs; the operation declares none of {}",
            undeclared.join(" or ")
        ),
    );
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

#[cfg(test)]
mod tests {
    use super::*;
    use pons_ir::{AuthRequirement, ResponseHeader, TypeKind};

    #[test]
    fn every_slot_an_operation_names_a_type_in_survives_pruning() {
        for errors_inherit in [true, false] {
            let mut document = slots_document(errors_inherit);
            document.validate().unwrap_or_else(|errors| {
                panic!(
                    "the fixture must be a document that validates: {}",
                    errors
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            });
            document::prune_unreferenced_types(&mut document, &[]);
            for slot in SLOTS {
                assert!(
                    document.types.contains_key(*slot),
                    "errors_inherit={errors_inherit} pruned the {slot} slot"
                );
            }
            assert!(
                document.types.contains_key("common"),
                "errors_inherit={errors_inherit} pruned the common error"
            );
            assert!(
                !document.types.contains_key("orphan"),
                "errors_inherit={errors_inherit} kept a definition no slot names"
            );
        }
    }

    const SLOTS: &[&str] = &[
        "path",
        "query",
        "body",
        "request-header",
        "response-body",
        "response-header",
        "error-body",
        "stream",
    ];

    fn slots_document(errors_inherit: bool) -> Document {
        let mut types: BTreeMap<String, TypeDef> = SLOTS
            .iter()
            .map(|slot| ((*slot).to_owned(), definition(slot)))
            .collect();

        types.insert("query".to_owned(), newtype("query", slot("query-page")));
        types.insert(
            "query-page".to_owned(),
            TypeDef {
                name: vec!["query".to_owned()],
                direction: pons_ir::Direction::Input,
                definition: TypeKind::Struct {
                    fields: Vec::new(),
                    flattened: Vec::new(),
                    openness: pons_ir::ObjectOpenness::Reject,
                },
                origin: None,
                doc: None,
                diagnostics: Vec::new(),
                ext: BTreeMap::new(),
            },
        );
        types.insert("common".to_owned(), definition("common"));
        types.insert("orphan".to_owned(), definition("orphan"));
        Document {
            ir_version: pons_ir::IR_VERSION.to_owned(),
            info: Info {
                title: "Slots".to_owned(),
                version: "1".to_owned(),
                description: None,
                servers: Vec::new(),
                ext: BTreeMap::new(),
            },
            types,
            auth_schemes: BTreeMap::new(),
            common_errors: vec![ErrorResponse {
                status: pons_ir::ErrorStatus::Exact(418),
                name: vec!["teapot".to_owned()],
                content: pons_ir::ErrorContent::Json,
                media: None,
                body: Some(slot("common")),
                headers: Vec::new(),
                retryable: None,
                doc: None,
                ext: BTreeMap::new(),
            }],
            operations: vec![operation("slots", errors_inherit)],
            ext: BTreeMap::new(),
            diagnostics: Vec::new(),
        }
    }

    fn operation(id: &str, errors_inherit: bool) -> pons_ir::Operation {
        pons_ir::Operation {
            id: vec![vec![id.to_owned()]],
            method: pons_ir::Method::Post,
            path: vec![
                pons_ir::PathSegment::Literal {
                    lit: "slots".to_owned(),
                },
                pons_ir::PathSegment::Param {
                    param: "id".to_owned(),
                    ty: slot("path"),
                },
            ],
            query: Some(slot("query")),
            headers: vec![pons_ir::HeaderParameter {
                wire: "X-Slot".to_owned(),
                ident: vec!["slot".to_owned()],
                ty: slot("request-header"),
                presence: pons_ir::Presence::Required,
                ext: BTreeMap::new(),
            }],
            body: Some(pons_ir::Body {
                ty: slot("body"),
                content: pons_ir::ContentKind::Json,
                required: true,
                media: None,
                ext: BTreeMap::new(),
            }),
            responses: vec![
                pons_ir::Response {
                    status: pons_ir::ResponseStatus::Exact(200),
                    content: pons_ir::ContentKind::Stream,
                    media: Some(pons_ir::MediaDescriptor::from_declared(
                        "application/x-ndjson",
                    )),
                    body: None,
                    headers: Vec::new(),
                    ext: BTreeMap::new(),
                },
                pons_ir::Response {
                    status: pons_ir::ResponseStatus::Exact(201),
                    content: pons_ir::ContentKind::Json,
                    media: None,
                    body: Some(slot("response-body")),
                    headers: vec![response_header("response-header")],
                    ext: BTreeMap::new(),
                },
            ],
            errors: vec![ErrorResponse {
                status: pons_ir::ErrorStatus::Exact(409),
                name: vec!["conflict".to_owned()],
                content: pons_ir::ErrorContent::Json,
                media: None,
                body: Some(slot("error-body")),
                headers: Vec::new(),
                retryable: None,
                doc: None,
                ext: BTreeMap::new(),
            }],
            errors_inherit,
            stream: Some(pons_ir::StreamDescription {
                kind: pons_ir::StreamKind::Ndjson {
                    item: slot("stream"),
                },
            }),
            auth: AuthRequirement::default(),
            provides: None,
            semantics: pons_ir::OperationSemantics {
                safe: false,
                idempotent: false,
                pagination: None,
            },
            doc: None,
            origin: None,
            ext: BTreeMap::new(),
            diagnostics: Vec::new(),
        }
    }

    fn response_header(id: &str) -> ResponseHeader {
        ResponseHeader {
            wire: "X-Slot".to_owned(),
            ty: slot(id),
            presence: pons_ir::Presence::Required,
            ext: BTreeMap::new(),
        }
    }

    fn slot(id: &str) -> TypeRef {
        TypeRef::Named(pons_ir::NamedRef {
            id: id.to_owned(),
            variant: None,
        })
    }

    fn definition(id: &str) -> TypeDef {
        TypeDef {
            name: vec![id.to_owned()],
            direction: pons_ir::Direction::Both,
            definition: TypeKind::Newtype {
                inner: TypeRef::Inline(pons_ir::InlineType::String {
                    format: None,
                    min_length: None,
                    max_length: None,
                    pattern: None,
                }),
            },
            origin: None,
            doc: None,
            diagnostics: Vec::new(),
            ext: BTreeMap::new(),
        }
    }

    fn newtype(id: &str, inner: TypeRef) -> TypeDef {
        TypeDef {
            name: vec![id.to_owned()],
            direction: pons_ir::Direction::Both,
            definition: TypeKind::Newtype { inner },
            origin: None,
            doc: None,
            diagnostics: Vec::new(),
            ext: BTreeMap::new(),
        }
    }
}
