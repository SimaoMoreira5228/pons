use axum::body::{Body, Bytes};
use axum::extract::{FromRequest, FromRequestParts, Request};
use axum::http::Method;
use pons_core::Schema;
use pons_ir::ContentKind;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::ViolationReport;
use crate::describe::{DescribeRequest, OperationCx};
use crate::validate::{check, check_pairs, decode_pairs, path_value, request_contract, violated};

pub struct Json<T>(pub T);

impl<T, S> FromRequest<S> for Json<T>
where
    T: Schema + DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ViolationReport;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        let (document, ty) = request_contract::<T>(request.extensions());
        let bytes = read_body(request, state).await?;
        let value: Value = serde_json::from_slice(&bytes)
            .map_err(|error| violated("a JSON document", error.to_string()))?;
        check(&document, &ty, &value)?;
        let value = serde_json::from_value(value)
            .map_err(|error| violated("a value the request type reads", error.to_string()))?;
        Ok(Json(value))
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
        let (document, ty) = request_contract::<T>(request.extensions());
        if matches!(*request.method(), Method::GET | Method::HEAD) {
            let query = request.uri().query().unwrap_or_default().to_owned();
            check_pairs(&document, &ty, decode_pairs(&query))?;
            return Ok(Form(read_form(request, state).await?));
        }
        let method = request.method().clone();
        let headers = request.headers().clone();
        let bytes = read_body(request, state).await?;
        check_pairs(
            &document,
            &ty,
            decode_pairs(&String::from_utf8_lossy(&bytes)),
        )?;
        let mut body = Request::new(Body::from(bytes));
        *body.method_mut() = method;
        *body.headers_mut() = headers;
        Ok(Form(read_form(body, state).await?))
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
        let (document, ty) = request_contract::<T>(&parts.extensions);
        check_pairs(
            &document,
            &ty,
            decode_pairs(parts.uri.query().unwrap_or_default()),
        )?;
        let value = axum::extract::Query::try_from_uri(&parts.uri)
            .map_err(|error| violated("a query the request type reads", error.to_string()))?;
        Ok(Query(value.0))
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
        let (document, ty) = request_contract::<T>(&parts.extensions);
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

async fn read_form<T: DeserializeOwned, S: Send + Sync>(
    request: Request,
    state: &S,
) -> Result<T, ViolationReport> {
    axum::extract::Form::<T>::from_request(request, state)
        .await
        .map(|form| form.0)
        .map_err(|error| violated("a form the request type reads", error.to_string()))
}
