use actix_web::body::BoxBody;
use actix_web::http::header::CONTENT_TYPE;
use actix_web::{FromRequest, HttpRequest, HttpResponse, Responder};
use futures_util::future::LocalBoxFuture;
use pons_core::Schema;
use pons_ir::ContentKind;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::ViolationReport;
use crate::describe::{DescribeRequest, DescribeResponse, OperationCx, StatusResponse};
use crate::validate::{
    check, declared_media_type, decode_pairs, path_value, read_pairs, request_contract, violated,
};

const JSON_MEDIA_TYPE: &str = "application/json";
const FORM_MEDIA_TYPE: &str = "application/x-www-form-urlencoded";

pub struct Json<T>(pub T);

impl<T> FromRequest for Json<T>
where
    T: Schema + DeserializeOwned + 'static,
{
    type Error = ViolationReport;
    type Future = LocalBoxFuture<'static, Result<Self, Self::Error>>;

    fn from_request(request: &HttpRequest, payload: &mut actix_web::dev::Payload) -> Self::Future {
        let (document, ty) = match request_contract::<T>(request, "input") {
            Ok(contract) => contract,
            Err(report) => return Box::pin(async move { Err(report) }),
        };
        let expected = declared_media_type(&document, request, JSON_MEDIA_TYPE).to_owned();
        let announced = announced_media_type(request, &expected);
        let body = actix_web::web::Bytes::from_request(request, payload);
        Box::pin(async move {
            let announced = announced?;
            let bytes = body
                .await
                .map_err(|error| violated("a readable request body", error.to_string()))?;
            refuse_unannounced(&expected, announced, &bytes)?;
            let value: Value = serde_json::from_slice(&bytes)
                .map_err(|error| violated("a JSON document", error.to_string()))?;
            check(&document, &ty, &value)?;
            let value = serde_json::from_value(value)
                .map_err(|error| violated("a value the request type reads", error.to_string()))?;
            Ok(Json(value))
        })
    }
}

impl<T: Schema> DescribeRequest for Json<T> {
    fn describe(cx: &mut OperationCx) {
        cx.request_body::<T>(ContentKind::Json);
        cx.mark_validated();
    }
}

impl<T: Schema> DescribeResponse for Json<T> {
    fn describe(cx: &mut OperationCx) {
        cx.response::<T>(200, ContentKind::Json);
    }
}

impl<T> Responder for Json<T>
where
    T: serde::Serialize,
{
    type Body = BoxBody;

    fn respond_to(self, request: &HttpRequest) -> HttpResponse<Self::Body> {
        actix_web::web::Json(self.0)
            .respond_to(request)
            .map_into_boxed_body()
    }
}

impl<T> StatusResponse for Json<T> where T: Schema + serde::Serialize {}

pub struct Form<T>(pub T);

impl<T> FromRequest for Form<T>
where
    T: Schema + DeserializeOwned + 'static,
{
    type Error = ViolationReport;
    type Future = LocalBoxFuture<'static, Result<Self, Self::Error>>;

    fn from_request(request: &HttpRequest, payload: &mut actix_web::dev::Payload) -> Self::Future {
        let (document, ty) = match request_contract::<T>(request, "form") {
            Ok(contract) => contract,
            Err(report) => return Box::pin(async move { Err(report) }),
        };
        if matches!(
            *request.method(),
            actix_web::http::Method::GET | actix_web::http::Method::HEAD
        ) {
            let pairs = decode_pairs(request.query_string());
            return Box::pin(async move { read_pairs(&document, &ty, pairs).map(Form) });
        }
        let expected = declared_media_type(&document, request, FORM_MEDIA_TYPE).to_owned();
        let announced = announced_media_type(request, &expected);
        let body = actix_web::web::Bytes::from_request(request, payload);
        Box::pin(async move {
            let announced = announced?;
            let bytes = body
                .await
                .map_err(|error| violated("a readable request body", error.to_string()))?;
            refuse_unannounced(&expected, announced, &bytes)?;
            let pairs = decode_pairs(&String::from_utf8_lossy(&bytes));
            read_pairs(&document, &ty, pairs).map(Form)
        })
    }
}

impl<T: Schema> DescribeRequest for Form<T> {
    fn describe(cx: &mut OperationCx) {
        cx.request_body::<T>(ContentKind::Form);
        cx.mark_validated();
    }
}

pub struct Query<T>(pub T);

impl<T> FromRequest for Query<T>
where
    T: Schema + DeserializeOwned + 'static,
{
    type Error = ViolationReport;
    type Future = LocalBoxFuture<'static, Result<Self, Self::Error>>;

    fn from_request(request: &HttpRequest, _: &mut actix_web::dev::Payload) -> Self::Future {
        let result = request_contract::<T>(request, "query").and_then(|(document, ty)| {
            read_pairs(&document, &ty, decode_pairs(request.query_string())).map(Query)
        });
        Box::pin(async move { result })
    }
}

impl<T: Schema> DescribeRequest for Query<T> {
    fn describe(cx: &mut OperationCx) {
        cx.query_type::<T>();
        cx.mark_validated();
    }
}

pub struct Path<T>(pub T);

impl<T> FromRequest for Path<T>
where
    T: Schema + DeserializeOwned + 'static,
{
    type Error = ViolationReport;
    type Future = LocalBoxFuture<'static, Result<Self, Self::Error>>;

    fn from_request(request: &HttpRequest, payload: &mut actix_web::dev::Payload) -> Self::Future {
        let (document, ty) = match request_contract::<T>(request, "path") {
            Ok(contract) => contract,
            Err(report) => return Box::pin(async move { Err(report) }),
        };
        let pairs = request
            .match_info()
            .iter()
            .map(|(name, text)| (name.to_owned(), text.to_owned()))
            .collect();
        if let Err(report) = check(&document, &ty, &path_value(&document, &ty, pairs)) {
            return Box::pin(async move { Err(report) });
        }
        let value = actix_web::web::Path::<T>::from_request(request, payload);
        Box::pin(async move {
            value
                .await
                .map(|path| Path(path.into_inner()))
                .map_err(|error| violated("path parameters the type reads", error.to_string()))
        })
    }
}

impl<T: Schema> DescribeRequest for Path<T> {
    fn describe(cx: &mut OperationCx) {
        cx.path_type::<T>();
        cx.mark_validated();
    }
}

fn announced_media_type(request: &HttpRequest, expected: &str) -> Result<bool, ViolationReport> {
    let Some(found) = request.headers().get(CONTENT_TYPE) else {
        return Ok(false);
    };
    let found = found.to_str().unwrap_or_default();
    match pons_ir::media_type_matches(found, expected) {
        true => Ok(true),
        false => Err(violated(expected, found.to_owned())),
    }
}

fn refuse_unannounced(
    expected: &str,
    announced: bool,
    bytes: &[u8],
) -> Result<(), ViolationReport> {
    match announced || bytes.is_empty() {
        true => Ok(()),
        false => Err(violated(expected, String::new())),
    }
}
