use pons_axum::extract::State;
use pons_axum::{DescribeRequest, NoContent, OperationCx};
use pons_core::Schema;
use pons_ir::{CredentialLocator, CredentialProvision};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Deserialize, Schema)]
pub struct UserPath {
    pub id: u64,
}

#[derive(Deserialize, Schema)]
#[serde(deny_unknown_fields)]
pub struct UserQuery {
    #[serde(default)]
    pub limit: Option<u16>,
}

#[derive(Serialize, Schema)]
pub struct User {
    pub id: u64,
    pub name: String,
}

#[derive(Deserialize, Schema)]
#[serde(deny_unknown_fields)]
pub struct CreateUser {
    pub name: String,
}

pub struct Provides;

impl pons_axum::extract::FromRequestParts<AppState> for Provides {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        _: &mut pons_axum::http::request::Parts,
        _: &AppState,
    ) -> Result<Self, Self::Rejection> {
        Ok(Self)
    }
}

impl DescribeRequest for Provides {
    fn describe(cx: &mut OperationCx) {
        cx.provide(CredentialProvision {
            scheme: "session".into(),
            value: CredentialLocator::Body {
                pointer: "/token".into(),
            },
            expires_at: None,
            ext: BTreeMap::new(),
        });
    }
}

#[derive(Clone)]
pub struct AppState;

pub async fn list_users(State(_state): State<AppState>) -> axum::Json<Vec<User>> {
    axum::Json(vec![User {
        id: 1,
        name: "Ada".into(),
    }])
}

pub async fn health() {}

pub async fn delete_user() -> NoContent {
    NoContent
}

pub async fn first_user() -> axum::Json<User> {
    axum::Json(User {
        id: 1,
        name: "Ada".into(),
    })
}

pub async fn second_user() -> NoContent {
    NoContent
}
