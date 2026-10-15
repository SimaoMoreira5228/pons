use axum::response::{IntoResponse, Response};
use pons_axum::http::StatusCode;
use pons_core::{ErrorSpec, Schema, TypeCx};
use pons_ir::{Direction, ErrorContent, ErrorResponse, ErrorStatus};
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Schema)]
#[serde(deny_unknown_fields)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

#[derive(Serialize, Schema)]
pub struct Session {
    pub token: String,
    pub expires_at: String,
}

#[derive(Serialize, Schema)]
pub struct InvalidCredentials {
    pub message: String,
}

impl ErrorSpec for InvalidCredentials {
    fn errors(cx: &mut TypeCx) -> Vec<ErrorResponse> {
        vec![ErrorResponse {
            status: ErrorStatus::Exact(401),
            name: vec!["invalid".into(), "credentials".into()],
            content: ErrorContent::Json,
            media_type: None,
            body: Some(<Self as Schema>::schema(cx, Direction::Output)),
            headers: Vec::new(),
            retryable: Some(true),
            doc: None,
            ext: Default::default(),
        }]
    }
}

impl IntoResponse for InvalidCredentials {
    fn into_response(self) -> Response {
        (StatusCode::UNAUTHORIZED, axum::Json(self)).into_response()
    }
}

#[derive(Serialize, Schema)]
pub struct AccountLocked {
    pub retry_after_secs: i32,
}

impl ErrorSpec for AccountLocked {
    fn errors(cx: &mut TypeCx) -> Vec<ErrorResponse> {
        vec![ErrorResponse {
            status: ErrorStatus::Exact(423),
            name: vec!["account".into(), "locked".into()],
            content: ErrorContent::Json,
            media_type: None,
            body: Some(<Self as Schema>::schema(cx, Direction::Output)),
            headers: Vec::new(),
            retryable: None,
            doc: None,
            ext: Default::default(),
        }]
    }
}

#[derive(Serialize, Schema)]
pub struct User {
    pub id: i64,
    pub name: String,
}

#[derive(Deserialize, Schema)]
#[serde(deny_unknown_fields)]
pub struct UsersGetQuery {
    pub active: Option<bool>,
    pub role: Option<RoleFilter>,
}

#[derive(Deserialize, Schema)]
#[serde(rename_all = "snake_case")]
pub enum RoleFilter {
    Admin,
    Member,
}

#[derive(Serialize, Schema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Notification {
    Mention { post_id: i64 },
    Follow { user_id: i64 },
}
