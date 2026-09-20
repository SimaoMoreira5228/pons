use pons_axum::Router;
use pons_axum::extract::{Path, Query, State};
use pons_axum::http::StatusCode;
use pons_axum::routing::get;
use pons_core::Schema;
use pons_ir::{AuthScheme, ContentKind, Info, InlineType, PathSegment, TypeKind, TypeRef};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Deserialize, Schema)]
struct UserPath {
    id: u64,
}

#[derive(Deserialize, Schema)]
struct UserQuery {
    limit: Option<u16>,
}

#[derive(Serialize, Schema)]
struct User {
    id: u64,
    name: String,
}

#[derive(Serialize, Schema)]
struct UserEvent {
    id: u64,
}

impl pons_axum::SseEvent for UserEvent {
    const NAME: &'static str = "user";
}

#[derive(Deserialize, Schema)]
struct CreateUser {
    name: String,
}

#[derive(Clone)]
struct AppState;

async fn get_user(
    Path(path): Path<UserPath>,
    Query(query): Query<UserQuery>,
    State(state): State<AppState>,
) -> pons_axum::extract::Json<User> {
    let _ = (path.id, query.limit, state);
    pons_axum::extract::Json(User {
        id: 1,
        name: "Ada".into(),
    })
}

async fn list_users(State(_state): State<AppState>) -> pons_axum::extract::Json<Vec<User>> {
    pons_axum::extract::Json(vec![User {
        id: 1,
        name: "Ada".into(),
    }])
}

async fn health() {}

async fn user_events()
-> pons_axum::Sse<UserEvent, impl futures_util::Stream<Item = UserEvent> + Send> {
    pons_axum::Sse::new(futures_util::stream::iter([UserEvent { id: 1 }]))
}

#[derive(Serialize, Schema, pons_core::ApiError)]
#[serde(rename_all = "snake_case")]
enum UserError {
    #[pons(status = 404)]
    Missing,
}

impl axum::response::IntoResponse for UserError {
    fn into_response(self) -> axum::response::Response {
        StatusCode::NOT_FOUND.into_response()
    }
}

async fn typed_user() -> Result<pons_axum::extract::Json<User>, UserError> {
    Err(UserError::Missing)
}

async fn create_user(
    pons_axum::extract::Json(input): pons_axum::extract::Json<CreateUser>,
) -> (StatusCode, pons_axum::extract::Json<User>) {
    let name = input.name;
    (
        StatusCode::CREATED,
        pons_axum::extract::Json(User { id: 1, name }),
    )
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

#[test]
fn router_describes_path_query_body_response_and_state() {
    let document = Router::<AppState>::new(info())
        .route("/users/{id}", get(get_user))
        .route("/users", get(list_users).post(create_user))
        .route("/", get(health))
        .route("/typed", get(typed_user))
        .route("/events", get(user_events))
        .with_state::<()>(AppState)
        .auth_scheme("session", AuthScheme::Bearer { format: None })
        .into_document()
        .unwrap();

    let get_user = document
        .operations
        .iter()
        .find(|operation| operation.method == pons_ir::Method::Get)
        .unwrap();
    assert_eq!(get_user.id.last().unwrap().join("_"), "get_user");
    assert_eq!(get_user.path.len(), 2);
    let PathSegment::Param { param, ty } = &get_user.path[1] else {
        panic!();
    };
    assert_eq!(param, "id");
    assert!(matches!(
        ty,
        TypeRef::Inline(InlineType::Int { bits: 64, .. })
    ));
    assert!(matches!(get_user.query, Some(TypeRef::Named(_))));
    assert!(get_user.body.is_none());
    assert_eq!(get_user.responses[0].content, ContentKind::Json);
    assert!(
        get_user
            .responses
            .iter()
            .any(|response| matches!(response.body, Some(TypeRef::Named(_))))
    );

    let post = document
        .operations
        .iter()
        .find(|operation| operation.method == pons_ir::Method::Post)
        .unwrap();
    assert_eq!(post.body.as_ref().unwrap().content, ContentKind::Json);
    assert_eq!(post.responses[0].status, 200);
    assert!(
        post.diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "PONS-W101")
    );
    assert!(document.types.values().any(|definition| {
        matches!(&definition.definition, TypeKind::Struct { fields, .. } if fields.iter().any(|field| field.wire == "name"))
    }));
    let health = document
        .operations
        .iter()
        .find(|operation| {
            operation
                .id
                .last()
                .is_some_and(|name| name.join("_") == "health")
        })
        .unwrap();
    assert!(health.path.is_empty());
    let typed = document
        .operations
        .iter()
        .find(|operation| {
            operation
                .path
                .iter()
                .any(|segment| matches!(segment, PathSegment::Literal { lit } if lit == "typed"))
        })
        .unwrap();
    assert_eq!(typed.errors.len(), 1);
    assert_eq!(typed.errors[0].status, pons_ir::ErrorStatus::Exact(404));
    let events = document
        .operations
        .iter()
        .find(|operation| operation.stream.is_some())
        .unwrap();
    let Some(pons_ir::StreamDescription {
        kind: pons_ir::StreamKind::Sse { events: messages },
    }) = &events.stream
    else {
        panic!()
    };
    assert_eq!(messages[0].event, "user");
    assert!(matches!(messages[0].data, TypeRef::Named(_)));
    assert_eq!(events.responses[0].content, ContentKind::Stream);
}
