use axum::response::{IntoResponse, Response};
use pons_core::{ErrorSpec, Schema, TypeCx};
use pons_ir::{Direction, ErrorResponse, Presence, ResponseHeader};

use crate::describe::{HeaderSpec, insert_response_header, invalid_response_header};

pub struct WithErrorHeader<S: HeaderSpec, E>(pub E, pub <S as HeaderSpec>::Value);

impl<S: HeaderSpec, E> WithErrorHeader<S, E> {
    pub fn new(error: E, value: S::Value) -> Self {
        Self(error, value)
    }
}

impl<S, E> IntoResponse for WithErrorHeader<S, E>
where
    S: HeaderSpec,
    S::Value: ToString,
    E: IntoResponse,
{
    fn into_response(self) -> Response {
        let mut response = self.0.into_response();
        if insert_response_header::<S>(&mut response, &self.1).is_err() {
            return invalid_response_header();
        }
        response
    }
}

impl<S, E> ErrorSpec for WithErrorHeader<S, E>
where
    S: HeaderSpec,
    E: ErrorSpec,
{
    fn errors(cx: &mut TypeCx) -> Vec<ErrorResponse> {
        let mut errors = E::errors(cx);
        let header = response_header::<S>(cx, Presence::Required);
        errors
            .iter_mut()
            .for_each(|error| error.headers.push(header.clone()));
        errors
    }
}

pub struct WithOptionalErrorHeader<S: HeaderSpec, E>(pub E, pub Option<<S as HeaderSpec>::Value>);

impl<S: HeaderSpec, E> WithOptionalErrorHeader<S, E> {
    pub fn new(error: E, value: Option<S::Value>) -> Self {
        Self(error, value)
    }
}

impl<S, E> IntoResponse for WithOptionalErrorHeader<S, E>
where
    S: HeaderSpec,
    S::Value: ToString,
    E: IntoResponse,
{
    fn into_response(self) -> Response {
        let mut response = self.0.into_response();
        if let Some(value) = self.1
            && insert_response_header::<S>(&mut response, &value).is_err()
        {
            return invalid_response_header();
        }
        response
    }
}

impl<S, E> ErrorSpec for WithOptionalErrorHeader<S, E>
where
    S: HeaderSpec,
    E: ErrorSpec,
{
    fn errors(cx: &mut TypeCx) -> Vec<ErrorResponse> {
        let mut errors = E::errors(cx);
        let header = response_header::<S>(cx, Presence::Optional);
        errors
            .iter_mut()
            .for_each(|error| error.headers.push(header.clone()));
        errors
    }
}

fn response_header<S>(cx: &mut TypeCx, presence: Presence) -> ResponseHeader
where
    S: HeaderSpec,
{
    ResponseHeader {
        wire: S::NAME.to_owned(),
        ty: <S::Value as Schema>::schema(cx, Direction::Output),
        presence,
        ext: Default::default(),
    }
}
