mod model;

use std::collections::BTreeMap;
use std::convert::Infallible;

use axum::extract::FromRequestParts;
use axum::response::IntoResponse;
use pons_axum::routing::{get_with, post_with};
use pons_axum::{
    Auth, AuthSpec, DescribeRequest, Header, HeaderSpec, Info, Json, JsonResponse, MediaType,
    NoContent, OperationCx, OperationMeta, OptionalHeader, Path, Query, Router, WithMedia,
    WithResponse, WithStatus,
};
use pons_ir::{ApiKeyLocation, AuthScheme, CredentialLocator};

pub use model::{
    AccountLocked, CreateUser, Delivery, InvalidCredentials, LoginRequest, Notification,
    RateLimited, Report, RoleFilter, Session, User, UsersGetQuery,
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

struct ProfileScope;

impl AuthSpec for ProfileScope {
    fn apply(cx: &mut OperationCx) {
        cx.require_scopes("session", ["profile:read"]);
    }
}

struct OptionalSession;

impl AuthSpec for OptionalSession {
    fn apply(cx: &mut OperationCx) {
        cx.require_all(["session"]);
        cx.optional_auth();
    }
}

struct SessionOrApiKey;

impl AuthSpec for SessionOrApiKey {
    fn apply(cx: &mut OperationCx) {
        cx.require_any(["session"]);
        cx.require_any(["api_key"]);
    }
}

struct Trace;

impl HeaderSpec for Trace {
    type Value = String;
    const NAME: &'static str = "X-Trace";
}

struct Tenant;

impl HeaderSpec for Tenant {
    type Value = String;
    const NAME: &'static str = "X-Tenant";
}

async fn login(
    Json(request): Json<LoginRequest>,
) -> Result<axum::Json<Session>, InvalidCredentials> {
    let _ = request.password;
    Ok(axum::Json(Session {
        token: "token".into(),
        expires_at: "2026-01-01T00:00:00Z".into(),
        refresh_token: None,
    }))
}

async fn get_user(
    _session: Auth<SessionGuard, ProfileScope>,
    Path(id): Path<i64>,
    Query(query): Query<UsersGetQuery>,
) -> axum::Json<User> {
    let _ = (query.active, query.role);
    axum::Json(User {
        id,
        name: "Ada".into(),
        nickname: None,
    })
}

async fn create_user(
    _session: Auth<SessionGuard, OptionalSession>,
    Header(_trace): Header<Trace>,
    OptionalHeader(_tenant): OptionalHeader<Tenant>,
    Json(request): Json<CreateUser>,
) -> Result<WithStatus<201, axum::Json<User>>, RateLimited> {
    Ok(WithStatus::<201, _>::new(axum::Json(User {
        id: 1,
        name: request.name,
        nickname: None,
    })))
}

async fn delete_user(Path(_id): Path<i64>) -> NoContent {
    NoContent
}

async fn latest_notification(
    _session: Auth<SessionGuard, SessionOrApiKey>,
) -> axum::Json<Notification> {
    axum::Json(Notification::Follow { user_id: 1 })
}

async fn latest_delivery() -> axum::Json<Delivery> {
    axum::Json(Delivery::Push {
        device: "phone".into(),
    })
}

struct VendorReport;

impl MediaType for VendorReport {
    const VALUE: &'static str = "application/vnd.acme.report.v1+json";
}

async fn vendor_report() -> WithMedia<Report, JsonResponse<200>, VendorReport> {
    WithResponse::new(
        axum::Json(Report {
            id: 1,
            title: "vendor".into(),
        })
        .into_response(),
    )
    .media::<VendorReport>()
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
    .auth_scheme(
        "api_key",
        AuthScheme::ApiKey {
            location: ApiKeyLocation::Header,
            name: "X-API-Key".into(),
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
                )
                .expires_at(CredentialLocator::Body {
                    pointer: "/expires_at".into(),
                }),
        ),
    )
    .route(
        "/users",
        post_with(
            create_user,
            OperationMeta::new("users.create").summary("Create a user."),
        ),
    )
    .route(
        "/users/{id}",
        get_with(
            get_user,
            OperationMeta::new("users.get").summary("Read a user."),
        )
        .delete(delete_user)
        .with_meta(OperationMeta::new("users.delete").summary("Delete a user.")),
    )
    .route(
        "/notifications/latest",
        get_with(
            latest_notification,
            OperationMeta::new("notifications.latest").summary("Read the latest notification."),
        ),
    )
    .route(
        "/notifications/delivery",
        get_with(
            latest_delivery,
            OperationMeta::new("notifications.delivery").summary("Read the latest delivery."),
        ),
    )
    .route(
        "/reports/vendor",
        get_with(
            vendor_report,
            OperationMeta::new("reports.vendor").summary("Read the vendor report."),
        ),
    )
}
