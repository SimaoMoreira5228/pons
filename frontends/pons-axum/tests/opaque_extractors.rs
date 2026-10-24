use axum::Json;
use pons_axum::Router;
use pons_axum::extract::{RawForm, RawPathParams, RawQuery, Request};
use pons_axum::routing::{get, post};
use pons_core::Schema;
use pons_ir::{ContentKind, Info, InlineType, OpaqueWire, PathSegment, TypeRef};
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Serialize, Schema)]
struct User {
    id: u64,
}

async fn raw_query(_query: RawQuery) -> Json<User> {
    Json(User { id: 1 })
}

async fn raw_form(_form: RawForm) -> Json<User> {
    Json(User { id: 1 })
}

async fn raw_path(_path: RawPathParams) -> Json<User> {
    Json(User { id: 1 })
}

async fn raw_request(_request: Request) -> Json<User> {
    Json(User { id: 1 })
}

fn info() -> Info {
    Info {
        title: "Opaque extractors".into(),
        version: "1".into(),
        description: None,
        servers: Vec::new(),
        ext: BTreeMap::new(),
    }
}

fn operation<'a>(document: &'a pons_ir::Document, literal: &str) -> &'a pons_ir::Operation {
    document
        .operations
        .iter()
        .find(|operation| {
            operation
                .path
                .iter()
                .any(|segment| matches!(segment, PathSegment::Literal { lit } if lit == literal))
        })
        .unwrap()
}

#[test]
fn a_document_of_raw_extractors_reports_diagnostics_it_does_not_validate() {
    let router = Router::<()>::new(info())
        .route("/query", get(raw_query))
        .route("/form", post(raw_form))
        .route("/path/{id}", get(raw_path))
        .route("/request", post(raw_request));
    let document = router.document();

    let query = operation(&document, "query");
    assert!(query.query.is_none());
    assert!(query.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "PONS-E102"
            && diagnostic.severity == pons_ir::DiagnosticSeverity::Error
            && diagnostic.message.contains("raw query strings")
    }));

    let form = operation(&document, "form");
    assert_eq!(form.body.as_ref().unwrap().content, ContentKind::Form);
    assert_eq!(
        form.body.as_ref().unwrap().ty,
        TypeRef::Inline(InlineType::Opaque {
            codec: "axum.form".into(),
            wire: OpaqueWire::Unknown,
            ext: Default::default(),
        })
    );
    assert!(form.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "PONS-W101" && diagnostic.message.contains("raw form fields")
    }));

    let path = operation(&document, "path");
    assert!(path.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "PONS-W101" && diagnostic.message.contains("raw path parameters")
    }));
    assert!(path.path.iter().any(|segment| {
        matches!(
            segment,
            PathSegment::Param {
                ty: TypeRef::Inline(InlineType::String { .. }),
                ..
            }
        )
    }));

    let request = operation(&document, "request");
    assert!(request.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "PONS-E102"
            && diagnostic.severity == pons_ir::DiagnosticSeverity::Error
            && diagnostic.message.contains("raw request extractors")
    }));
    assert!(router.into_document().is_err());
}
