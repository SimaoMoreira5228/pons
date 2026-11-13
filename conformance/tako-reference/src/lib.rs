mod error;

use std::collections::BTreeMap;
use std::convert::Infallible;

use pons_ir::{ApiKeyLocation, AuthScheme, CredentialLocator};
use pons_tako::{
    Auth, AuthSpec, DescribeRequest, Header, HeaderSpec, Info, Json, MediaType, OperationCx,
    OperationMeta, OptionalHeader, Path, Query, Router, WithMedia, WithStatus,
};
use reference_api::{
    AccountLocked, CreateUser, Delivery, LoginRequest, Notification, Report, Session, User,
    UsersGetQuery,
};
use tako::extractors::json::Json as TakoJson;
use tako::extractors::{FromRequest, FromRequestParts};

pub use error::{InvalidCredentials, RateLimited};

struct SessionGuard;

impl<'a> FromRequestParts<'a> for SessionGuard {
    type Error = Infallible;

    async fn from_request_parts(_: &'a mut http::request::Parts) -> Result<Self, Self::Error> {
        Ok(Self)
    }
}

impl<'a> FromRequest<'a> for SessionGuard {
    type Error = Infallible;

    async fn from_request(_: &'a mut tako::types::Request) -> Result<Self, Self::Error> {
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
        cx.optional();
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

async fn login(Json(request): Json<LoginRequest>) -> Result<TakoJson<Session>, InvalidCredentials> {
    let _ = request.password;
    Ok(TakoJson(Session {
        token: "token".into(),
        expires_at: "2026-01-01T00:00:00Z".into(),
        refresh_token: None,
    }))
}

async fn get_user(
    _session: Auth<SessionGuard, ProfileScope>,
    Path(id): Path<i64>,
    Query(query): Query<UsersGetQuery>,
) -> TakoJson<User> {
    let _ = (query.active, query.role);
    TakoJson(User {
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
) -> Result<WithStatus<201, TakoJson<User>>, RateLimited> {
    Ok(WithStatus::<201, _>::new(TakoJson(User {
        id: 1,
        name: request.name,
        nickname: None,
    })))
}

async fn delete_user(Path(_id): Path<i64>) -> WithStatus<204, ()> {
    WithStatus::new(())
}

async fn latest_notification(
    _session: Auth<SessionGuard, SessionOrApiKey>,
) -> TakoJson<Notification> {
    TakoJson(Notification::Follow { user_id: 1 })
}

async fn latest_delivery() -> TakoJson<Delivery> {
    TakoJson(Delivery::Push {
        device: "phone".into(),
    })
}

struct VendorReport;

impl MediaType for VendorReport {
    const VALUE: &'static str = "application/vnd.acme.report.v1+json";
}

async fn vendor_report() -> WithMedia<TakoJson<Report>, VendorReport> {
    WithMedia::new(TakoJson(Report {
        id: 1,
        title: "vendor".into(),
    }))
}

pub fn router() -> Router {
    let mut router = Router::new(Info {
        title: "Pons Conformance API".into(),
        version: "1.0.0".into(),
        description: None,
        servers: Vec::new(),
        ext: BTreeMap::new(),
    });
    router
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
        .common_error::<AccountLocked>();
    router.post_with(
        "/auth/login",
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
    );
    router.post_with(
        "/users",
        create_user,
        OperationMeta::new("users.create").summary("Create a user."),
    );
    router.get_with(
        "/users/{id}",
        get_user,
        OperationMeta::new("users.get").summary("Read a user."),
    );
    router.delete_with(
        "/users/{id}",
        delete_user,
        OperationMeta::new("users.delete").summary("Delete a user."),
    );
    router.get_with(
        "/notifications/latest",
        latest_notification,
        OperationMeta::new("notifications.latest").summary("Read the latest notification."),
    );
    router.get_with(
        "/notifications/delivery",
        latest_delivery,
        OperationMeta::new("notifications.delivery").summary("Read the latest delivery."),
    );
    router.get_with(
        "/reports/vendor",
        vendor_report,
        OperationMeta::new("reports.vendor").summary("Read the vendor report."),
    );
    router
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_tako_document_matches_the_axum_document() {
        let axum = reference_api::router().into_document().unwrap();
        let tako = super::router().into_document().unwrap();
        pons_ir::equivalent(&axum, &tako).unwrap_or_else(|differences| {
            panic!(
                "the documents differ:\n{}",
                differences
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("\n")
            )
        });
    }
}
