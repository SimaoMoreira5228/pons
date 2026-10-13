use std::str::FromStr;

use actix_web::body::{BoxBody, MessageBody};
use actix_web::http::StatusCode;
use actix_web::http::header::{HeaderName, HeaderValue};
use actix_web::{
    FromRequest, HttpRequest, HttpResponse, HttpResponseBuilder, Responder, ResponseError,
};
use pons_core::{ErrorSpec, Schema, TypeCx};
use pons_ir::{
    AuthRequirement, Body, ContentKind, CredentialProvision, Diagnostic, Direction, ErrorResponse,
    HeaderParameter, InlineType, IntegerWire, OpaqueWire, Presence, Response, ResponseHeader,
    ResponseStatus, StreamDescription, TypeRef,
};

#[derive(Debug, Default)]
pub struct OperationCx {
    pub(crate) types: TypeCx,
    pub(crate) path_types: Vec<TypeRef>,
    pub(crate) flat_carriers: Vec<(TypeRef, &'static str)>,
    pub(crate) carriers: Vec<TypeRef>,
    pub(crate) headers: Vec<HeaderParameter>,
    pub(crate) query: Option<TypeRef>,
    pub(crate) body: Option<Body>,
    pub(crate) responses: Vec<Response>,
    pub(crate) errors: Vec<ErrorResponse>,
    pub(crate) stream: Option<StreamDescription>,
    pub(crate) auth: AuthRequirement,
    pub(crate) provides: Option<CredentialProvision>,
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

pub trait StatusResponse: DescribeResponse + Responder {}

#[derive(Debug)]
pub enum HeaderRejection {
    Missing,
    Repeated,
    Invalid,
    Contract(crate::ViolationReport),
}

pub trait HeaderSpec {
    type Value: Schema + FromStr;
    const NAME: &'static str;
}

pub struct Header<S: HeaderSpec>(pub S::Value);

pub struct OptionalHeader<S: HeaderSpec>(pub Option<S::Value>);

pub struct WithHeader<S: HeaderSpec, R>(pub R, pub S::Value);

impl<S: HeaderSpec, R> WithHeader<S, R> {
    pub fn new(body: R, value: S::Value) -> Self {
        Self(body, value)
    }
}

pub struct WithOptionalHeader<S: HeaderSpec, R>(pub R, pub Option<S::Value>);

impl<S: HeaderSpec, R> WithOptionalHeader<S, R> {
    pub fn new(body: R, value: Option<S::Value>) -> Self {
        Self(body, value)
    }
}

#[derive(Clone)]
pub(crate) struct InvalidResponseHeader;

pub(crate) fn insert_response_header<B, S>(
    response: &mut HttpResponse<B>,
    value: &S::Value,
) -> Result<(), &'static str>
where
    B: MessageBody,
    S: HeaderSpec,
    S::Value: ToString,
{
    let name =
        HeaderName::from_bytes(S::NAME.as_bytes()).map_err(|_| "invalid response header name")?;
    let value = HeaderValue::from_bytes(value.to_string().as_bytes())
        .map_err(|_| "invalid response header value")?;
    response.headers_mut().insert(name, value);
    Ok(())
}

pub(crate) fn invalid_response_header() -> HttpResponse<BoxBody> {
    let mut response = HttpResponse::InternalServerError()
        .content_type("text/plain; charset=utf-8")
        .body("invalid response header value");
    response.extensions_mut().insert(InvalidResponseHeader);
    response
}

pub(crate) fn is_invalid_response_header<B>(response: &HttpResponse<B>) -> bool
where
    B: MessageBody,
{
    response
        .extensions()
        .get::<InvalidResponseHeader>()
        .is_some()
}

pub(crate) fn valid_status(status: u16, wrapper: &str) -> u16 {
    assert!(
        (100..=599).contains(&status),
        "{wrapper} requires an HTTP status representable by Pons IR"
    );
    status
}

pub(crate) fn header_schema<T: Schema>(
    cx: &mut TypeCx,
    direction: Direction,
    name: &str,
) -> TypeRef {
    let ty = T::schema(cx, direction);
    match ty {
        TypeRef::Inline(InlineType::Int {
            bits,
            signed,
            minimum,
            maximum,
            multiple_of,
            ..
        }) => TypeRef::Inline(InlineType::Int {
            bits,
            signed,
            wire: IntegerWire::String,
            minimum,
            maximum,
            multiple_of,
        }),
        TypeRef::Inline(
            InlineType::Array { .. } | InlineType::Map { .. } | InlineType::Tuple { .. },
        ) => {
            cx.push_error(
                "PONS-E102",
                format!(
                    "header {name} carries a JSON value; a header value is a text token, so it is described as opaque text"
                ),
            );
            TypeRef::Inline(InlineType::Opaque {
                codec: "actix.header".into(),
                wire: OpaqueWire::Text,
                ext: Default::default(),
            })
        }
        ty => ty,
    }
}

fn describe_response_header<S: HeaderSpec>(cx: &mut OperationCx, start: usize, presence: Presence) {
    let header = ResponseHeader {
        wire: S::NAME.to_owned(),
        ty: header_schema::<S::Value>(&mut cx.types, Direction::Output, S::NAME),
        presence,
        ext: Default::default(),
    };
    cx.responses[start..]
        .iter_mut()
        .for_each(|response| response.headers.push(header.clone()));
}

impl<S, R> Responder for WithHeader<S, R>
where
    S: HeaderSpec,
    S::Value: ToString,
    R: Responder,
{
    type Body = BoxBody;

    fn respond_to(self, req: &HttpRequest) -> HttpResponse<Self::Body> {
        let mut response = self.0.respond_to(req).map_into_boxed_body();
        if insert_response_header::<BoxBody, S>(&mut response, &self.1).is_err() {
            return invalid_response_header();
        }
        response
    }
}

impl<S, R> DescribeResponse for WithHeader<S, R>
where
    S: HeaderSpec,
    R: DescribeResponse,
{
    fn describe(cx: &mut OperationCx) {
        let start = cx.responses.len();
        R::describe(cx);
        describe_response_header::<S>(cx, start, Presence::Required);
    }
}

impl<S, R> StatusResponse for WithHeader<S, R>
where
    S: HeaderSpec,
    S::Value: ToString,
    R: StatusResponse,
{
}

impl<S, R> Responder for WithOptionalHeader<S, R>
where
    S: HeaderSpec,
    S::Value: ToString,
    R: Responder,
{
    type Body = BoxBody;

    fn respond_to(self, req: &HttpRequest) -> HttpResponse<Self::Body> {
        let mut response = self.0.respond_to(req).map_into_boxed_body();
        if let Some(value) = self.1
            && insert_response_header::<BoxBody, S>(&mut response, &value).is_err()
        {
            return invalid_response_header();
        }
        response
    }
}

impl<S, R> DescribeResponse for WithOptionalHeader<S, R>
where
    S: HeaderSpec,
    R: DescribeResponse,
{
    fn describe(cx: &mut OperationCx) {
        let start = cx.responses.len();
        R::describe(cx);
        describe_response_header::<S>(cx, start, Presence::Optional);
    }
}

impl<S, R> StatusResponse for WithOptionalHeader<S, R>
where
    S: HeaderSpec,
    S::Value: ToString,
    R: StatusResponse,
{
}

impl std::fmt::Display for HeaderRejection {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("the request header does not satisfy the contract")
    }
}

impl ResponseError for HeaderRejection {
    fn status_code(&self) -> StatusCode {
        StatusCode::UNPROCESSABLE_ENTITY
    }

