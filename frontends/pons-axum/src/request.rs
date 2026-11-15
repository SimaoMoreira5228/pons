use axum::body::Bytes;
use axum::extract::{FromRequest, FromRequestParts, Request};
use axum::http::Method;
use pons_core::Schema;
use pons_ir::ContentKind;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::ViolationReport;
use crate::describe::{DescribeRequest, OperationCx};
use crate::matching::declared_media_type;
use crate::urlencoded::decode_pairs;
use crate::validate::{check, read_pairs, request_contract, violated};
use crate::wire_value::path_value;

const JSON_MEDIA_TYPE: &str = "application/json";
const FORM_MEDIA_TYPE: &str = "application/x-www-form-urlencoded";

pub struct Json<T>(pub T);

impl<T, S> FromRequest<S> for Json<T>
where
    T: Schema + DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ViolationReport;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        let (document, ty) = request_contract::<T>(request.extensions(), "input")?;
        let expected = declared_media_type(&document, &request, JSON_MEDIA_TYPE);
        let announced = announced_media_type(&request, &expected)?;
        let bytes = read_body(request, state).await?;
        refuse_unannounced(&expected, announced, &bytes)?;
        let value: Value = serde_json::from_slice(&bytes)
            .map_err(|error| violated("a JSON document", error.to_string()))?;
        check(&document, &ty, &value)?;
        let value = serde_json::from_value(value)
            .map_err(|error| violated("a value the request type reads", error.to_string()))?;
        Ok(Json(value))
    }
}

fn announced_media_type(request: &Request, expected: &str) -> Result<bool, ViolationReport> {
    let Some(found) = request.headers().get(axum::http::header::CONTENT_TYPE) else {
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
    bytes: &Bytes,
) -> Result<(), ViolationReport> {
    match announced || bytes.is_empty() {
        true => Ok(()),
        false => Err(violated(expected, String::new())),
    }
}

impl<T: Schema> DescribeRequest for Json<T> {
    fn describe(cx: &mut OperationCx) {
        cx.request_body::<T>(ContentKind::Json);
        cx.mark_validated();
    }
}

pub struct Form<T>(pub T);

impl<T, S> FromRequest<S> for Form<T>
where
    T: Schema + DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ViolationReport;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        let (document, ty) = request_contract::<T>(request.extensions(), "form")?;
        if matches!(*request.method(), Method::GET | Method::HEAD) {
            return Ok(Form(read_pairs(
                &document,
                &ty,
                decode_pairs(request.uri().query().unwrap_or_default()),
            )?));
        }
        let expected = declared_media_type(&document, &request, FORM_MEDIA_TYPE);
        let announced = announced_media_type(&request, &expected)?;
        let bytes = read_body(request, state).await?;
        refuse_unannounced(&expected, announced, &bytes)?;
        Ok(Form(read_pairs(
            &document,
            &ty,
            decode_pairs(&String::from_utf8_lossy(&bytes)),
        )?))
    }
}

impl<T: Schema> DescribeRequest for Form<T> {
    fn describe(cx: &mut OperationCx) {
        cx.request_body::<T>(ContentKind::Form);
        cx.mark_validated();
    }
}

pub struct Query<T>(pub T);

impl<T, S> FromRequestParts<S> for Query<T>
where
    T: Schema + DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ViolationReport;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _: &S,
    ) -> Result<Self, Self::Rejection> {
        let (document, ty) = request_contract::<T>(&parts.extensions, "query")?;
        Ok(Query(read_pairs(
            &document,
            &ty,
            decode_pairs(parts.uri.query().unwrap_or_default()),
        )?))
    }
}

impl<T: Schema> DescribeRequest for Query<T> {
    fn describe(cx: &mut OperationCx) {
        cx.query_type::<T>();
        cx.mark_validated();
    }
}

pub struct Path<T>(pub T);

impl<T, S> FromRequestParts<S> for Path<T>
where
    T: Schema + DeserializeOwned + Send,
    S: Send + Sync,
{
    type Rejection = ViolationReport;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        state: &S,
    ) -> Result<Self, Self::Rejection> {
        let (document, ty) = request_contract::<T>(&parts.extensions, "path")?;
        let params = axum::extract::RawPathParams::from_request_parts(parts, state)
            .await
            .map_err(|error| violated("path parameters the route declares", error.to_string()))?;
        let params = params
            .iter()
            .map(|(name, text)| (name.to_owned(), text.to_owned()))
            .collect();
        check(&document, &ty, &path_value(&document, &ty, params))?;
        let value = axum::extract::Path::from_request_parts(parts, state)
            .await
            .map_err(|error| violated("path parameters the type reads", error.to_string()))?;
        Ok(Path(value.0))
    }
}

impl<T: Schema> DescribeRequest for Path<T> {
    fn describe(cx: &mut OperationCx) {
        cx.path_type::<T>();
        cx.mark_validated();
    }
}

async fn read_body<S: Send + Sync>(request: Request, state: &S) -> Result<Bytes, ViolationReport> {
    Bytes::from_request(request, state)
        .await
        .map_err(|error| violated("a readable request body", error.to_string()))
}
