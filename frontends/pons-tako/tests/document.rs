use std::collections::BTreeMap;

use pons_core::Schema;
use pons_ir::{
    ContentKind, InlineType, Method, PathSegment, Presence, ResponseStatus, TypeKind, TypeRef,
};
use pons_tako::{Info, OperationMeta, Router, WithStatus};
use serde::{Deserialize, Serialize};
use tako::extractors::json::Json;
use tako::extractors::path::Path;
use tako::extractors::query::Query;
#[cfg(feature = "typed-header")]
use tako::extractors::typed_header::TypedHeader;

#[derive(Deserialize, Schema)]
struct UserId {
    id: u64,
}

#[derive(Deserialize, Schema)]
#[serde(deny_unknown_fields)]
struct ListQuery {
    limit: Option<u16>,
}

#[derive(Deserialize, Schema)]
struct CreateUser {
    name: String,
}

#[derive(Serialize, Schema)]
struct User {
    id: u64,
    name: String,
}

fn info() -> Info {
    Info {
        title: "Users".into(),
        version: "1".into(),
        description: None,
        servers: Vec::new(),
        ext: BTreeMap::new(),
    }
}

#[cfg(feature = "typed-header")]
async fn get_user(
    Path(user): Path<UserId>,
    Query(query): Query<ListQuery>,
    TypedHeader(user_agent): TypedHeader<headers::UserAgent>,
) -> Json<User> {
    let _ = (user.id, query.limit, user_agent);
    Json(User {
        id: user.id,
        name: "Ada".into(),
    })
}

#[cfg(not(feature = "typed-header"))]
async fn get_user(Path(user): Path<UserId>, Query(query): Query<ListQuery>) -> Json<User> {
    let _ = (user.id, query.limit);
    Json(User {
        id: user.id,
        name: "Ada".into(),
    })
}

async fn create_user(Json(input): Json<CreateUser>) -> WithStatus<201, Json<User>> {
    WithStatus::new(Json(User {
        id: 1,
        name: input.name,
    }))
}

async fn remove_user(Path(user): Path<UserId>) {
    let _ = user.id;
}

fn router() -> Router {
    let mut router = Router::new(info());
    router.get("/users/{id}", get_user);
    router.post("/users", create_user);
    router.delete_with(
        "/users/{id}",
        remove_user,
        OperationMeta::new("users.remove")
            .summary("Remove a user")
            .description("Deletes the user with the given id.")
            .tag("users")
            .deprecated(true),
    );
    router
}

fn operation(document: &pons_ir::Document, method: Method) -> &pons_ir::Operation {
    document
        .operations
        .iter()
        .find(|operation| operation.method == method)
        .unwrap_or_else(|| panic!("no {method:?} operation"))
}

#[test]
fn describes_path_query_body_and_exact_response() {
    let document = router().into_document().unwrap();
    document.validate().unwrap();
    assert_eq!(document.operations.len(), 3);

    let get = operation(&document, Method::Get);
    assert_eq!(
        get.path,
        vec![
            PathSegment::Literal {
                lit: "users".into()
            },
            PathSegment::Param {
                param: "id".into(),
                ty: TypeRef::Inline(InlineType::Int {
                    bits: 64,
                    signed: false,
                    wire: pons_ir::IntegerWire::Number,
                    minimum: None,
                    maximum: None,
                    multiple_of: None,
                }),
            },
        ]
    );

    let TypeRef::Named(query) = get.query.as_ref().expect("the query is described") else {
        panic!("{:?} is not a named type", get.query)
    };
    let TypeKind::Struct { fields, .. } = &document.types[&query.id].definition else {
        panic!("{} is not a struct", query.id)
    };
    let limit = fields.iter().find(|field| field.wire == "limit").unwrap();
    assert_eq!(limit.presence, Presence::Optional);
    assert!(matches!(
        limit.ty,
        TypeRef::Inline(InlineType::Int { bits: 16, .. })
    ));

    #[cfg(feature = "typed-header")]
    {
        assert_eq!(get.headers.len(), 1);
        assert_eq!(get.headers[0].wire, "user-agent");
        assert_eq!(get.headers[0].presence, Presence::Required);
        assert!(matches!(
            get.headers[0].ty,
            TypeRef::Inline(InlineType::Opaque {
                wire: pons_ir::OpaqueWire::Text,
                ..
            })
        ));
    }

    let post = operation(&document, Method::Post);
    let body = post.body.as_ref().expect("the JSON body is described");
    assert_eq!(body.content, ContentKind::Json);
    assert!(body.required);
    let TypeRef::Named(created) = &body.ty else {
        panic!("{:?} is not a named type", body.ty)
    };
    let TypeKind::Struct { fields, .. } = &document.types[&created.id].definition else {
        panic!("{} is not a struct", created.id)
    };
    assert!(fields.iter().any(|field| field.wire == "name"));

    assert_eq!(post.responses.len(), 1);
    assert_eq!(post.responses[0].status, ResponseStatus::Exact(201));
    assert_eq!(post.responses[0].content, ContentKind::Json);
    assert!(matches!(post.responses[0].body, Some(TypeRef::Named(_))));

    let remove = operation(&document, Method::Delete);
    assert_eq!(
        remove.id,
        vec![vec!["users".to_owned()], vec!["remove".to_owned()]]
    );
    let doc = remove.doc.as_ref().expect("the meta is recorded");
    assert_eq!(doc.summary.as_deref(), Some("Remove a user"));
    assert_eq!(
        doc.description.as_deref(),
        Some("Deletes the user with the given id.")
    );
    assert_eq!(doc.tags, vec!["users".to_owned()]);
    assert_eq!(doc.deprecated, Some(true));
}

#[test]
fn into_router_serves_the_described_paths() {
    let router = router().into_router();
    let paths = router
        .routes()
        .iter()
        .map(|route| route.path.to_string())
        .collect::<Vec<_>>();
    assert_eq!(paths.len(), 3);
    assert!(paths.iter().any(|path| path == "/users/{id}"));
    assert!(paths.iter().any(|path| path == "/users"));
}