    fn error_response(&self) -> HttpResponse<BoxBody> {
        match self {
            Self::Contract(report) => report.error_response(),
            Self::Missing => crate::validate::violated(
                "a request carrying the required header",
                "no header".to_owned(),
            )
            .error_response(),
            Self::Repeated => {
                crate::validate::violated("a header spelled once", "a repeated header".to_owned())
                    .error_response()
            }
            Self::Invalid => crate::validate::violated(
                "a header value the type reads",
                "a value it cannot read".to_owned(),
            )
            .error_response(),
        }
    }
}

fn valid_header_name(name: &str) -> bool {
    !name.is_empty()
        && name.as_bytes().iter().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(
                    byte,
                    b'!' | b'#'
                        | b'$'
                        | b'%'
                        | b'&'
                        | b'\''
                        | b'*'
                        | b'+'
                        | b'-'
                        | b'.'
                        | b'^'
                        | b'_'
                        | b'`'
                        | b'|'
                        | b'~'
                )
        })
}

fn header_text(
    headers: &actix_web::http::header::HeaderMap,
    name: &str,
) -> Result<Option<String>, HeaderRejection> {
    if !valid_header_name(name) {
        return Err(HeaderRejection::Invalid);
    }
    let mut values = headers.get_all(name);
    let Some(value) = values.next() else {
        return Ok(None);
    };
    if values.next().is_some() {
        return Err(HeaderRejection::Repeated);
    }
    value
        .to_str()
        .map(|text| Some(text.to_owned()))
        .map_err(|_| HeaderRejection::Invalid)
}

