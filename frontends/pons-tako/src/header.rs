use std::str::FromStr;

use http::Extensions;
use http::header::HeaderMap;
use pons_core::Schema;
use pons_ir::Presence;
use tako_rs_core::extractors::{Entries, FromRequest, FromRequestParts};
use tako_rs_core::responder::Responder;
use tako_rs_core::types::{Request, Response};

use crate::ViolationReport;
use crate::describe::{DescribeRequest, OperationCx};
use crate::validate::{check, request_contract, token_value, violated};

pub trait HeaderSpec {
    type Value: Schema + FromStr + Send;
    const NAME: &'static str;
}

pub struct Header<S: HeaderSpec>(pub S::Value);

pub struct OptionalHeader<S: HeaderSpec>(pub Option<S::Value>);

#[derive(Debug)]
pub enum HeaderRejection {
    Missing,
    Repeated,
    Invalid,
    Contract(ViolationReport),
}

impl Responder for HeaderRejection {
    fn into_response(self) -> Response {
        match self {
            Self::Missing => violated("a request carrying the required header", "no header".into())
                .into_response(),
            Self::Repeated => {
                violated("a header spelled once", "a repeated header".into()).into_response()
            }
            Self::Invalid => violated(
                "a header value the type reads",
                "a value it cannot read".into(),
            )
            .into_response(),
            Self::Contract(report) => report.into_response(),
        }
    }
}

fn read<S: HeaderSpec>(
    extensions: &Extensions,
    headers: &HeaderMap,
) -> Result<Option<S::Value>, HeaderRejection> {
    let mut values = headers.get_all(S::NAME).iter();
    let Some(value) = values.next() else {
        return Ok(None);
    };
    if values.next().is_some() {
        return Err(HeaderRejection::Repeated);
    }
    let text = value.to_str().map_err(|_| HeaderRejection::Invalid)?;
    let (document, _) =
        request_contract::<S::Value>(extensions, "input").map_err(HeaderRejection::Contract)?;
    let ty = crate::operation_cx::header_schema::<S::Value>(
        &mut pons_core::TypeCx::new(),
        pons_ir::Direction::Input,
        S::NAME,
    );
    check(&document, &ty, &token_value(&document, &ty, text)).map_err(HeaderRejection::Contract)?;
    S::Value::from_str(text)
        .map(Some)
        .map_err(|_| HeaderRejection::Invalid)
}

impl<'a, S: HeaderSpec + 'a> FromRequestParts<'a> for Header<S> {
    type Error = HeaderRejection;

    async fn from_request_parts(parts: &'a mut http::request::Parts) -> Result<Self, Self::Error> {
        read::<S>(&parts.extensions, &parts.headers)?
            .map(Self)
            .ok_or(HeaderRejection::Missing)
    }
}

impl<'a, S: HeaderSpec + 'a> FromRequestParts<'a> for OptionalHeader<S> {
    type Error = HeaderRejection;

    async fn from_request_parts(parts: &'a mut http::request::Parts) -> Result<Self, Self::Error> {
        Ok(Self(read::<S>(&parts.extensions, &parts.headers)?))
    }
}

impl<'a, S: HeaderSpec + 'a> FromRequest<'a> for Header<S> {
    type Error = HeaderRejection;
    const ENTRIES: Entries = Entries::NONE;

    async fn from_request(request: &'a mut Request) -> Result<Self, Self::Error> {
        read::<S>(request.extensions(), request.headers())?
            .map(Self)
            .ok_or(HeaderRejection::Missing)
    }
}

impl<'a, S: HeaderSpec + 'a> FromRequest<'a> for OptionalHeader<S> {
    type Error = HeaderRejection;
    const ENTRIES: Entries = Entries::NONE;

    async fn from_request(request: &'a mut Request) -> Result<Self, Self::Error> {
        Ok(Self(read::<S>(request.extensions(), request.headers())?))
    }
}

impl<S: HeaderSpec> DescribeRequest for Header<S> {
    fn describe(cx: &mut OperationCx) {
        cx.request_header::<S::Value>(S::NAME, Presence::Required);
        cx.mark_validated();
    }
}

impl<S: HeaderSpec> DescribeRequest for OptionalHeader<S> {
    fn describe(cx: &mut OperationCx) {
        cx.request_header::<S::Value>(S::NAME, Presence::Optional);
        cx.mark_validated();
    }
}
