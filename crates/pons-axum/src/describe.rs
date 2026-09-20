use pons_core::{ErrorSpec, Schema, TypeCx};
use pons_ir::{
    AuthRequirement, Body, ContentKind, Diagnostic, DiagnosticSeverity, Direction, ErrorResponse,
    InlineType, Response, StreamDescription, StreamKind, StreamMessage, TypeRef,
};

#[derive(Debug, Default)]
pub struct OperationCx {
    pub(crate) types: TypeCx,
    pub(crate) path_types: Vec<TypeRef>,
    pub(crate) query: Option<TypeRef>,
    pub(crate) body: Option<Body>,
    pub(crate) responses: Vec<Response>,
    pub(crate) errors: Vec<ErrorResponse>,
    pub(crate) stream: Option<StreamDescription>,
    pub(crate) auth: AuthRequirement,
    pub(crate) diagnostics: Vec<Diagnostic>,
}

pub trait DescribeRequest {
    fn describe(cx: &mut OperationCx);
}

pub trait DescribeResponse {
    fn describe(cx: &mut OperationCx);
}

impl OperationCx {
    pub fn type_ref<T: Schema>(&mut self, direction: Direction) -> TypeRef {
        T::schema(&mut self.types, direction)
    }

    pub fn path_type<T: Schema>(&mut self) {
        self.path_types
            .push(T::schema(&mut self.types, Direction::Input));
    }

    pub fn query_type<T: Schema>(&mut self) {
        let query = T::schema(&mut self.types, Direction::Input);
        if self.query.replace(query).is_some() {
            self.diagnostic(
                "PONS-W101",
                "multiple query extractors are not represented separately",
            );
        }
    }

    pub fn request_body<T: Schema>(&mut self, content: ContentKind) {
        let body = Body {
            ty: T::schema(&mut self.types, Direction::Input),
            content,
            ext: Default::default(),
        };
        if self.body.replace(body).is_some() {
            self.diagnostic(
                "PONS-W101",
                "multiple request body extractors cannot be represented",
            );
        }
    }

    pub fn response<T: Schema>(&mut self, status: u16, content: ContentKind) {
        self.responses.push(Response {
            status,
            content,
            body: Some(T::schema(&mut self.types, Direction::Output)),
            headers: Vec::new(),
            ext: Default::default(),
        });
    }

    pub fn empty_response(&mut self, status: u16) {
        self.responses.push(Response {
            status,
            content: ContentKind::Empty,
            body: None,
            headers: Vec::new(),
            ext: Default::default(),
        });
    }

    pub fn sse<T: Schema>(&mut self, event: &'static str) {
        self.stream = Some(StreamDescription {
            kind: StreamKind::Sse {
                events: vec![StreamMessage {
                    event: event.to_owned(),
                    data: T::schema(&mut self.types, Direction::Output),
                }],
            },
        });
        self.responses.push(Response {
            status: 200,
            content: ContentKind::Stream,
            body: None,
            headers: Vec::new(),
            ext: Default::default(),
        });
    }

    pub fn require_auth(&mut self, scheme: impl Into<String>) {
        let scheme = scheme.into();
        if let Some(alternatives) = self.auth.alternatives.first_mut() {
            alternatives.push(scheme);
        } else {
            self.auth.alternatives.push(vec![scheme]);
        }
    }

    pub fn diagnostic(&mut self, code: &str, message: impl Into<String>) {
        let message = message.into();
        if self
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == code && diagnostic.message == message)
        {
            return;
        }
        self.diagnostics.push(Diagnostic {
            severity: DiagnosticSeverity::Warning,
            code: code.to_owned(),
            message,
            origin: None,
            ext: Default::default(),
        });
    }

    pub(crate) fn type_definitions(&self) -> &std::collections::BTreeMap<String, pons_ir::TypeDef> {
        self.types.definitions()
    }
}

impl<T: Schema> DescribeRequest for axum::Json<T> {
    fn describe(cx: &mut OperationCx) {
        cx.request_body::<T>(ContentKind::Json);
    }
}

impl<T: Schema> DescribeRequest for axum::extract::Query<T> {
    fn describe(cx: &mut OperationCx) {
        cx.query_type::<T>();
    }
}

impl<T: Schema> DescribeRequest for axum::extract::Path<T> {
    fn describe(cx: &mut OperationCx) {
        cx.path_type::<T>();
    }
}

impl<T> DescribeRequest for axum::extract::State<T> {
    fn describe(_: &mut OperationCx) {}
}

impl<T> DescribeRequest for axum::extract::Extension<T> {
    fn describe(_: &mut OperationCx) {}
}

impl DescribeRequest for axum::http::HeaderMap {
    fn describe(_: &mut OperationCx) {}
}

impl DescribeRequest for String {
    fn describe(cx: &mut OperationCx) {
        cx.request_body::<String>(ContentKind::Text);
    }
}

impl DescribeRequest for axum::body::Bytes {
    fn describe(cx: &mut OperationCx) {
        cx.request_body::<ByteArray>(ContentKind::Bytes);
    }
}

impl DescribeResponse for axum::response::Response {
    fn describe(cx: &mut OperationCx) {
        cx.diagnostic(
            "PONS-W101",
            "raw Axum responses do not expose their status or body; represented as 200 JSON any",
        );
        cx.responses.push(Response {
            status: 200,
            content: ContentKind::Json,
            body: Some(TypeRef::Inline(InlineType::Any)),
            headers: Vec::new(),
            ext: Default::default(),
        });
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
        cx.errors.extend(E::errors(&mut cx.types));
    }
}

impl<R: DescribeResponse> DescribeResponse for (axum::http::StatusCode, R) {
    fn describe(cx: &mut OperationCx) {
        let first_response = cx.responses.len();
        R::describe(cx);
        for response in &mut cx.responses[first_response..] {
            response.status = 200;
        }
        cx.diagnostic("PONS-W101", "runtime response status is represented as 200");
    }
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

impl Schema for ByteArray {
    fn schema(_: &mut TypeCx, _: Direction) -> TypeRef {
        TypeRef::Inline(InlineType::Bytes {
            encoding: pons_ir::ByteEncoding::Base64,
        })
    }
}
