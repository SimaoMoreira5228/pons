use pons_core::Schema;
use pons_ir::{
    Body, ContentKind, CredentialProvision, Diagnostic, DiagnosticSeverity, Direction,
    HeaderParameter, Presence, Response, ResponseStatus, StreamDescription, StreamKind,
    StreamMessage, TypeRef,
};

use crate::describe::OperationCx;

impl OperationCx {
    pub fn type_ref<T: Schema>(&mut self, direction: Direction) -> TypeRef {
        T::schema(&mut self.types, direction)
    }

    pub fn path_type<T: Schema>(&mut self) {
        self.path_types
            .push(T::schema(&mut self.types, Direction::Input));
    }

    pub fn request_header<T: Schema>(&mut self, wire: &str, presence: Presence) {
        let base = if wire.len() >= 2
            && matches!(
                (wire.as_bytes()[0], wire.as_bytes()[1]),
                (b'x' | b'X', b'-' | b'_')
            ) {
            &wire[2..]
        } else {
            wire
        };
        let ident = base
            .split(['-', '_'])
            .filter(|part| !part.is_empty())
            .map(str::to_ascii_lowercase)
            .collect::<Vec<_>>();
        self.headers.push(HeaderParameter {
            wire: wire.to_owned(),
            ident: if ident.is_empty() {
                vec!["header".to_owned()]
            } else {
                ident
            },
            ty: crate::describe::header_schema::<T>(&mut self.types, Direction::Input, wire),
            presence,
            ext: Default::default(),
        });
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
        self.request_body_with_presence::<T>(content, None, true);
    }

    pub fn optional_request_body<T: Schema>(&mut self, content: ContentKind) {
        self.request_body_with_presence::<T>(content, None, false);
    }

    pub fn request_body_with_media<T: Schema>(
        &mut self,
        content: ContentKind,
        media_type: Option<String>,
    ) {
        self.request_body_with_presence::<T>(content, media_type, true);
    }

    pub fn optional_request_body_with_media<T: Schema>(
        &mut self,
        content: ContentKind,
        media_type: Option<String>,
    ) {
        self.request_body_with_presence::<T>(content, media_type, false);
    }

    fn request_body_with_presence<T: Schema>(
        &mut self,
        content: ContentKind,
        media_type: Option<String>,
        required: bool,
    ) {
        let body = Body {
            ty: T::schema(&mut self.types, Direction::Input),
            content,
            required,
            media_type,
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
        self.response_with_media::<T>(status, content, None);
    }

    pub fn response_with_media<T: Schema>(
        &mut self,
        status: u16,
        content: ContentKind,
        media_type: Option<String>,
    ) {
        self.responses.push(Response {
            status: ResponseStatus::Exact(status),
            content,
            media_type,
            body: Some(T::schema(&mut self.types, Direction::Output)),
            headers: Vec::new(),
            ext: Default::default(),
        });
    }

    pub fn unknown_response<T: Schema>(&mut self, content: ContentKind) {
        self.responses.push(Response {
            status: ResponseStatus::Unknown,
            content,
            media_type: None,
            body: Some(T::schema(&mut self.types, Direction::Output)),
            headers: Vec::new(),
            ext: Default::default(),
        });
    }

    pub fn empty_response(&mut self, status: u16) {
        self.responses.push(Response {
            status: ResponseStatus::Exact(status),
            content: ContentKind::Empty,
            media_type: None,
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
            status: ResponseStatus::Exact(200),
            content: ContentKind::Stream,
            media_type: None,
            body: None,
            headers: Vec::new(),
            ext: Default::default(),
        });
    }

    pub fn ndjson<T: Schema>(&mut self) {
        self.stream = Some(StreamDescription {
            kind: StreamKind::Ndjson {
                item: T::schema(&mut self.types, Direction::Output),
            },
        });
        self.responses.push(Response {
            status: ResponseStatus::Exact(200),
            content: ContentKind::Stream,
            media_type: None,
            body: None,
            headers: Vec::new(),
            ext: Default::default(),
        });
    }

    pub fn bytes_stream(&mut self, content_type: impl Into<String>) {
        self.stream = Some(StreamDescription {
            kind: StreamKind::Bytes {
                content_type: content_type.into(),
            },
        });
        self.responses.push(Response {
            status: ResponseStatus::Exact(200),
            content: ContentKind::Stream,
            media_type: None,
            body: None,
            headers: Vec::new(),
            ext: Default::default(),
        });
    }

    pub fn provide(&mut self, provides: CredentialProvision) {
        self.provides = Some(provides);
    }

    pub fn diagnostic(&mut self, code: &str, message: impl Into<String>) {
        push_diagnostic(&mut self.diagnostics, code, message);
    }

    pub(crate) fn type_definitions(&self) -> &std::collections::BTreeMap<String, pons_ir::TypeDef> {
        self.types.definitions()
    }
}

pub(crate) fn push_diagnostic(
    diagnostics: &mut Vec<Diagnostic>,
    code: &str,
    message: impl Into<String>,
) {
    let message = message.into();
    if diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == code && diagnostic.message == message)
    {
        return;
    }
    diagnostics.push(Diagnostic {
        severity: severity_for(code),
        code: code.to_owned(),
        message,
        origin: None,
        ext: Default::default(),
    });
}

fn severity_for(code: &str) -> DiagnosticSeverity {
    match code.split('-').nth(1) {
        Some(segment) if segment.starts_with('W') => DiagnosticSeverity::Warning,
        _ => DiagnosticSeverity::Error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn severity_follows_the_code_and_duplicates_are_dropped() {
        let mut diagnostics = Vec::new();
        push_diagnostic(&mut diagnostics, "PONS-W102", "wildcard segment");
        push_diagnostic(&mut diagnostics, "PONS-W102", "wildcard segment");
        push_diagnostic(&mut diagnostics, "PONS-E101", "flatten conflict");
        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].severity, DiagnosticSeverity::Warning);
        assert_eq!(diagnostics[1].severity, DiagnosticSeverity::Error);
    }
}