fn header_value<T: HeaderSpec>(
    request: &HttpRequest,
) -> Result<Option<<T as HeaderSpec>::Value>, HeaderRejection> {
    let Some(text) = header_text(request.headers(), T::NAME)? else {
        return Ok(None);
    };
    let (document, _) = crate::validate::request_contract::<T::Value>(request, "input")
        .map_err(HeaderRejection::Contract)?;
    let ty = header_schema::<T::Value>(&mut TypeCx::new(), Direction::Input, T::NAME);
    let value = crate::validate::token_value(&document, &ty, &text);
    if let Err(report) = crate::validate::check(&document, &ty, &value) {
        return Err(HeaderRejection::Contract(report));
    }
    Ok(Some(
        <T as HeaderSpec>::Value::from_str(&text).map_err(|_| HeaderRejection::Invalid)?,
    ))
}

impl<S> FromRequest for Header<S>
where
    S: HeaderSpec,
{
    type Error = HeaderRejection;
    type Future = std::future::Ready<Result<Self, Self::Error>>;

    fn from_request(request: &HttpRequest, _: &mut actix_web::dev::Payload) -> Self::Future {
        std::future::ready(
            header_value::<S>(request)
                .map(|value| value.map(Self))
                .and_then(|value| value.ok_or(HeaderRejection::Missing)),
        )
    }
}

impl<S> FromRequest for OptionalHeader<S>
where
    S: HeaderSpec,
{
    type Error = HeaderRejection;
    type Future = std::future::Ready<Result<Self, Self::Error>>;

    fn from_request(request: &HttpRequest, _: &mut actix_web::dev::Payload) -> Self::Future {
        std::future::ready(header_value::<S>(request).map(Self))
    }
}

impl<T: HeaderSpec> DescribeRequest for Header<T> {
    fn describe(cx: &mut OperationCx) {
        cx.request_header::<T::Value>(T::NAME, Presence::Required);
        cx.mark_validated();
    }
}

impl<T: HeaderSpec> DescribeRequest for OptionalHeader<T> {
    fn describe(cx: &mut OperationCx) {
        cx.request_header::<T::Value>(T::NAME, Presence::Optional);
        cx.mark_validated();
    }
}

pub struct WithStatus<const STATUS: u16, R>(pub R);

impl<const STATUS: u16, R> WithStatus<STATUS, R> {
    pub fn new(body: R) -> Self {
        Self(body)
    }
}

impl<const STATUS: u16, R> Responder for WithStatus<STATUS, R>
where
    R: Responder,
{
    type Body = R::Body;

    fn respond_to(self, req: &HttpRequest) -> HttpResponse<Self::Body> {
        let status = valid_status(STATUS, "WithStatus");
        let mut response = self.0.respond_to(req);
        if !is_invalid_response_header(&response) {
            *response.status_mut() =
                StatusCode::from_u16(status).expect("WithStatus requires a valid HTTP status");
        }
        response
    }
}

