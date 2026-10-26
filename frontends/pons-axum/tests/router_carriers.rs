mod support;

use pons_axum::extract::Multipart;
use pons_axum::routing::{get, post};
use pons_axum::{Form, Path, Query, Router};
use pons_core::Schema;
use pons_ir::{
    ContentKind, Field, InlineType, ObjectOpenness, OpaqueWire, PathSegment, TypeKind, TypeRef,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use support::router::*;
use support::*;

#[derive(Deserialize, Schema)]
#[serde(deny_unknown_fields)]
struct Both {
    slug: String,
    limit: Option<u16>,
}

async fn both_query(Query(query): Query<Both>) -> axum::Json<User> {
    let _ = (query.slug, query.limit);
    axum::Json(User {
        id: 1,
        name: "Ada".into(),
    })
}

async fn both_body(pons_axum::Json(body): pons_axum::Json<Both>) -> axum::Json<User> {
    let _ = (body.slug, body.limit);
    axum::Json(User {
        id: 1,
        name: "Ada".into(),
    })
}

async fn query_and_body(
    Query(_query): Query<Q>,
    pons_axum::Json(_body): pons_axum::Json<Q>,
) -> axum::Json<User> {
    axum::Json(User {
        id: 1,
        name: "Ada".into(),
    })
}

async fn query_and_path(Query(_query): Query<Q>, Path(_path): Path<Q>) -> axum::Json<User> {
    axum::Json(User {
        id: 1,
        name: "Ada".into(),
    })
}

#[derive(Deserialize, Schema)]
#[serde(deny_unknown_fields)]
struct Q {
    #[serde(default)]
    limit: Option<u16>,
}

#[derive(Deserialize)]
struct RequiredNullableQ;

impl Schema for RequiredNullableQ {
    fn schema(cx: &mut pons_core::TypeCx, direction: pons_ir::Direction) -> TypeRef {
        cx.named::<Self>(
            vec!["required".into(), "nullable".into(), "q".into()],
            direction,
            None,
            |cx| TypeKind::Struct {
                fields: vec![Field {
                    wire: "limit".into(),
                    ident: vec!["limit".into()],
                    ty: <Option<u16> as Schema>::schema(cx, direction),
                    presence: pons_ir::Presence::Required,
                    default: Default::default(),
                    access: None,
                    doc: None,
                    diagnostics: Vec::new(),
                    ext: Default::default(),
                }],
                flattened: Vec::new(),
                openness: ObjectOpenness::Reject,
            },
        )
    }
}

async fn form_user(Form(input): Form<CreateUser>) -> axum::Json<User> {
    axum::Json(User {
        id: 1,
        name: input.name,
    })
}

async fn query_q(Query(query): Query<Q>) -> axum::Json<User> {
    let _ = query.limit;
    axum::Json(User {
        id: 1,
        name: "Ada".into(),
    })
}

async fn form_q(Form(form): Form<Q>) -> axum::Json<User> {
    let _ = form.limit;
    axum::Json(User {
        id: 1,
        name: "Ada".into(),
    })
}

async fn path_q(Path(path): Path<Q>) -> axum::Json<User> {
    let _ = path.limit;
    axum::Json(User {
        id: 1,
        name: "Ada".into(),
    })
}

async fn required_nullable_q(Query(_query): Query<RequiredNullableQ>) -> axum::Json<User> {
    axum::Json(User {
        id: 1,
        name: "Ada".into(),
    })
}

async fn multipart_user(_multipart: Multipart) -> axum::Json<User> {
    axum::Json(User {
        id: 1,
        name: "Ada".into(),
    })
}

#[derive(Deserialize, Serialize, Schema)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum FlattenedCreate {
    Simple {
        name: String,
        #[serde(flatten)]
        extra: BTreeMap<String, String>,
    },
}

async fn flattened_create(
    pons_axum::Json(FlattenedCreate::Simple { name, .. }): pons_axum::Json<FlattenedCreate>,
) -> axum::Json<User> {
    axum::Json(User { id: 1, name })
}

fn optional_integer(ty: &TypeRef) -> bool {
    matches!(ty, TypeRef::Inline(InlineType::Int { bits: 16, .. }))
}

