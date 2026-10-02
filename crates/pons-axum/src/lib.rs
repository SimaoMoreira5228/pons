mod auth;
mod describe;
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

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use axum::Extension;
use pons_core::{ErrorSpec, TypeCx};

pub use auth::{Auth, AuthSpec, SessionAuth};
pub use axum::{body, http};
pub use describe::{
    DescribeRequest, DescribeResponse, Header, HeaderRejection, HeaderSpec, NoContent, OperationCx,
    OptionalHeader, StatusResponse, WithHeader, WithOptionalHeader, WithStatus,
};
pub use error::{Violation, ViolationReport, WithErrorHeader, WithOptionalErrorHeader};
pub use handler::DescribeHandler;
pub use layer::DocumentedLayer;
pub use metadata::OperationMeta;
pub use ndjson::Ndjson;
use operation::build;
pub use pons_ir::Info;
use pons_ir::{
    AuthRequirement, AuthScheme, Diagnostic, Document, ErrorResponse, ResponseHeader, TypeDef,
    TypeKind, TypeRef, ValidationError,
};
pub use raw_bytes::RawBytes;
pub use request::{Form, Json, Path, Query};
pub use response::{
    BytesResponse, JsonResponse, ProblemJsonResponse, ResponseSpec, TextResponse, WithResponse,
    WithUnknownStatus,
};
pub use routing::MethodRoute;
pub use sse::{Sse, SseEvent};

pub struct Router<S = ()> {
    inner: axum::Router<S>,
    info: Info,
    types: BTreeMap<String, TypeDef>,
    carriers: Vec<TypeRef>,
    auth_schemes: BTreeMap<String, AuthScheme>,
    common_errors: Vec<ErrorResponse>,
    operations: Vec<pons_ir::Operation>,
    diagnostics: Vec<Diagnostic>,
    auth_requirement: AuthRequirement,
    pending_layers: Vec<layer::BoxedLayer>,
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
            auth_requirement: AuthRequirement::default(),
            pending_layers: Vec::new(),
        }
    }

    pub fn route(mut self, path: &str, route: MethodRoute<S>) -> Self {
        self.inner = self.inner.route(path, route.inner);
        for description in route.descriptions {
            let built = build(path, description);
            self.types.extend(built.types);
            self.carriers.extend(built.carriers);
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
            carriers: self.carriers,
            auth_schemes: self.auth_schemes,
            common_errors: self.common_errors,
            operations: self.operations,
            diagnostics: self.diagnostics,
            auth_requirement: self.auth_requirement,
            pending_layers: self.pending_layers,
        }
    }

    pub fn into_axum(mut self) -> axum::Router<S> {
        for layer in std::mem::take(&mut self.pending_layers) {
            self.inner = self.inner.layer(layer);
        }
        let inner = std::mem::take(&mut self.inner);
        let document = checked(self.into_document_value()).unwrap_or_else(|errors| {
            panic!(
                "the document Router::into_axum installs does not validate:\n{}",
                errors
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("\n")
            )
        });
        inner.layer(Extension(Arc::new(document)))
    }

    pub fn document(&self) -> Document {
        let mut operations = self.operations.clone();
        for operation in &mut operations {
            auth::merge_auth_requirements(&mut operation.auth, &self.auth_requirement);
        }
        reuse_inherited_slots(&self.common_errors, &mut operations);
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
        prune_unreferenced_types(&mut document, &self.carriers);
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
        checked(self.into_document_value())
    }

    fn into_document_value(mut self) -> Document {
        for operation in &mut self.operations {
            auth::merge_auth_requirements(&mut operation.auth, &self.auth_requirement);
        }
        reuse_inherited_slots(&self.common_errors, &mut self.operations);
        let mut document = Document {
            ir_version: pons_ir::IR_VERSION.to_owned(),
            info: self.info,
            types: self.types,
            auth_schemes: self.auth_schemes,
            common_errors: self.common_errors,
            operations: self.operations,
            ext: BTreeMap::new(),
            diagnostics: self.diagnostics,
        };
        prune_unreferenced_types(&mut document, &self.carriers);
        document
    }
}

fn checked(mut document: Document) -> Result<Document, Vec<ValidationError>> {
    document.canonicalize();
    document.validate()?;
    Ok(document)
}

fn prune_unreferenced_types(document: &mut Document, carriers: &[TypeRef]) {
    let mut reachable = BTreeSet::new();
    let mut pending = root_references(document);
    pending.extend_from_slice(carriers);
    while let Some(ty) = pending.pop() {
        let TypeRef::Named(named) = &ty else {
            pending.extend(inline_references(&ty));
            continue;
        };
        if !reachable.insert(named.id.clone()) {
            continue;
        }
        if let Some(definition) = document.types.get(&named.id) {
            pending.extend(definition_references(&definition.definition));
        }
    }
    document.types.retain(|id, _| reachable.contains(id));
}

