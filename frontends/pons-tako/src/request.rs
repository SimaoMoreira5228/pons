use http::request::Parts;
use pons_core::Schema;
use pons_ir::ContentKind;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tako::body::TakoBody;
use tako::extractors::body::collect_body;
use tako::extractors::form::Form as TakoForm;
use tako::extractors::json::Json as TakoJson;
use tako::extractors::params::PathParams;
use tako::extractors::path::Path as TakoPath;
use tako::extractors::query::Query as TakoQuery;
use tako::extractors::{Entries, FromRequest, FromRequestParts};
use tako::types::Request;

use crate::ViolationReport;
use crate::describe::{DescribeRequest, OperationCx};
use crate::validate::{check, check_pairs, decode_pairs, path_value, request_contract, violated};

pub struct Json<T>(pub T);

impl<'a, T> FromRequest<'a> for Json<T>
where
    T: Schema + DeserializeOwned + Send + 'static,
{
    type Error = ViolationReport;
    const ENTRIES: Entries = Entries::BODY_LIMIT.union(Entries::SIMD_JSON);

    async fn from_request(request: &'a mut Request) -> Result<Self, Self::Error> {
        let (document, ty) = request_contract::<T>(request.extensions(), "input")?;
        let bytes = collect_body(request)
            .await
            .map_err(|error| violated("a readable request body", error.to_string()))?;
        let value: Value = serde_json::from_slice(&bytes)
            .map_err(|error| violated("a JSON document", error.to_string()))?;
        check(&document, &ty, &value)?;
        *request.body_mut() = TakoBody::from(bytes);
        TakoJson::<T>::from_request(request)
            .await
            .map(|TakoJson(value)| Json(value))
            .map_err(|error| violated("a value the request type reads", error.to_string()))
    }
}

impl<T: Schema> DescribeRequest for Json<T> {
    fn describe(cx: &mut OperationCx) {
        cx.request_body::<T>(ContentKind::Json);
        cx.mark_validated();
    }
}

pub struct Form<T>(pub T);

impl<'a, T> FromRequest<'a> for Form<T>
where
    T: Schema + DeserializeOwned + Send + 'static,
{
    type Error = ViolationReport;
    const ENTRIES: Entries = Entries::BODY_LIMIT;

    async fn from_request(request: &'a mut Request) -> Result<Self, Self::Error> {
        let (document, ty) = request_contract::<T>(request.extensions(), "form")?;
        let bytes = collect_body(request)
            .await
            .map_err(|error| violated("a readable request body", error.to_string()))?;
        check_pairs(
            &document,
            &ty,
            decode_pairs(&String::from_utf8_lossy(&bytes)),
        )?;
        *request.body_mut() = TakoBody::from(bytes);
        TakoForm::<T>::from_request(request)
            .await
            .map(|TakoForm(value)| Form(value))
            .map_err(|error| violated("a value the request type reads", error.to_string()))
    }
}

impl<T: Schema> DescribeRequest for Form<T> {
    fn describe(cx: &mut OperationCx) {
        cx.request_body::<T>(ContentKind::Form);
        cx.mark_validated();
    }
}

pub struct Query<T>(pub T);

impl<'a, T> FromRequest<'a> for Query<T>
where
    T: Schema + DeserializeOwned + Send + 'a,
{
    type Error = ViolationReport;
    const ENTRIES: Entries = Entries::NONE;

    async fn from_request(request: &'a mut Request) -> Result<Self, Self::Error> {
        check_query::<T>(request.extensions(), request.uri().query())?;
        TakoQuery::<T>::from_request(request)
            .await
            .map(|TakoQuery(value)| Query(value))
            .map_err(|error| violated("a value the request type reads", error.to_string()))
    }
}

impl<'a, T> FromRequestParts<'a> for Query<T>
where
    T: Schema + DeserializeOwned + Send + 'a,
{
    type Error = ViolationReport;
    const ENTRIES: Entries = Entries::NONE;

    async fn from_request_parts(parts: &'a mut Parts) -> Result<Self, Self::Error> {
        check_query::<T>(&parts.extensions, parts.uri.query())?;
        TakoQuery::<T>::from_request_parts(parts)
            .await
            .map(|TakoQuery(value)| Query(value))
            .map_err(|error| violated("a value the request type reads", error.to_string()))
    }
}

fn check_query<T: Schema>(
    extensions: &http::Extensions,
    query: Option<&str>,
) -> Result<(), ViolationReport> {
    let (document, ty) = request_contract::<T>(extensions, "query")?;
    check_pairs(&document, &ty, decode_pairs(query.unwrap_or_default()))
}

impl<T: Schema> DescribeRequest for Query<T> {
    fn describe(cx: &mut OperationCx) {
        cx.query_type::<T>();
        cx.mark_validated();
    }
}

pub struct Path<T>(pub T);

impl<'a, T> FromRequest<'a> for Path<T>
where
    T: Schema + DeserializeOwned + Send + 'a,
{
    type Error = ViolationReport;
    const ENTRIES: Entries = Entries::PARAMS;

    async fn from_request(request: &'a mut Request) -> Result<Self, Self::Error> {
        check_path::<T>(request.extensions())?;
        TakoPath::<T>::from_request(request)
            .await
            .map(|TakoPath(value)| Path(value))
            .map_err(|error| violated("a value the request type reads", error.to_string()))
    }
}

impl<'a, T> FromRequestParts<'a> for Path<T>
where
    T: Schema + DeserializeOwned + Send + 'a,
{
    type Error = ViolationReport;
    const ENTRIES: Entries = Entries::PARAMS;

    async fn from_request_parts(parts: &'a mut Parts) -> Result<Self, Self::Error> {
        check_path::<T>(&parts.extensions)?;
        TakoPath::<T>::from_request_parts(parts)
            .await
            .map(|TakoPath(value)| Path(value))
            .map_err(|error| violated("a value the request type reads", error.to_string()))
    }
}

fn check_path<T: Schema>(extensions: &http::Extensions) -> Result<(), ViolationReport> {
    let (document, ty) = request_contract::<T>(extensions, "path")?;
    let params = extensions
        .get::<PathParams>()
        .map(|params| {
            params
                .0
                .iter()
                .map(|(name, value)| (name.to_string(), value.clone()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    check(&document, &ty, &path_value(&document, &ty, params))
}

impl<T: Schema> DescribeRequest for Path<T> {
    fn describe(cx: &mut OperationCx) {
        cx.path_type::<T>();
        cx.mark_validated();
    }
}
