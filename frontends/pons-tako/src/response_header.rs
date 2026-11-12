use headers::Header;
use pons_core::{ErrorSpec, TypeCx};
use pons_ir::{ErrorResponse, Presence, ResponseHeader};
use tako_rs_core::responder::Responder;
use tako_rs_core::types::Response;

use crate::describe::{DescribeResponse, OperationCx};

pub struct WithHeader<H: Header, R>(pub R, pub H);

impl<H: Header, R> WithHeader<H, R> {
    pub fn new(responder: R, value: H) -> Self {
        Self(responder, value)
    }
}

impl<H: Header, R: Responder> Responder for WithHeader<H, R> {
    fn into_response(self) -> Response {
        let mut response = self.0.into_response();
        set_header(&mut response, &self.1);
        response
    }
}

impl<H: Header, R: DescribeResponse> DescribeResponse for WithHeader<H, R> {
    fn describe(cx: &mut OperationCx) {
        let start = cx.responses.len();
        R::describe(cx);
        cx.response_header(start, H::name().as_str(), Presence::Required);
    }
}

pub struct WithOptionalHeader<H: Header, R>(pub R, pub Option<H>);

impl<H: Header, R> WithOptionalHeader<H, R> {
    pub fn new(responder: R, value: Option<H>) -> Self {
        Self(responder, value)
    }
}

impl<H: Header, R: Responder> Responder for WithOptionalHeader<H, R> {
    fn into_response(self) -> Response {
        let mut response = self.0.into_response();
        if let Some(value) = &self.1 {
            set_header(&mut response, value);
        }
        response
    }
}

impl<H: Header, R: DescribeResponse> DescribeResponse for WithOptionalHeader<H, R> {
    fn describe(cx: &mut OperationCx) {
        let start = cx.responses.len();
        R::describe(cx);
        cx.response_header(start, H::name().as_str(), Presence::Optional);
    }
}

pub struct WithErrorHeader<H: Header, E>(pub E, pub H);

impl<H: Header, E> WithErrorHeader<H, E> {
    pub fn new(error: E, value: H) -> Self {
        Self(error, value)
    }
}

impl<H: Header, E: Responder> Responder for WithErrorHeader<H, E> {
    fn into_response(self) -> Response {
        let mut response = self.0.into_response();
        set_header(&mut response, &self.1);
        response
    }
}

impl<H: Header, E: ErrorSpec> ErrorSpec for WithErrorHeader<H, E> {
    fn errors(cx: &mut TypeCx) -> Vec<ErrorResponse> {
        with_error_header(E::errors(cx), H::name().as_str(), Presence::Required)
    }
}

pub struct WithOptionalErrorHeader<H: Header, E>(pub E, pub Option<H>);

impl<H: Header, E> WithOptionalErrorHeader<H, E> {
    pub fn new(error: E, value: Option<H>) -> Self {
        Self(error, value)
    }
}

impl<H: Header, E: Responder> Responder for WithOptionalErrorHeader<H, E> {
    fn into_response(self) -> Response {
        let mut response = self.0.into_response();
        if let Some(value) = &self.1 {
            set_header(&mut response, value);
        }
        response
    }
}

impl<H: Header, E: ErrorSpec> ErrorSpec for WithOptionalErrorHeader<H, E> {
    fn errors(cx: &mut TypeCx) -> Vec<ErrorResponse> {
        with_error_header(E::errors(cx), H::name().as_str(), Presence::Optional)
    }
}

fn set_header<H: Header>(response: &mut Response, value: &H) {
    let mut values = Vec::new();
    value.encode(&mut values);
    let headers = response.headers_mut();
    headers.remove(H::name());
    values.into_iter().for_each(|value| {
        headers.append(H::name(), value);
    });
}

fn with_error_header(
    mut errors: Vec<ErrorResponse>,
    name: &str,
    presence: Presence,
) -> Vec<ErrorResponse> {
    let header = ResponseHeader {
        wire: name.to_owned(),
        ty: crate::operation_cx::opaque_header_type(),
        presence,
        ext: Default::default(),
    };
    errors
        .iter_mut()
        .for_each(|error| error.headers.push(header.clone()));
    errors
}
