mod model;

use std::collections::BTreeMap;
use std::convert::Infallible;

use axum::extract::FromRequestParts;
use pons_axum::routing::{get_with, post_with};
use pons_axum::{
    Auth, DescribeRequest, Info, Json, OperationCx, OperationMeta, Path, Query, Router,
};
use pons_ir::{AuthScheme, CredentialLocator};

pub use model::{
    AccountLocked, InvalidCredentials, LoginRequest, Notification, RoleFilter, Session, User,
    UsersGetQuery,
};

struct SessionGuard;

impl<S: Send + Sync> FromRequestParts<S> for SessionGuard {
    type Rejection = Infallible;

    async fn from_request_parts(
        _: &mut pons_axum::http::request::Parts,
        _: &S,
    ) -> Result<Self, Self::Rejection> {
        Ok(Self)
    }
}

impl DescribeRequest for SessionGuard {
    fn describe(_: &mut OperationCx) {}
}

async fn login(
    Json(request): Json<LoginRequest>,
) -> Result<axum::Json<Session>, InvalidCredentials> {
    let _ = request.password;
    Ok(axum::Json(Session {
        token: "token".into(),
        expires_at: "2026-01-01T00:00:00Z".into(),
    }))
}

async fn get_user(
    _session: Auth<SessionGuard>,
    Path(id): Path<i64>,
    Query(query): Query<UsersGetQuery>,
) -> axum::Json<User> {
    let _ = (query.active, query.role);
    axum::Json(User {
        id,
        name: "Ada".into(),
    })
}

async fn latest_notification() -> axum::Json<Notification> {
    axum::Json(Notification::Follow { user_id: 1 })
}

pub fn router() -> Router {
    Router::<()>::new(Info {
        title: "Pons Conformance API".into(),
        version: "1.0.0".into(),
        description: None,
        servers: Vec::new(),
        ext: BTreeMap::new(),
    })
    .auth_scheme(
        "session",
        AuthScheme::Bearer {
            format: Some("jwt".into()),
        },
    )
    .common_error::<AccountLocked>()
    .route(
        "/auth/login",
        post_with(
            login,
            OperationMeta::new("auth.login")
                .summary("Exchange credentials for a session token.")
                .provides(
                    "session",
                    CredentialLocator::Body {
                        pointer: "/token".into(),
                    },
                ),
        ),
    )
    .route(
        "/users/{id}",
        get_with(
            get_user,
            OperationMeta::new("users.get").summary("Read a user."),
        ),
    )
    .route(
        "/notifications/latest",
        get_with(
            latest_notification,
            OperationMeta::new("notifications.latest").summary("Read the latest notification."),
        ),
    )
}