fn root_references(document: &Document) -> Vec<TypeRef> {
    let mut roots = Vec::new();
    let push = |body: &Option<TypeRef>, headers: &[ResponseHeader], roots: &mut Vec<TypeRef>| {
        roots.extend(body.clone());
        roots.extend(headers.iter().map(|header| header.ty.clone()));
    };
    for operation in &document.operations {
        operation.path.iter().for_each(|segment| {
            if let pons_ir::PathSegment::Param { ty, .. } = segment {
                roots.push(ty.clone());
            }
        });
        roots.extend(operation.query.clone());
        operation
            .headers
            .iter()
            .for_each(|header| roots.push(header.ty.clone()));
        if let Some(body) = &operation.body {
            roots.push(body.ty.clone());
        }
        operation
            .responses
            .iter()
            .for_each(|response| push(&response.body, &response.headers, &mut roots));
        operation
            .errors
            .iter()
            .for_each(|error| push(&error.body, &error.headers, &mut roots));
        if let Some(stream) = &operation.stream {
            match &stream.kind {
                pons_ir::StreamKind::Sse { events } => events
                    .iter()
                    .for_each(|event| roots.push(event.data.clone())),
                pons_ir::StreamKind::Ndjson { item } => roots.push(item.clone()),
                pons_ir::StreamKind::Bytes { .. } => {}
                pons_ir::StreamKind::Websocket { client, server } => {
                    roots.push(client.clone());
                    roots.push(server.clone());
                }
            }
        }
    }
    document
        .common_errors
        .iter()
        .for_each(|error| push(&error.body, &error.headers, &mut roots));
    roots
}

fn definition_references(definition: &TypeKind) -> Vec<TypeRef> {
    match definition {
        TypeKind::Struct {
            fields,
            flattened,
            openness,
        } => {
            let mut references = fields
                .iter()
                .map(|field| field.ty.clone())
                .collect::<Vec<_>>();
            references.extend(flattened.clone());
            if let pons_ir::ObjectOpenness::PreserveTyped { value } = openness {
                references.push(value.clone());
            }
            references
        }
        TypeKind::Enum { variants, .. } => variants
            .iter()
            .flat_map(|variant| match &variant.payload {
                pons_ir::VariantPayload::Unit => Vec::new(),
                pons_ir::VariantPayload::Newtype { ty } => vec![ty.clone()],
                pons_ir::VariantPayload::Tuple { items } => items.clone(),
                pons_ir::VariantPayload::Struct { fields } => {
                    fields.iter().map(|field| field.ty.clone()).collect()
                }
            })
            .collect(),
        TypeKind::Newtype { inner } => vec![inner.clone()],
    }
}

fn inline_references(ty: &TypeRef) -> Vec<TypeRef> {
    match ty {
        TypeRef::Inline(pons_ir::InlineType::Nullable { inner }) => vec![(**inner).clone()],
        TypeRef::Inline(pons_ir::InlineType::Array { item, .. }) => vec![(**item).clone()],
        TypeRef::Inline(pons_ir::InlineType::Map { key, value }) => {
            vec![(**key).clone(), (**value).clone()]
        }
        TypeRef::Inline(pons_ir::InlineType::Tuple { items }) => items.clone(),
        _ => Vec::new(),
    }
}

fn reuse_inherited_slots(common: &[ErrorResponse], operations: &mut [pons_ir::Operation]) {
    if common.is_empty() {
        return;
    }
    for operation in operations {
        let mut collisions = Vec::new();
        operation.errors.retain(|error| {
            if error.name != crate::error::REJECTED_NAME {
                return true;
            }
            let Some(inherited) = common
                .iter()
                .find(|inherited| inherited.status == error.status)
            else {
                return true;
            };
            match wire_difference(inherited, error) {
                None => false,
                Some(difference) => {
                    collisions.push((error.status.clone(), inherited.name.clone(), difference));
                    true
                }
            }
        });
        collisions
            .into_iter()
            .for_each(|(status, name, difference)| {
                operation_cx::push_diagnostic(
                    &mut operation.diagnostics,
                    "PONS-E102",
                    format!(
                        "a request is rejected as {status:?}, but the common error {name:?} answers that status with a different {difference}"
                    ),
                );
            });
    }
}

fn wire_difference(inherited: &ErrorResponse, declared: &ErrorResponse) -> Option<&'static str> {
    [
        ("body", inherited.body != declared.body),
        ("content", inherited.content != declared.content),
        ("media type", inherited.media_type != declared.media_type),
        (
            "response headers",
            !same_headers(&inherited.headers, &declared.headers),
        ),
        ("retryable", inherited.retryable != declared.retryable),
        ("extensions", inherited.ext != declared.ext),
    ]
    .into_iter()
    .find(|(_, differs)| *differs)
    .map(|(field, _)| field)
}

fn same_headers(inherited: &[ResponseHeader], declared: &[ResponseHeader]) -> bool {
    inherited.len() == declared.len()
        && inherited.iter().all(|left| {
            declared.iter().any(|right| {
                left.wire.eq_ignore_ascii_case(&right.wire)
                    && left.ty == right.ty
                    && left.presence == right.presence
                    && left.ext == right.ext
            })
        })
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
            prune_unreferenced_types(&mut document, &[]);
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
                media_type: None,
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
                media_type: None,
                ext: BTreeMap::new(),
            }),
            responses: vec![
                pons_ir::Response {
                    status: pons_ir::ResponseStatus::Exact(200),
                    content: pons_ir::ContentKind::Stream,
                    media_type: Some("application/x-ndjson".to_owned()),
                    body: None,
                    headers: Vec::new(),
                    ext: BTreeMap::new(),
                },
                pons_ir::Response {
                    status: pons_ir::ResponseStatus::Exact(201),
                    content: pons_ir::ContentKind::Json,
                    media_type: None,
                    body: Some(slot("response-body")),
                    headers: vec![response_header("response-header")],
                    ext: BTreeMap::new(),
                },
            ],
            errors: vec![ErrorResponse {
                status: pons_ir::ErrorStatus::Exact(409),
                name: vec!["conflict".to_owned()],
                content: pons_ir::ErrorContent::Json,
                media_type: None,
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
