use std::any::type_name;
use std::collections::BTreeMap;

use pons_core::Schema;
#[cfg(feature = "sse")]
use pons_ir::StreamMessage;
use pons_ir::{
    Body, ContentKind, Diagnostic, DiagnosticSeverity, Direction, InlineType, NamedRef, Presence,
    Response, ResponseStatus, TypeDef, TypeKind, TypeRef,
};
#[cfg(feature = "typed-header")]
use pons_ir::{HeaderParameter, OpaqueWire};
#[cfg(any(feature = "sse", feature = "websocket"))]
use pons_ir::{StreamDescription, StreamKind};

use crate::describe::OperationCx;

impl OperationCx {
    pub(crate) fn path_type<T: Schema>(&mut self) {
        let ty = self.flat_carrier::<T>("path");
        self.path_types.push(ty);
    }

    fn flat_carrier<T: Schema>(&mut self, role: &'static str) -> TypeRef {
        let ty = self
            .types
            .register_as(format!("{}#{role}", type_name::<T>()), |types| {
                T::schema(types, Direction::Input)
            });
        self.flat_carriers.push((ty.clone(), role));
        self.carriers.push(ty.clone());
        ty
    }

    #[cfg(feature = "typed-header")]
    pub(crate) fn request_opaque_header(&mut self, wire: &str, codec: &'static str) {
        self.headers.push(HeaderParameter {
            wire: wire.to_owned(),
            ident: header_ident(wire),
            ty: TypeRef::Inline(InlineType::Opaque {
                codec: codec.into(),
                wire: OpaqueWire::Text,
                ext: Default::default(),
            }),
            presence: Presence::Required,
            ext: Default::default(),
        });
    }

    pub(crate) fn query_type<T: Schema>(&mut self) {
        let query = self.flat_carrier::<T>("query");
        if self.query.replace(query).is_some() {
            self.diagnostic(
                "PONS-W101",
                "multiple query extractors are not represented separately",
            );
        }
    }

    pub(crate) fn request_body<T: Schema>(&mut self, content: ContentKind) {
        let ty = match content {
            ContentKind::Form => self.flat_carrier::<T>("form"),
            _ => T::schema(&mut self.types, Direction::Input),
        };
        let body = Body {
            ty,
            content,
            required: true,
            media: None,
            ext: Default::default(),
        };
        if self.body.replace(body).is_some() {
            self.diagnostic(
                "PONS-W101",
                "multiple request body extractors cannot be represented",
            );
        }
    }

    pub(crate) fn response<T: Schema>(&mut self, status: u16, content: ContentKind) {
        self.responses.push(Response {
            status: ResponseStatus::Exact(status),
            content,
            media: None,
            body: Some(T::schema(&mut self.types, Direction::Output)),
            headers: Vec::new(),
            ext: Default::default(),
        });
    }

    pub(crate) fn empty_response(&mut self, status: u16) {
        self.responses.push(Response {
            status: ResponseStatus::Exact(status),
            content: ContentKind::Empty,
            media: None,
            body: None,
            headers: Vec::new(),
            ext: Default::default(),
        });
    }

    #[cfg(feature = "sse")]
    pub(crate) fn sse<T: Schema>(&mut self, event: &'static str) {
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
            media: None,
            body: None,
            headers: Vec::new(),
            ext: Default::default(),
        });
    }

    #[cfg(feature = "websocket")]
    pub(crate) fn websocket<Client: Schema, Server: Schema>(&mut self) {
        self.stream = Some(StreamDescription {
            kind: StreamKind::Websocket {
                client: Client::schema(&mut self.types, Direction::Input),
                server: Server::schema(&mut self.types, Direction::Output),
            },
        });
        self.empty_response(101);
    }

    pub(crate) fn diagnostic(&mut self, code: &str, message: impl Into<String>) {
        push_diagnostic(&mut self.diagnostics, code, message);
    }
}

#[cfg(feature = "typed-header")]
fn header_ident(wire: &str) -> Vec<String> {
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
    if ident.is_empty() {
        vec!["header".to_owned()]
    } else {
        ident
    }
}

pub(crate) fn lower_flat_carriers(
    types: &mut BTreeMap<String, TypeDef>,
    carriers: &[(TypeRef, &str)],
) {
    carriers.iter().for_each(|(carrier, role)| {
        lower_flat_carrier(types, carrier, role, &mut Vec::new());
    });
}

fn lower_flat_carrier(
    types: &mut BTreeMap<String, TypeDef>,
    carrier: &TypeRef,
    role: &str,
    visiting: &mut Vec<String>,
) {
    let TypeRef::Named(named) = carrier else {
        return;
    };
    if visiting.contains(&named.id) {
        return;
    }
    let Some(definition) = types.get_mut(&named.id) else {
        return;
    };
    visiting.push(named.id.clone());
    let nested = match &mut definition.definition {
        TypeKind::Newtype { inner } => vec![inner.clone()],
        TypeKind::Struct {
            fields, flattened, ..
        } => {
            fields.iter_mut().for_each(|field| {
                if field.presence == Presence::Optional
                    && let TypeRef::Inline(InlineType::Nullable { inner }) = &field.ty
                {
                    field.ty = *inner.clone();
                }
            });
            flattened.clone()
        }
        TypeKind::Enum { .. } => Vec::new(),
    };
    let owned = nested
        .iter()
        .map(|ty| own(types, ty, role))
        .collect::<Vec<_>>();
    if let Some(definition) = types.get_mut(&named.id) {
        match &mut definition.definition {
            TypeKind::Newtype { inner } => {
                if let [only] = owned.as_slice() {
                    *inner = only.clone();
                }
            }
            TypeKind::Struct { flattened, .. } => *flattened = owned.clone(),
            TypeKind::Enum { .. } => {}
        }
    }
    owned
        .iter()
        .for_each(|ty| lower_flat_carrier(types, ty, role, visiting));
    visiting.pop();
}

fn own(types: &mut BTreeMap<String, TypeDef>, ty: &TypeRef, role: &str) -> TypeRef {
    let TypeRef::Named(reference) = ty else {
        return ty.clone();
    };
    if reference.id.ends_with(&format!("#{role}")) {
        return ty.clone();
    }
    let owned = format!("{}#{role}", reference.id);
    let Some(definition) = types.get(&reference.id).cloned() else {
        return ty.clone();
    };
    types.insert(owned.clone(), definition);
    TypeRef::Named(NamedRef {
        id: owned,
        variant: reference.variant.clone(),
    })
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
        fidelity: None,
        capability: None,
        ext: Default::default(),
    });
}

fn severity_for(code: &str) -> DiagnosticSeverity {
    match code.split('-').nth(1) {
        Some(segment) if segment.starts_with('W') => DiagnosticSeverity::Warning,
        _ => DiagnosticSeverity::Error,
    }
}
