#![cfg(feature = "macros")]

use std::collections::BTreeMap;

use pons_core::Schema;
use pons_ir::{Document, Method, Operation, PathSegment};
use pons_tako::{Info, Router};
use serde::{Deserialize, Serialize};
use tako::extractors::json::Json;
use tako::extractors::path::Path;

#[derive(Deserialize, Schema)]
struct UserId {
    id: u64,
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
    assert_eq!(document.operations.len(), 2);

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