fn nullable_integer(ty: &TypeRef) -> bool {
    matches!(
        ty,
        TypeRef::Inline(InlineType::Nullable { inner }) if optional_integer(inner)
    )
}

fn limit_field<'a>(document: &'a pons_ir::Document, ty: &'a TypeRef) -> (&'a str, TypeRef) {
    let TypeRef::Named(reference) = ty else {
        panic!("{ty:?} is not a named type")
    };
    let TypeKind::Struct { fields, .. } = &document.types[&reference.id].definition else {
        panic!("{} is not a struct", reference.id)
    };
    let field = fields.iter().find(|field| field.wire == "limit").unwrap();
    assert_eq!(field.presence, pons_ir::Presence::Optional);
    (reference.id.as_str(), field.ty.clone())
}

fn q_components(document: &pons_ir::Document) -> Vec<&str> {
    document
        .types
        .keys()
        .map(String::as_str)
        .filter(|id| id.starts_with("router_carriers::Q#"))
        .collect()
}

#[test]
fn a_query_carrier_publishes_one_component_with_the_lowered_shape() {
    let document = Router::<()>::new(info())
        .route("/query", get(query_q))
        .into_document()
        .unwrap();

    assert_eq!(
        q_components(&document),
        ["router_carriers::Q#query"],
        "the derived copy no operation references is not published"
    );
    let (query_id, limit) = limit_field(
        &document,
        operation(&document, "query").query.as_ref().unwrap(),
    );
    assert_eq!(query_id, "router_carriers::Q#query");
    assert!(optional_integer(&limit), "{limit:?} is an absent key");

    let coverage = pons_ir::canonical_coverage(&document, false).unwrap();
    assert!(
        !coverage.keys().any(|subject| subject.ends_with("#input")),
        "{:?} reports a subject no operation references",
        coverage.keys().collect::<Vec<_>>()
    );
}

#[test]
fn a_route_that_takes_one_type_as_a_query_and_a_body_publishes_a_component_each() {
    let document = Router::<()>::new(info())
        .route("/both", post(query_and_body))
        .into_document()
        .unwrap();

    assert_eq!(
        q_components(&document),
        ["router_carriers::Q#input", "router_carriers::Q#query"]
    );
    let operation = operation(&document, "both");
    let (query_id, query_limit) = limit_field(&document, operation.query.as_ref().unwrap());
    assert_eq!(query_id, "router_carriers::Q#query");
    assert!(
        optional_integer(&query_limit),
        "{query_limit:?} is an absent key"
    );
    let (body_id, body_limit) = limit_field(&document, &operation.body.as_ref().unwrap().ty);
    assert_eq!(body_id, "router_carriers::Q#input");
    assert!(
        nullable_integer(&body_limit),
        "{body_limit:?} keeps the nullability a JSON body carries"
    );
}

#[test]
fn a_type_no_operation_references_is_not_published() {
    let document = Router::<()>::new(info())
        .route("/segment/{limit}", get(query_and_path))
        .into_document()
        .unwrap();

    assert_eq!(
        q_components(&document),
        ["router_carriers::Q#path", "router_carriers::Q#query"],
        "the two carriers keep their own copy and the derived one is dropped"
    );
}

#[test]
fn a_flat_carrier_publishes_a_nullable_optional_field_as_an_absent_key() {
    let document = Router::<()>::new(info())
        .route("/query", get(query_q))
        .route("/form-body", post(form_q))
        .route("/segment/{limit}", get(path_q))
        .into_document()
        .unwrap();

    let query = operation(&document, "query");
    let fields = pons_ir::query_fields(&document, query.query.as_ref().unwrap()).unwrap();
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0].wire, "limit");
    assert_eq!(fields[0].presence, pons_ir::Presence::Optional);
    assert!(optional_integer(&fields[0].ty));

    let TypeRef::Named(form) = &operation(&document, "form-body").body.as_ref().unwrap().ty else {
        panic!();
    };
    let TypeKind::Struct { fields, .. } = &document.types[&form.id].definition else {
        panic!();
    };
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0].wire, "limit");
    assert_eq!(fields[0].presence, pons_ir::Presence::Optional);
    assert!(optional_integer(&fields[0].ty));

    let PathSegment::Param { param, ty } = &operation(&document, "segment").path[1] else {
        panic!();
    };
    assert_eq!(param, "limit");
    assert!(optional_integer(ty));
}

