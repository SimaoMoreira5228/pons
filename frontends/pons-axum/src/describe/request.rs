use std::str::FromStr;

use axum::response::IntoResponse;
use pons_core::{Schema, TypeCx};
use pons_ir::{ContentKind, Direction, InlineType, OpaqueWire, Presence, TypeRef};

use super::header::{HeaderSpec, header_schema};
use super::response::ByteArray;
use super::{DescribeRequest, OperationCx};

#[derive(Debug)]
pub enum HeaderRejection {
    Missing,
    Repeated,
    Invalid,
    Contract(crate::ViolationReport),
}

pub struct Header<S: HeaderSpec>(pub S::Value);

pub struct OptionalHeader<S: HeaderSpec>(pub Option<S::Value>);

impl IntoResponse for HeaderRejection {
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

pub(super) fn header_text(
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

pub struct OpaqueForm;

pub struct OpaqueMultipart;

fn opaque(codec: &str) -> TypeRef {
    TypeRef::Inline(InlineType::Opaque {
        codec: codec.into(),
        wire: OpaqueWire::Unknown,
        ext: Default::default(),
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
