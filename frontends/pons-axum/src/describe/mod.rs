mod header;
mod request;
mod response;

use pons_core::TypeCx;
use pons_ir::{
    AuthRequirement, Body, CredentialProvision, Diagnostic, ErrorResponse, HeaderParameter,
    Response, StreamDescription, TypeRef,
};

pub use header::HeaderSpec;
pub use request::{Header, HeaderRejection, OptionalHeader};
pub use response::{NoContent, WithHeader, WithOptionalHeader, WithStatus};

pub(crate) use header::header_schema;
pub(crate) use response::{
    insert_response_header, invalid_response_header, is_invalid_response_header, valid_status,
};

#[cfg(test)]
use pons_core::Schema;
#[cfg(test)]
use pons_ir::{ContentKind, Direction, InlineType, IntegerWire, OpaqueWire};
#[cfg(test)]
use request::header_text;

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
                ext: Default::default(),
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