#[test]
fn a_query_and_a_json_body_publish_a_definition_each_of_one_rust_type() {
    let document = Router::<()>::new(info())
        .route("/both-query", get(both_query))
        .route("/both-body", post(both_body))
        .into_document()
        .unwrap();

    let (query_id, query_limit) = limit_field(
        &document,
        operation(&document, "both-query").query.as_ref().unwrap(),
    );
    let (body_id, body_limit) = limit_field(
        &document,
        &operation(&document, "both-body").body.as_ref().unwrap().ty,
    );
    assert_eq!(
        body_id, "router_carriers::Both#input",
        "the JSON body keeps the derived definition"
    );
    assert_eq!(
        query_id, "router_carriers::Both#query",
        "the query is published as its own carrier"
    );
    assert!(
        nullable_integer(&body_limit),
        "{body_limit:?} keeps its nullability"
    );
    assert!(
        optional_integer(&query_limit),
        "{query_limit:?} is an absent key"
    );
}

#[test]
fn the_definitions_one_rust_type_publishes_do_not_depend_on_the_route_order() {
    fn router(query_first: bool) -> pons_axum::Router<()> {
        let query = || get(both_query);
        let body = || post(both_body);
        let router = Router::<()>::new(info());
        match query_first {
            true => router
                .route("/both-query", query())
                .route("/both-body", body()),
            false => router
                .route("/both-body", body())
                .route("/both-query", query()),
        }
    }
    assert_eq!(
        router(true).into_document().unwrap(),
        router(false).into_document().unwrap()
    );
}

#[test]
fn a_required_nullable_query_field_is_refused_by_name() {
    let errors = Router::<()>::new(info())
        .route("/required", get(required_nullable_q))
        .into_document()
        .unwrap_err();
    let errors = errors.iter().map(ToString::to_string).collect::<Vec<_>>();
    assert!(
        errors.iter().any(|error| {
            error.contains("\"limit\"") && error.contains("no encoding for a present null")
        }),
        "an Option<T> field without a serde default is required, and a query string has no encoding for a present null: {errors:?}"
    );
}

#[test]
fn router_describes_form_request_body() {
    let document = Router::<()>::new(info())
        .route("/form", post(form_user))
        .into_document()
        .unwrap();
    let operation = document
        .operations
        .iter()
        .find(|operation| {
            operation
                .path
                .iter()
                .any(|segment| matches!(segment, PathSegment::Literal { lit } if lit == "form"))
        })
        .unwrap();
    let body = operation.body.as_ref().unwrap();
    assert_eq!(body.content, ContentKind::Form);
    assert!(matches!(body.ty, TypeRef::Named(_)));
}

#[test]
fn router_describes_multipart_request_as_opaque() {
    let router = Router::<()>::new(info()).route("/multipart", post(multipart_user));
    let document = router.document();
    let operation = document
        .operations
        .iter()
        .find(|operation| {
            operation.path.iter().any(
                |segment| matches!(segment, PathSegment::Literal { lit } if lit == "multipart"),
            )
        })
        .unwrap();
    let body = operation.body.as_ref().unwrap();
    assert_eq!(body.content, ContentKind::Multipart);
    assert_eq!(
        body.ty,
        TypeRef::Inline(InlineType::Opaque {
            codec: "axum.multipart".into(),
            wire: OpaqueWire::Unknown,
            ext: Default::default(),
        })
    );
    assert!(operation.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "PONS-W101"
            && diagnostic.severity == pons_ir::DiagnosticSeverity::Warning
            && diagnostic.message.contains("multipart fields are opaque")
    }));
    assert!(router.into_document().is_ok());
}

#[test]
fn enum_payload_flatten_refuses_to_publish_a_narrowed_body() {
    let router = Router::<()>::new(info()).route("/flattened", post(flattened_create));
    let document = router.document();
    let operation = &document.operations[0];
    assert!(operation.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "PONS-E101"
            && diagnostic.severity == pons_ir::DiagnosticSeverity::Error
            && diagnostic.message.contains("flatten")
    }));
    assert!(router.into_document().is_err());
}
