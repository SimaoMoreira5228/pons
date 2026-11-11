use pons_core::{ErrorSpec, Schema, TypeCx};
use pons_ir::{Direction, Doc, ErrorContent, ErrorResponse, ErrorStatus, TypeRef};
use tako::extractors::json::Json;
use tako::responder::Responder;
use tako::types::Response;

pub(crate) const REJECTED: u16 = 422;
pub(crate) const REJECTED_NAME: [&str; 2] = ["request", "rejected"];

#[derive(Debug, serde::Serialize, pons_core::Schema)]
#[serde(deny_unknown_fields)]
pub struct Violation {
    pub path: String,
    pub reason: String,
}

#[derive(Debug, serde::Serialize, pons_core::Schema)]
#[serde(deny_unknown_fields)]
pub struct ViolationReport {
    pub violations: Vec<Violation>,
}

impl ViolationReport {
    pub(crate) fn rejected(violations: Vec<pons_ir::Violation>) -> Self {
        Self {
            violations: violations
                .into_iter()
                .map(|violation| Violation {
                    path: violation.path().to_owned(),
                    reason: violation.reason(),
                })
                .collect(),
        }
    }

    pub(crate) fn error(body: Option<TypeRef>) -> ErrorResponse {
        ErrorResponse {
            status: ErrorStatus::Exact(REJECTED),
            name: REJECTED_NAME
                .iter()
                .map(|word| (*word).to_owned())
                .collect(),
            content: match &body {
                Some(_) => ErrorContent::Json,
                None => ErrorContent::Empty,
            },
            media: None,
            body,
            headers: Vec::new(),
            retryable: Some(false),
            doc: Some(Doc {
                summary: Some(
                    "The request does not satisfy the published request contract.".to_owned(),
                ),
                description: None,
                deprecated: None,
                tags: Vec::new(),
            }),
            ext: Default::default(),
        }
    }
}

impl ErrorSpec for ViolationReport {
    fn errors(cx: &mut TypeCx) -> Vec<ErrorResponse> {
        vec![Self::error(Some(<Self as Schema>::schema(
            cx,
            Direction::Output,
        )))]
    }
}

impl Responder for ViolationReport {
    fn into_response(self) -> Response {
        (http::StatusCode::UNPROCESSABLE_ENTITY, Json(self)).into_response()
    }
}