impl<const STATUS: u16, R> DescribeResponse for WithStatus<STATUS, R>
where
    R: StatusResponse,
{
    fn describe(cx: &mut OperationCx) {
        let status = valid_status(STATUS, "WithStatus");
        let first_response = cx.responses.len();
        R::describe(cx);
        for response in &mut cx.responses[first_response..] {
            response.status = ResponseStatus::Exact(status);
        }
    }
}

impl<const STATUS: u16, R> StatusResponse for WithStatus<STATUS, R> where R: StatusResponse {}

pub struct NoContent;

impl Responder for NoContent {
    type Body = BoxBody;

    fn respond_to(self, _: &HttpRequest) -> HttpResponse<BoxBody> {
        HttpResponse::NoContent().finish()
    }
}

impl DescribeResponse for NoContent {
    fn describe(cx: &mut OperationCx) {
        cx.empty_response(204);
    }
}

impl StatusResponse for NoContent {}

impl DescribeRequest for HttpRequest {
    fn describe(cx: &mut OperationCx) {
        cx.diagnostic(
            "PONS-E102",
            "raw HttpRequest extractors expose no request contract; use typed extractors",
        );
    }
}

impl DescribeRequest for actix_web::http::header::HeaderMap {
    fn describe(cx: &mut OperationCx) {
        cx.diagnostic(
            "PONS-E102",
            "raw HeaderMap extractors expose no named headers; use Header or OptionalHeader",
        );
    }
}

impl DescribeRequest for actix_web::web::Payload {
    fn describe(cx: &mut OperationCx) {
        cx.diagnostic(
            "PONS-E102",
            "raw payload extractors expose no body contract; use Json, Form, or a typed wrapper",
        );
    }
}

impl<T> DescribeRequest for actix_web::web::Data<T> {
    fn describe(_: &mut OperationCx) {}
}

impl<T> DescribeRequest for actix_web::web::Json<T> {
    fn describe(cx: &mut OperationCx) {
        cx.diagnostic(
            "PONS-E102",
            "raw actix Json extractors bypass contract validation; use pons_actix::Json",
        );
    }
}

impl<T> DescribeRequest for actix_web::web::Path<T> {
    fn describe(cx: &mut OperationCx) {
        cx.diagnostic(
            "PONS-E102",
            "raw actix Path extractors bypass contract validation; use pons_actix::Path",
        );
    }
}

impl<T> DescribeRequest for actix_web::web::Query<T> {
    fn describe(cx: &mut OperationCx) {
        cx.diagnostic(
            "PONS-E102",
            "raw actix Query extractors bypass contract validation; use pons_actix::Query",
        );
    }
}

impl<T> DescribeRequest for actix_web::web::Form<T> {
    fn describe(cx: &mut OperationCx) {
        cx.diagnostic(
            "PONS-E102",
            "raw actix Form extractors bypass contract validation; use pons_actix::Form",
        );
    }
}

impl DescribeRequest for String {
    fn describe(cx: &mut OperationCx) {
        cx.optional_request_body::<String>(ContentKind::Text);
    }
}

impl DescribeResponse for String {
    fn describe(cx: &mut OperationCx) {
        cx.response::<String>(200, ContentKind::Text);
    }
}

impl StatusResponse for String {}

impl<T: Schema> DescribeResponse for actix_web::web::Json<T> {
    fn describe(cx: &mut OperationCx) {
        cx.response::<T>(200, ContentKind::Json);
    }
}

impl<T> StatusResponse for actix_web::web::Json<T> where T: Schema + serde::Serialize {}

pub struct ByteArray;

impl Schema for ByteArray {
    fn schema(_: &mut TypeCx, _: Direction) -> TypeRef {
        TypeRef::Inline(InlineType::Bytes {
            encoding: pons_ir::ByteEncoding::Base64,
        })
    }
}

impl DescribeResponse for actix_web::web::Bytes {
    fn describe(cx: &mut OperationCx) {
        cx.response::<ByteArray>(200, ContentKind::Bytes);
    }
}

