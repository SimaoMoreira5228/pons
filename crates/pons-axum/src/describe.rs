use std::str::FromStr;

use axum::response::IntoResponse;
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

pub trait StatusResponse: DescribeResponse + axum::response::IntoResponse {}

#[derive(Debug)]
pub enum HeaderRejection {
    Missing,
    Repeated,
    Invalid,
    Contract(crate::ViolationReport),
}

pub trait HeaderSpec {
    type Value: Schema + std::str::FromStr;
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

#[derive(Clone)]
pub(crate) struct InvalidResponseHeader;

impl<S: HeaderSpec, R> WithOptionalHeader<S, R> {
    pub fn new(body: R, value: Option<S::Value>) -> Self {
        Self(body, value)
    }
}

pub(crate) fn insert_response_header<S>(
    response: &mut axum::response::Response,
    value: &S::Value,
) -> Result<(), &'static str>
where
    S: HeaderSpec,
    S::Value: ToString,
{
    let name = axum::http::HeaderName::from_bytes(S::NAME.as_bytes())
        .map_err(|_| "invalid response header name")?;
    let value = axum::http::HeaderValue::from_bytes(value.to_string().as_bytes())
        .map_err(|_| "invalid response header value")?;
    response.headers_mut().insert(name, value);
    Ok(())
}

pub(crate) fn invalid_response_header() -> axum::response::Response {
    let mut response = (
        axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        "invalid response header value",
    )
        .into_response();
    response.extensions_mut().insert(InvalidResponseHeader);
    response
}

pub(crate) fn is_invalid_response_header(response: &axum::response::Response) -> bool {
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
                codec: "axum.header".into(),
                wire: OpaqueWire::Text,
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

impl<S, R> axum::response::IntoResponse for WithHeader<S, R>
where
    S: HeaderSpec,
    S::Value: ToString,
    R: StatusResponse,
{
    fn into_response(self) -> axum::response::Response {
        let mut response = self.0.into_response();
        if insert_response_header::<S>(&mut response, &self.1).is_err() {
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

impl<S, R> axum::response::IntoResponse for WithOptionalHeader<S, R>
where
    S: HeaderSpec,
    S::Value: ToString,
    R: StatusResponse,
{
    fn into_response(self) -> axum::response::Response {
        let mut response = self.0.into_response();
        if let Some(value) = self.1
            && insert_response_header::<S>(&mut response, &value).is_err()
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

impl axum::response::IntoResponse for HeaderRejection {
    fn into_response(self) -> axum::response::Response {
        let (expected, found) = match self {
            Self::Missing => ("a request carrying the required header", "no header"),
            Self::Repeated => ("a header spelled once", "a repeated header"),
            Self::Invalid => ("a header value the type reads", "a value it cannot read"),
            Self::Contract(report) => return report.into_response(),
        };
        crate::validate::violated(expected, found.to_owned()).into_response()
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
    headers: &axum::http::HeaderMap,
    name: &str,
) -> Result<Option<String>, HeaderRejection> {
    if !valid_header_name(name) {
        return Err(HeaderRejection::Invalid);
    }
    let mut values = headers.get_all(name).iter();
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
    parts: &axum::http::request::Parts,
) -> Result<Option<<T as HeaderSpec>::Value>, HeaderRejection> {
    let Some(text) = header_text(&parts.headers, T::NAME)? else {
        return Ok(None);
    };
    let (document, _) = crate::validate::request_contract::<T::Value>(&parts.extensions, "input")
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

impl<S, T> axum::extract::FromRequestParts<S> for Header<T>
where
    S: Send + Sync,
    T: HeaderSpec,
{
    type Rejection = HeaderRejection;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _: &S,
    ) -> Result<Self, Self::Rejection> {
        header_value::<T>(parts)?
            .map(Self)
            .ok_or(HeaderRejection::Missing)
    }
}

impl<S, T> axum::extract::FromRequestParts<S> for OptionalHeader<T>
where
    S: Send + Sync,
    T: HeaderSpec,
{
    type Rejection = HeaderRejection;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _: &S,
    ) -> Result<Self, Self::Rejection> {
        Ok(Self(header_value::<T>(parts)?))
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

impl<const STATUS: u16, R> axum::response::IntoResponse for WithStatus<STATUS, R>
where
    R: StatusResponse,
{
    fn into_response(self) -> axum::response::Response {
        let status = valid_status(STATUS, "WithStatus");
        let mut response = self.0.into_response();
        if !is_invalid_response_header(&response) {
            *response.status_mut() = axum::http::StatusCode::from_u16(status)
                .expect("WithStatus requires a valid HTTP status");
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

pub struct NoContent;

impl axum::response::IntoResponse for NoContent {
    fn into_response(self) -> axum::response::Response {
        axum::http::StatusCode::NO_CONTENT.into_response()
    }
}

impl DescribeResponse for NoContent {
    fn describe(cx: &mut OperationCx) {
        cx.empty_response(204);
    }
}

impl StatusResponse for NoContent {}

impl DescribeRequest for axum::extract::Multipart {
    fn describe(cx: &mut OperationCx) {
        cx.request_body::<OpaqueMultipart>(ContentKind::Multipart);
        cx.diagnostic(
            "PONS-W101",
            "multipart fields are opaque; wrap them with a typed request description",
        );
    }
}

impl DescribeRequest for axum::extract::RawForm {
    fn describe(cx: &mut OperationCx) {
        cx.request_body::<OpaqueForm>(ContentKind::Form);
        cx.diagnostic(
            "PONS-W101",
            "raw form fields are opaque; use Form<T> for a typed form",
        );
    }
}

impl DescribeRequest for axum::extract::RawQuery {
    fn describe(cx: &mut OperationCx) {
        cx.diagnostic(
            "PONS-E102",
            "raw query strings expose no query contract; use Query<T> for typed fields",
        );
    }
}

impl DescribeRequest for axum::extract::RawPathParams {
    fn describe(cx: &mut OperationCx) {
        cx.diagnostic(
            "PONS-W101",
            "raw path parameters are opaque; use Path<T> for typed fields",
        );
    }
}

impl DescribeRequest for axum::extract::Request {
    fn describe(cx: &mut OperationCx) {
        cx.diagnostic(
            "PONS-E102",
            "raw request extractors expose no request contract; use typed extractors",
        );
    }
}

impl<T> DescribeRequest for axum::extract::State<T> {
    fn describe(_: &mut OperationCx) {}
}

impl<T> DescribeRequest for axum::extract::Extension<T> {
    fn describe(_: &mut OperationCx) {}
}

impl DescribeRequest for axum::http::HeaderMap {
    fn describe(cx: &mut OperationCx) {
        cx.diagnostic(
            "PONS-E102",
            "raw HeaderMap extractors declare no headers the contract can carry; use typed header wrappers",
        );
    }
}

impl DescribeRequest for String {
    fn describe(cx: &mut OperationCx) {
        cx.optional_request_body::<String>(ContentKind::Text);
    }
}

impl DescribeRequest for axum::body::Bytes {
    fn describe(cx: &mut OperationCx) {
        cx.optional_request_body::<ByteArray>(ContentKind::Bytes);
    }
}

impl DescribeResponse for axum::response::Response {
    fn describe(cx: &mut OperationCx) {
        cx.diagnostic(
            "PONS-E102",
            "raw Axum responses expose neither status nor body; use a typed response wrapper",
        );
    }
}

impl<R> DescribeResponse for (axum::http::StatusCode, R)
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
            "a (StatusCode, R) response publishes the unknown status, a catch-all the status the handler writes is never checked against; wrap the body in WithResponse to publish a status the framework writes itself",
        );
    }
}

impl DescribeResponse for () {
    fn describe(cx: &mut OperationCx) {
        cx.empty_response(200);
    }
}

impl<T: Schema> DescribeResponse for axum::Json<T> {
    fn describe(cx: &mut OperationCx) {
        cx.response::<T>(200, ContentKind::Json);
    }
}

impl DescribeResponse for String {
    fn describe(cx: &mut OperationCx) {
        cx.response::<String>(200, ContentKind::Text);
    }
}

impl<R, E> DescribeResponse for Result<R, E>
where
    R: DescribeResponse,
    E: axum::response::IntoResponse + ErrorSpec,
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

impl<T> StatusResponse for axum::Json<T> where T: Schema + serde::Serialize {}

impl StatusResponse for String {}

impl StatusResponse for () {}

impl StatusResponse for axum::body::Bytes {}

impl<T, S> StatusResponse for crate::Sse<T, S>
where
    T: crate::SseEvent + Send + 'static,
    S: futures_util::Stream<Item = T> + Send + 'static,
{
}

impl DescribeResponse for axum::body::Bytes {
    fn describe(cx: &mut OperationCx) {
        cx.response::<ByteArray>(200, ContentKind::Bytes);
    }
}

impl DescribeResponse for ByteArray {
    fn describe(cx: &mut OperationCx) {
        cx.response::<Self>(200, ContentKind::Bytes);
    }
}

impl<T: crate::SseEvent, S> DescribeResponse for crate::Sse<T, S> {
    fn describe(cx: &mut OperationCx) {
        cx.sse::<T>(T::NAME);
    }
}

pub struct ByteArray;

pub struct OpaqueForm;

pub struct OpaqueMultipart;

fn opaque(codec: &str) -> TypeRef {
    TypeRef::Inline(InlineType::Opaque {
        codec: codec.into(),
        wire: OpaqueWire::Unknown,
    })
}

impl Schema for OpaqueForm {
    fn schema(_: &mut TypeCx, _: Direction) -> TypeRef {
        opaque("axum.form")
    }
}

impl Schema for OpaqueMultipart {
    fn schema(_: &mut TypeCx, _: Direction) -> TypeRef {
        opaque("axum.multipart")
    }
}

impl Schema for ByteArray {
    fn schema(_: &mut TypeCx, _: Direction) -> TypeRef {
        TypeRef::Inline(InlineType::Bytes {
            encoding: pons_ir::ByteEncoding::Base64,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::response::IntoResponse;
    use pons_ir::DiagnosticSeverity;

    struct Retry;

    impl HeaderSpec for Retry {
        type Value = u16;
        const NAME: &'static str = "X-Retry";
    }

    struct Offset;

    impl HeaderSpec for Offset {
        type Value = u128;
        const NAME: &'static str = "X-Offset";
    }

    struct Debug;

    impl HeaderSpec for Debug {
        type Value = bool;
        const NAME: &'static str = "X-Debug";
    }

    struct Retries;

    impl Schema for Retries {
        fn schema(_: &mut TypeCx, _: Direction) -> TypeRef {
            TypeRef::Inline(InlineType::Int {
                bits: 16,
                signed: false,
                wire: IntegerWire::Number,
                minimum: Some(serde_json::json!(1)),
                maximum: Some(serde_json::json!(5)),
                multiple_of: Some(serde_json::json!(1)),
            })
        }
    }

    struct Tags;

    impl Schema for Tags {
        fn schema(cx: &mut TypeCx, direction: Direction) -> TypeRef {
            <Vec<String> as Schema>::schema(cx, direction)
        }
    }

    impl std::str::FromStr for Tags {
        type Err = std::convert::Infallible;

        fn from_str(_text: &str) -> Result<Self, Self::Err> {
            Ok(Self)
        }
    }

    async fn header_handler(Header(retry): Header<Retry>) -> WithHeader<Offset, String> {
        WithHeader::new(retry.to_string(), u128::from(u32::MAX))
    }

    #[allow(dead_code)]
    #[derive(serde::Serialize, pons_core::Schema, pons_core::ApiError)]
    enum Conflict {
        #[pons(status = 409)]
        Conflict,
    }

    fn info() -> pons_ir::Info {
        pons_ir::Info {
            title: "Headers".into(),
            version: "1".into(),
            description: None,
            servers: Vec::new(),
            ext: Default::default(),
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
        assert_eq!(
            header_schema::<u128>(&mut TypeCx::new(), Direction::Output, Offset::NAME),
            TypeRef::Inline(InlineType::Int {
                bits: 128,
                signed: false,
                wire: IntegerWire::String,
                minimum: None,
                maximum: None,
                multiple_of: None,
            })
        );
        assert_eq!(
            header_schema::<i8>(&mut TypeCx::new(), Direction::Output, "X-Delta"),
            TypeRef::Inline(InlineType::Int {
                bits: 8,
                signed: true,
                wire: IntegerWire::String,
                minimum: None,
                maximum: None,
                multiple_of: None,
            })
        );
        assert_eq!(
            header_schema::<Retries>(&mut TypeCx::new(), Direction::Output, "X-Retries"),
            TypeRef::Inline(InlineType::Int {
                bits: 16,
                signed: false,
                wire: IntegerWire::String,
                minimum: Some(serde_json::json!(1)),
                maximum: Some(serde_json::json!(5)),
                multiple_of: Some(serde_json::json!(1)),
            })
        );
    }

    #[test]
    fn a_boolean_header_value_is_the_true_false_text_token() {
        assert_eq!(
            header_schema::<bool>(&mut TypeCx::new(), Direction::Input, Debug::NAME),
            TypeRef::Inline(InlineType::Bool)
        );
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(Debug::NAME, axum::http::HeaderValue::from_static("true"));
        assert_eq!(
            header_text(&headers, Debug::NAME).unwrap(),
            Some("true".to_owned())
        );
    }

    #[test]
    fn header_values_that_are_json_documents_are_refused() {
        let mut cx = TypeCx::new();
        assert_eq!(
            header_schema::<Tags>(&mut cx, Direction::Input, "X-Tags"),
            TypeRef::Inline(InlineType::Opaque {
                codec: "axum.header".into(),
                wire: OpaqueWire::Text,
            })
        );
        assert_eq!(cx.diagnostics().len(), 1);
        assert_eq!(cx.diagnostics()[0].code, "PONS-E102");
        assert_eq!(cx.diagnostics()[0].severity, DiagnosticSeverity::Error);
        assert!(cx.diagnostics()[0].message.contains("X-Tags"));
    }

    #[test]
    fn a_router_describes_every_typed_header_as_a_text_token() {
        let document = crate::Router::<()>::new(info())
            .route("/headers", crate::routing::get(header_handler))
            .into_document()
            .unwrap();
        let operation = &document.operations[0];
        assert_eq!(operation.headers[0].wire, "X-Retry");
        assert_eq!(
            operation.headers[0].ty,
            TypeRef::Inline(InlineType::Int {
                bits: 16,
                signed: false,
                wire: IntegerWire::String,
                minimum: None,
                maximum: None,
                multiple_of: None,
            })
        );
        assert_eq!(operation.responses[0].headers[0].wire, "X-Offset");
        assert_eq!(
            operation.responses[0].headers[0].ty,
            TypeRef::Inline(InlineType::Int {
                bits: 128,
                signed: false,
                wire: IntegerWire::String,
                minimum: None,
                maximum: None,
                multiple_of: None,
            })
        );
    }

    #[test]
    fn error_response_headers_are_described_as_text_tokens() {
        let mut cx = TypeCx::new();
        let errors =
            <crate::WithErrorHeader<Retry, Conflict> as pons_core::ErrorSpec>::errors(&mut cx);
        assert_eq!(errors[0].headers[0].wire, "X-Retry");
        assert_eq!(
            errors[0].headers[0].ty,
            TypeRef::Inline(InlineType::Int {
                bits: 16,
                signed: false,
                wire: IntegerWire::String,
                minimum: None,
                maximum: None,
                multiple_of: None,
            })
        );
        assert!(cx.diagnostics().is_empty());
    }

    #[test]
    fn header_text_is_read_once_case_insensitively() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert("x-request-id", axum::http::HeaderValue::from_static("abc"));
        assert_eq!(
            header_text(&headers, "X-Request-ID").unwrap(),
            Some("abc".to_owned())
        );
        assert_eq!(header_text(&headers, "X-Retry").unwrap(), None);
        assert!(matches!(
            header_text(&headers, "X Request ID"),
            Err(HeaderRejection::Invalid)
        ));

        headers.insert("X-Retry", axum::http::HeaderValue::from_static("2"));
        headers.append("X-Retry", axum::http::HeaderValue::from_static("3"));
        assert!(matches!(
            header_text(&headers, "X-Retry"),
            Err(HeaderRejection::Repeated)
        ));
    }

    #[test]
    fn marks_optional_request_bodies() {
        let mut cx = OperationCx::default();
        cx.optional_request_body::<String>(ContentKind::Text);
        assert!(!cx.body.as_ref().unwrap().required);
    }

    #[test]
    #[should_panic(expected = "representable by Pons IR")]
    fn rejects_statuses_outside_the_ir_range() {
        let _ = WithStatus::<600, String>::new("body".to_owned()).into_response();
    }
}
