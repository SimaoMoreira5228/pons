use http::StatusCode;
use pons_core::{ErrorSpec, Schema};
use pons_ir::{
    AuthRequirement, Body, ContentKind, Diagnostic, ErrorResponse, HeaderParameter, Response,
    ResponseStatus, StreamDescription, TypeRef,
};
use tako_rs_core::responder::Responder;
use tako_rs_core::types::Response as TakoResponse;

#[derive(Debug, Default)]
pub struct OperationCx {
    pub(crate) types: pons_core::TypeCx,
    pub(crate) path_types: Vec<TypeRef>,
    pub(crate) flat_carriers: Vec<(TypeRef, &'static str)>,
    pub(crate) carriers: Vec<TypeRef>,
    pub(crate) headers: Vec<HeaderParameter>,
    pub(crate) query: Option<TypeRef>,
    pub(crate) body: Option<Body>,
    pub(crate) responses: Vec<Response>,
    pub(crate) stream: Option<StreamDescription>,
    pub(crate) errors: Vec<ErrorResponse>,
    pub(crate) auth: AuthRequirement,
    pub(crate) diagnostics: Vec<Diagnostic>,
    pub(crate) validated: bool,
}

impl OperationCx {
    pub fn mark_validated(&mut self) {
        self.validated = true;
    }
}

pub trait DescribeRequest {
    fn describe(cx: &mut OperationCx);
}

pub trait DescribeResponse {
    fn describe(cx: &mut OperationCx);
}

pub struct WithStatus<const STATUS: u16, R>(pub R);

impl<const STATUS: u16, R> WithStatus<STATUS, R> {
    pub fn new(body: R) -> Self {
        Self(body)
    }
}

impl<const STATUS: u16, R: Responder> Responder for WithStatus<STATUS, R> {
    fn into_response(self) -> TakoResponse {
        let status = valid_status(STATUS, "WithStatus");
        let mut response = self.0.into_response();
        *response.status_mut() =
            StatusCode::from_u16(status).expect("WithStatus requires a valid HTTP status");
        response
    }
}

impl<const STATUS: u16, R: DescribeResponse> DescribeResponse for WithStatus<STATUS, R> {
    fn describe(cx: &mut OperationCx) {
        let start = cx.responses.len();
        R::describe(cx);
        for response in &mut cx.responses[start..] {
            response.status = ResponseStatus::Exact(valid_status(STATUS, "WithStatus"));
        }
    }
}

fn valid_status(status: u16, wrapper: &str) -> u16 {
    assert!(
        (100..=599).contains(&status),
        "{wrapper} requires an HTTP status representable by Pons IR"
    );
    status
}

impl<R, E> DescribeResponse for Result<R, E>
where
    R: DescribeResponse,
    E: ErrorSpec,
{
    fn describe(cx: &mut OperationCx) {
        R::describe(cx);
        cx.errors.extend(E::errors(&mut cx.types));
    }
}

impl<T: Schema> DescribeRequest for tako::extractors::json::Json<T> {
    fn describe(cx: &mut OperationCx) {
        cx.request_body::<T>(ContentKind::Json);
    }
}

impl<T: Schema> DescribeRequest for tako::extractors::query::Query<T> {
    fn describe(cx: &mut OperationCx) {
        cx.query_type::<T>();
    }
}

impl<T: Schema> DescribeRequest for tako::extractors::path::Path<T> {
    fn describe(cx: &mut OperationCx) {
        cx.path_type::<T>();
    }
}

impl<T: Schema> DescribeRequest for tako::extractors::params::Params<T> {
    fn describe(cx: &mut OperationCx) {
        cx.path_type::<T>();
    }
}

impl<T: Schema> DescribeRequest for tako::extractors::form::Form<T> {
    fn describe(cx: &mut OperationCx) {
        cx.request_body::<T>(ContentKind::Form);
    }
}

impl<T> DescribeRequest for tako::extractors::state::State<T> {
    fn describe(_: &mut OperationCx) {}
}

impl<T> DescribeRequest for tako::extractors::extension::Extension<T> {
    fn describe(_: &mut OperationCx) {}
}

impl DescribeRequest for tako::types::Request {
    fn describe(_: &mut OperationCx) {}
}

impl DescribeRequest for tako::extractors::header_map::HeaderMap {
    fn describe(cx: &mut OperationCx) {
        cx.diagnostic(
            "PONS-E102",
            "raw HeaderMap extractors expose no named headers; use TypedHeader<H> for a typed header",
        );
    }
}

#[cfg(feature = "typed-header")]
impl<H> DescribeRequest for tako::extractors::typed_header::TypedHeader<H>
where
    H: headers::Header,
{
    fn describe(cx: &mut OperationCx) {
        cx.request_opaque_header(H::name().as_str());
    }
}

impl<T: Schema> DescribeResponse for tako::extractors::json::Json<T> {
    fn describe(cx: &mut OperationCx) {
        cx.response::<T>(200, ContentKind::Json);
    }
}

#[cfg(feature = "sse")]
impl<T: Schema, S> DescribeResponse for crate::PonsSse<T, S> {
    fn describe(cx: &mut OperationCx) {
        cx.sse::<T>("message");
    }
}

impl DescribeResponse for String {
    fn describe(cx: &mut OperationCx) {
        cx.response::<String>(200, ContentKind::Text);
    }
}

impl DescribeResponse for &'static str {
    fn describe(cx: &mut OperationCx) {
        cx.response::<&'static str>(200, ContentKind::Text);
    }
}

impl DescribeResponse for () {
    fn describe(cx: &mut OperationCx) {
        cx.empty_response(200);
    }
}

impl<R: DescribeResponse> DescribeResponse for (StatusCode, R) {
    fn describe(cx: &mut OperationCx) {
        let start = cx.responses.len();
        R::describe(cx);
        for response in &mut cx.responses[start..] {
            response.status = ResponseStatus::Unknown;
        }
        cx.diagnostic(
            "PONS-W101",
            "a (StatusCode, R) response publishes the unknown status, a catch-all the status the handler writes is never checked against; wrap the body in WithStatus<STATUS, R> to publish a status Tako writes itself",
        );
    }
}

impl DescribeResponse for serde_json::Value {
    fn describe(cx: &mut OperationCx) {
        cx.response::<serde_json::Value>(200, ContentKind::Json);
    }
}
