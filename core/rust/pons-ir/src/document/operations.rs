use std::collections::BTreeSet;

use crate::operation::{ContentKind, Operation, PathSegment};
use crate::pagination::PaginationRequest;
use crate::types::{Presence, TypeRef};

use super::{Document, ValidationError};

fn owns_request_header(wire: &str, has_body: bool) -> bool {
    match wire.to_ascii_lowercase().as_str() {
        "content-type" => has_body,
        "accept" | "authorization" | "cookie" => true,
        _ => false,
    }
}

impl Document {
    pub(super) fn validate_operations(&self, errors: &mut Vec<ValidationError>) {
        let mut operation_ids = BTreeSet::new();
        for operation in &self.operations {
            self.validate_operation(operation, &mut operation_ids, errors);
        }
    }

    fn validate_operation(
        &self,
        operation: &Operation,
        operation_ids: &mut BTreeSet<Vec<Vec<String>>>,
        errors: &mut Vec<ValidationError>,
    ) {
        let id = operation.id.clone();
        let owner = format!("operation {id:?}");
        self.validate_diagnostics(&owner, &operation.diagnostics, errors);
        if id.is_empty()
            || id
                .iter()
                .any(|name| name.is_empty() || name.iter().any(String::is_empty))
        {
            errors.push(ValidationError::InvalidValue {
                path: owner.clone(),
                message: "operation id must contain non-empty name segments".into(),
            });
        }
        if !operation_ids.insert(id.clone()) {
            errors.push(ValidationError::DuplicateOperation { id });
        }

        let mut path_parameters = BTreeSet::new();
        for segment in &operation.path {
            if let PathSegment::Param { param, ty } = segment {
                if !path_parameters.insert(param) {
                    errors.push(ValidationError::InvalidValue {
                        path: owner.clone(),
                        message: format!("duplicate path parameter {param}"),
                    });
                }
                if let Err(refusal) = crate::query::path_fields(self, param, ty) {
                    errors.push(ValidationError::InvalidValue {
                        path: owner.clone(),
                        message: refusal.in_carrier(&crate::query::PATH),
                    });
                }
            }
        }
        let mut header_names = BTreeSet::new();
        for header in &operation.headers {
            self.validate_header_name(
                &owner,
                "request header",
                &header.wire,
                &mut header_names,
                errors,
            );
            if header.presence == Presence::Required
                && self.is_nullable_type(&header.ty, &mut BTreeSet::new())
            {
                errors.push(ValidationError::InvalidValue {
                    path: owner.clone(),
                    message: format!("required request header {} cannot be nullable", header.wire),
                });
            }
            if owns_request_header(&header.wire, operation.body.is_some()) {
                errors.push(ValidationError::InvalidValue {
                    path: owner.clone(),
                    message: format!(
                        "request header {} is owned by the request body or the security model, so every client and server built from the document writes it too",
                        header.wire
                    ),
                });
            }
        }
        if let Some(body) = &operation.body {
            if body.content == ContentKind::Stream {
                errors.push(ValidationError::InvalidValue {
                    path: owner.clone(),
                    message: "a request body cannot be stream content, because no built-in target reads one"
                        .into(),
                });
            }
            if body.content == ContentKind::Empty {
                errors.push(ValidationError::InvalidValue {
                    path: owner.clone(),
                    message: format!(
                        "request body content {:?} cannot carry a body type",
                        body.content
                    ),
                });
            }
            if body.content == ContentKind::Empty && body.media.is_some() {
                errors.push(ValidationError::InvalidValue {
                    path: owner.clone(),
                    message: "empty request bodies cannot declare a media type".into(),
                });
            }
            if let Some(media) = &body.media {
                self.validate_media(
                    &owner,
                    "request body",
                    super::media::encoding(body.content),
                    media,
                    errors,
                );
            }
            if body.content == ContentKind::Bytes
                && !self.is_byte_type(&body.ty, &mut BTreeSet::new())
            {
                errors.push(ValidationError::InvalidValue {
                    path: owner.clone(),
                    message: "raw byte bodies must use a byte type, byte newtype, nullable bytes, or a bytes opaque"
                        .into(),
                });
            }
            if body.content == ContentKind::Form
                && let Err(refusal) = crate::query::form_body_fields(self, &body.ty)
            {
                errors.push(ValidationError::InvalidValue {
                    path: owner.clone(),
                    message: refusal.in_carrier(&crate::query::FORM),
                });
            }
        }
        if let Some(query) = &operation.query
            && let Err(rejection) = crate::query::query_fields(self, query)
        {
            errors.push(ValidationError::InvalidValue {
                path: owner.clone(),
                message: rejection.message(),
            });
        }
        if let Some(pagination) = &operation.semantics.pagination {
            if let Err(message) = pagination.validate() {
                errors.push(ValidationError::InvalidValue {
                    path: owner.clone(),
                    message,
                });
            }
            for field in std::iter::once(pagination.request.field())
                .chain(pagination.page_size.iter().map(|size| size.field.as_str()))
            {
                if self.request_field_type(operation, field).is_none() {
                    errors.push(ValidationError::InvalidValue {
                        path: owner.clone(),
                        message: format!(
                            "pagination field {field:?} is not declared on the operation query or body"
                        ),
                    });
                }
            }
            if let PaginationRequest::Offset { field } = &pagination.request
                && let Some(ty) = self.request_field_type(operation, field)
                && !self.is_integer_type(ty, &mut BTreeSet::new())
            {
                errors.push(ValidationError::InvalidValue {
                    path: owner.clone(),
                    message: format!(
                        "offset pagination field {field:?} must be an integer, because the client advances it by the number of items the page returned"
                    ),
                });
            }
        }
        self.visit_operation_refs(&owner, operation, errors);
        if let Some(stream) = &operation.stream
            && let Err(message) = stream.kind.validate()
        {
            errors.push(ValidationError::InvalidValue {
                path: owner.clone(),
                message,
            });
        }
        if operation.stream.is_some() && operation.semantics.pagination.is_some() {
            errors.push(ValidationError::InvalidValue {
                path: owner.clone(),
                message: "a streaming operation cannot declare pagination, because a stream has no page body".into(),
            });
        }

        self.validate_operation_auth(&owner, operation, errors);
        self.validate_responses(&owner, operation, errors);
        self.validate_operation_errors(&owner, operation, errors);
    }

    fn request_field_type<'a>(
        &'a self,
        operation: &'a Operation,
        wire: &str,
    ) -> Option<&'a TypeRef> {
        if let Some(query) = &operation.query
            && let Ok(fields) = crate::query::query_fields(self, query)
            && let Some(field) = fields.iter().find(|field| field.wire == wire)
        {
            return Some(&field.ty);
        }
        operation
            .body
            .as_ref()
            .and_then(|body| self.struct_field_type(&body.ty, wire, &mut BTreeSet::new()))
    }
}
