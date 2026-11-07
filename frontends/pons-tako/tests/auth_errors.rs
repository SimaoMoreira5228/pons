use std::collections::BTreeMap;

use http::StatusCode;
use pons_core::Schema;
use pons_ir::{
    AuthScheme, Document, ErrorContent, ErrorStatus, Method, Operation, PathSegment,
    ResponseStatus, TypeRef,
};
use pons_tako::{Auth, AuthSpec, Info, OperationCx, Router, SessionAuth};
use serde::{Deserialize, Serialize};
use tako::extractors::extension::Extension;
use tako::extractors::json::Json;
use tako::extractors::path::Path;
use tako_rs_core::responder::Responder;
use tako_rs_core::types::Response as TakoResponse;

#[derive(Clone)]
struct Principal;

struct OptionalSession;

impl AuthSpec for OptionalSession {
    fn apply(cx: &mut OperationCx) {
        cx.require_all(["session"]);
        cx.optional();
    }
}

#[derive(Deserialize, Schema)]
struct UserId {
    id: u64,
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

#[derive(Debug, Serialize, Schema, pons_core::ApiError)]
#[serde(tag = "code", rename_all = "snake_case")]
#[allow(dead_code)]
enum LoginError {
    #[pons(status = 401)]
    InvalidCredentials,
    #[pons(status = 423, retryable = false)]
    Locked { retry_after_secs: u32 },
}

impl Responder for LoginError {
    fn into_response(self) -> TakoResponse {
        let status = match &self {
            Self::InvalidCredentials => StatusCode::UNAUTHORIZED,
            Self::Locked { .. } => StatusCode::from_u16(423).expect("423 is a status"),
        };
        (status, Json(self)).into_response()
    }
}

async fn me(auth: Auth<Extension<Principal>, SessionAuth>, Path(user): Path<UserId>) -> Json<User> {
    let _ = auth.into_inner().0;
    Json(User {
        id: user.id,
        name: "Ada".into(),
    })
}

async fn optional_profile(
    auth: Auth<Extension<Principal>, OptionalSession>,
    Path(user): Path<UserId>,
) -> Json<User> {
    let _ = auth.into_inner().0;
    Json(User {
        id: user.id,
        name: "Ada".into(),
    })
}

async fn login(Json(input): Json<CreateUser>) -> Result<Json<User>, LoginError> {
    let _ = input.name;
    Ok(Json(User {
        id: 1,
        name: "Ada".into(),
    }))
}

fn info() -> Info {
    Info {
        title: "Auth and errors".into(),
        version: "1".into(),
        description: None,
        servers: Vec::new(),
        ext: BTreeMap::new(),
    }
}

fn router() -> Router {
    let mut router = Router::new(info());
    router.auth_scheme("session", AuthScheme::Bearer { format: None });
    router.get("/me/{id}", me);
    router.get("/optional/{id}", optional_profile);
    router.post("/login", login);
    router
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

fn statuses(operation: &Operation) -> Vec<&ErrorStatus> {
    operation.errors.iter().map(|error| &error.status).collect()
}

#[test]
fn auth_wrapper_publishes_the_required_scheme() {
    let document = router().into_document().unwrap();
    document.validate().unwrap();

    let me = operation(&document, "me");
    assert_eq!(me.auth.alternatives, vec![vec![String::from("session")]]);
    assert!(!me.auth.optional);
    assert!(me.auth.scopes.is_empty());
}

#[test]
fn optional_auth_route_publishes_optionality() {
    let document = router().into_document().unwrap();
    document.validate().unwrap();

    let optional = operation(&document, "optional");
    assert_eq!(
        optional.auth.alternatives,
        vec![vec![String::from("session")]]
    );
    assert!(optional.auth.optional);
}

#[test]
fn result_describes_typed_error_statuses_and_bodies() {
    let document = router().into_document().unwrap();
    document.validate().unwrap();

    let login = operation(&document, "login");
    assert_eq!(login.method, Method::Post);
    assert_eq!(login.responses.len(), 1);
    assert_eq!(login.responses[0].status, ResponseStatus::Exact(200));
    assert!(matches!(login.responses[0].body, Some(TypeRef::Named(_))));

    assert_eq!(
        statuses(login),
        vec![&ErrorStatus::Exact(401), &ErrorStatus::Exact(423)]
    );
    assert_eq!(login.errors[0].content, ErrorContent::Json);
    assert_eq!(login.errors[0].name, vec!["invalid", "credentials"]);
    assert_eq!(login.errors[1].content, ErrorContent::Json);
    assert_eq!(login.errors[1].name, vec!["locked"]);
    assert_eq!(login.errors[1].retryable, Some(false));

    for (error, variant) in login.errors.iter().zip(["invalid_credentials", "locked"]) {
        let Some(TypeRef::Named(named)) = &error.body else {
            panic!("{:?} is not a named error body", error.body)
        };
        assert!(
            named.id.contains("LoginError"),
            "unexpected error body id {}",
            named.id
        );
        assert_eq!(named.variant.as_deref(), Some(variant));
    }
}
