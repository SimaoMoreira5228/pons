#![cfg(feature = "macros")]

use std::collections::BTreeMap;

use pons_core::Schema;
use pons_ir::{ContentKind, Document, Method, Operation, PathSegment, ResponseStatus};
use pons_tako::{Info, OperationMeta, Router};
use serde::{Deserialize, Serialize};
use tako::extractors::json::Json;
use tako::extractors::path::Path;
use tako::extractors::query::Query;
use tako::responder::Responder;

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
struct UpdateUser {
    name: String,
}

#[derive(Serialize, Schema)]
struct User {
    id: u64,
    name: String,
}

#[pons_tako::get("/users/{id}", name = "users.get")]
async fn get_user(Path(user): Path<UserId>) -> Json<User> {
    Json(User {
        id: user.id,
        name: "Ada".into(),
    })
}

#[pons_tako::route(GET, "/teams/{id}")]
async fn get_team(Path(team): Path<UserId>) -> Json<User> {
    Json(User {
        id: team.id,
        name: "Core".into(),
    })
}

#[pons_tako::get("/opaque/users/{id}", response = Json<User>)]
async fn opaque_get_user(Path(user): Path<UserId>) -> impl Responder {
    Json(User {
        id: user.id,
        name: "Ada".into(),
    })
}

async fn concrete_get_user(Path(user): Path<UserId>) -> Json<User> {
    Json(User {
        id: user.id,
        name: "Ada".into(),
    })
}

#[pons_tako::route(
    POST,
    "/teams/{id}",
    response = Json<User>,
    name = "teams.update"
)]
async fn opaque_update_team(
    Query(query): Query<ListQuery>,
    Path(team): Path<UserId>,
    Json(input): Json<UpdateUser>,
) -> impl Responder {
    let _ = query.limit;
    Json(User {
        id: team.id,
        name: input.name,
    })
}

async fn concrete_update_team(
    Path(team): Path<UserId>,
    Query(query): Query<ListQuery>,
    Json(input): Json<UpdateUser>,
) -> Json<User> {
    let _ = query.limit;
    Json(User {
        id: team.id,
        name: input.name,
    })
}

fn info() -> Info {
    Info {
        title: "Attribute routes".into(),
        version: "1".into(),
        description: None,
        servers: Vec::new(),
        ext: BTreeMap::new(),
    }
}

fn operation<'a>(document: &'a Document, first: &str) -> &'a Operation {
    document
        .operations
        .iter()
        .find(|operation| {
            matches!(
                operation.path.first(),
                Some(PathSegment::Literal { lit }) if lit == first
            )
        })
        .unwrap_or_else(|| panic!("no operation starting with {first}"))
}

#[test]
fn attribute_routes_mount_into_the_document() {
    let document = Router::new(info()).mount_all().document();
    document.validate().unwrap();
    assert_eq!(document.operations.len(), 4);

    let users = operation(&document, "users");
    assert_eq!(users.method, Method::Get);
    assert_eq!(
        users.id,
        vec![vec!["users".to_owned()], vec!["get".to_owned()]]
    );
    assert_eq!(users.path.len(), 2);
    assert!(matches!(&users.path[0], PathSegment::Literal { lit } if lit == "users"));
    assert!(matches!(&users.path[1], PathSegment::Param { param, .. } if param == "id"));
    assert!(users.doc.is_some());

    let teams = operation(&document, "teams");
    assert_eq!(teams.method, Method::Get);
    assert_eq!(
        teams.id,
        vec![
            vec!["get".to_owned()],
            vec!["teams".to_owned()],
            vec!["id".to_owned()],
        ]
    );
    assert_eq!(teams.path.len(), 2);
    assert!(matches!(&teams.path[0], PathSegment::Literal { lit } if lit == "teams"));
    assert!(matches!(&teams.path[1], PathSegment::Param { param, .. } if param == "id"));
    assert!(teams.doc.is_none());
}

#[test]
fn a_declared_response_matches_a_concrete_return() {
    let document = Router::new(info()).mount_all().document();
    document.validate().unwrap();
    let described = operation(&document, "opaque");

    let mut typed_router = Router::new(info());
    typed_router.get("/opaque/users/{id}", concrete_get_user);
    let typed_document = typed_router.document();
    let typed = operation(&typed_document, "opaque");

    assert_eq!(described, typed);
    assert_eq!(described.responses.len(), 1);
    assert_eq!(described.responses[0].status, ResponseStatus::Exact(200));
    assert_eq!(described.responses[0].content, ContentKind::Json);
}

#[test]
fn a_route_response_with_a_name_matches_a_concrete_return() {
    let document = Router::new(info()).mount_all().document();
    document.validate().unwrap();
    let described = document
        .operations
        .iter()
        .find(|operation| operation.id == vec![vec!["teams".to_owned()], vec!["update".to_owned()]])
        .expect("the named route is registered");

    let mut typed_router = Router::new(info());
    typed_router.post_with(
        "/teams/{id}",
        concrete_update_team,
        OperationMeta::new("teams.update"),
    );
    let typed_document = typed_router.document();
    let typed = &typed_document.operations[0];

    assert_eq!(described, typed);
    assert!(described.body.is_some());
    assert!(described.query.is_some());
    assert_eq!(described.responses.len(), 1);
    assert_eq!(described.responses[0].content, ContentKind::Json);
}
