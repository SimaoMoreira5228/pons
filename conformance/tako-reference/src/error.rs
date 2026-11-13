use pons_core::{ErrorSpec, TypeCx};
use pons_ir::ErrorResponse;
use pons_tako::StatusCode;
use tako::extractors::json::Json;
use tako::responder::Responder;
use tako::types::Response;

pub struct InvalidCredentials(pub reference_api::InvalidCredentials);

impl ErrorSpec for InvalidCredentials {
    fn errors(cx: &mut TypeCx) -> Vec<ErrorResponse> {
        <reference_api::InvalidCredentials as ErrorSpec>::errors(cx)
    }
}

impl Responder for InvalidCredentials {
    fn into_response(self) -> Response {
        (StatusCode::UNAUTHORIZED, Json(self.0)).into_response()
    }
}

pub struct RateLimited(pub reference_api::RateLimited);

impl ErrorSpec for RateLimited {
    fn errors(cx: &mut TypeCx) -> Vec<ErrorResponse> {
        <reference_api::RateLimited as ErrorSpec>::errors(cx)
    }
}

impl Responder for RateLimited {
    fn into_response(self) -> Response {
        let retry_after = self.0.retry_after_secs.to_string();
        let mut response = (StatusCode::TOO_MANY_REQUESTS, Json(self.0)).into_response();
        let Ok(value) = http::HeaderValue::from_str(&retry_after) else {
            return (StatusCode::INTERNAL_SERVER_ERROR, "").into_response();
        };
        response.headers_mut().insert("Retry-After", value);
        response
    }
}