impl StatusResponse for actix_web::web::Bytes {}

impl<B> DescribeResponse for HttpResponse<B>
where
    B: MessageBody + 'static,
{
    fn describe(cx: &mut OperationCx) {
        cx.diagnostic(
            "PONS-E102",
            "raw actix responses expose neither status nor body; use a typed response wrapper",
        );
    }
}

impl DescribeResponse for HttpResponseBuilder {
    fn describe(cx: &mut OperationCx) {
        cx.diagnostic(
            "PONS-E102",
            "raw actix response builders expose neither status nor body; use a typed response wrapper",
        );
    }
}

impl<R> DescribeResponse for (R, StatusCode)
where
    R: DescribeResponse,
{
    fn describe(cx: &mut OperationCx) {
        let start = cx.responses.len();
        R::describe(cx);
        for response in &mut cx.responses[start..] {
            response.status = ResponseStatus::Unknown;
        }
        cx.diagnostic(
            "PONS-W101",
            "an (R, StatusCode) response publishes the unknown status, a catch-all the status the handler writes is never checked against; wrap the body in WithResponse to publish a status the framework writes itself",
        );
    }
}

impl DescribeResponse for () {
    fn describe(cx: &mut OperationCx) {
        cx.empty_response(200);
    }
}

impl<R, E> DescribeResponse for Result<R, E>
where
    R: DescribeResponse,
    E: ErrorSpec,
{
    fn describe(cx: &mut OperationCx) {
        R::describe(cx);
        let errors = E::errors(&mut cx.types);
        if !crate::error::enforces_declared_status::<E>() {
            let statuses = errors
                .iter()
                .map(|error| format!("{:?}", error.status))
                .collect::<Vec<_>>()
                .join(", ");
            cx.diagnostic(
                "PONS-W101",
                format!(
                    "a bare Result declares the error statuses it names ({statuses}) but nothing enforces them at runtime; wrap the error in WithErrorHeader to serve only a status its error type declares"
                ),
            );
        }
        cx.errors.extend(errors);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pons_ir::DiagnosticSeverity;

    struct Retry;

    impl HeaderSpec for Retry {
        type Value = u16;
        const NAME: &'static str = "X-Retry";
    }

    struct Tags;

    impl Schema for Tags {
        fn schema(cx: &mut TypeCx, direction: Direction) -> TypeRef {
            <Vec<String> as Schema>::schema(cx, direction)
        }
    }

    impl FromStr for Tags {
        type Err = std::convert::Infallible;

        fn from_str(_text: &str) -> Result<Self, Self::Err> {
            Ok(Self)
        }
    }

    #[test]
    fn integer_header_values_are_described_as_text_tokens() {
        assert_eq!(
            header_schema::<u16>(&mut TypeCx::new(), Direction::Input, Retry::NAME),
            TypeRef::Inline(InlineType::Int {
                bits: 16,
                signed: false,
                wire: IntegerWire::String,
                minimum: None,
                maximum: None,
                multiple_of: None,
            })
        );
    }

    #[test]
    fn header_values_that_are_json_documents_are_refused() {
        let mut cx = TypeCx::new();
        assert_eq!(
            header_schema::<Tags>(&mut cx, Direction::Input, "X-Tags"),
            TypeRef::Inline(InlineType::Opaque {
                codec: "actix.header".into(),
                wire: OpaqueWire::Text,
                ext: Default::default(),
            })
        );
        assert_eq!(cx.diagnostics().len(), 1);
        assert_eq!(cx.diagnostics()[0].code, "PONS-E102");
        assert_eq!(cx.diagnostics()[0].severity, DiagnosticSeverity::Error);
    }

    #[test]
    #[should_panic(expected = "representable by Pons IR")]
    fn rejects_statuses_outside_the_ir_range() {
        let _ = WithStatus::<600, String>::new("body".to_owned());
        let status = valid_status(600, "WithStatus");
        let _ = actix_web::http::StatusCode::from_u16(status);
    }
}
