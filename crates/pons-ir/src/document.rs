use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::IR_VERSION;
use crate::operation::{
    AuthScheme, ContentKind, ErrorContent, ErrorResponse, ErrorStatus, Operation,
};
use crate::types::{
    Diagnostic, EnumRepr, Field, InlineType, TypeDef, TypeKind, TypeRef, VariantPayload,
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Document {
    #[schemars(regex(pattern = "^0\\.1$"))]
    pub ir_version: String,
    pub info: Info,
    #[serde(default)]
    pub types: BTreeMap<String, TypeDef>,
    #[serde(default)]
    pub auth_schemes: BTreeMap<String, AuthScheme>,
    #[serde(default)]
    pub common_errors: Vec<ErrorResponse>,
    #[serde(default)]
    pub operations: Vec<Operation>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub ext: BTreeMap<String, Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<Diagnostic>,
}

impl Document {
    pub fn canonicalize(&mut self) {
        self.types
            .values_mut()
            .for_each(|definition| definition.definition.canonicalize());
        self.auth_schemes
            .values_mut()
            .for_each(AuthScheme::canonicalize);
        self.common_errors.sort_by(|left, right| {
            left.status
                .cmp(&right.status)
                .then_with(|| left.name.cmp(&right.name))
        });
        self.operations
            .sort_by(|left, right| left.id.cmp(&right.id));
        self.operations.iter_mut().for_each(Operation::canonicalize);
    }

    pub fn to_canonical_json(&self) -> Result<String, serde_json::Error> {
        let mut document = self.clone();
        document.canonicalize();
        serde_json::to_string_pretty(&document)
    }

    pub fn validate(&self) -> Result<(), Vec<ValidationError>> {
        let mut errors = Vec::new();

        if self.ir_version != IR_VERSION {
            errors.push(ValidationError::UnsupportedVersion {
                found: self.ir_version.clone(),
            });
        }

        for (id, definition) in &self.types {
            self.validate_type(id, definition, &mut errors);
        }

        for error in &self.common_errors {
            self.validate_error("common_errors", error, &mut errors);
            self.visit_error_refs("common_errors", error, &mut errors);
        }

        let mut operation_ids = BTreeSet::new();
        for operation in &self.operations {
            let id = operation.id.clone();
            let owner = format!("operation {id:?}");
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
                if let crate::operation::PathSegment::Param { param, .. } = segment
                    && !path_parameters.insert(param)
                {
                    errors.push(ValidationError::InvalidValue {
                        path: owner.clone(),
                        message: format!("duplicate path parameter {param}"),
                    });
                }
            }
            let mut header_names = BTreeSet::new();
            for header in &operation.headers {
                self.validate_header_name(
                    &owner,
                    "request header",
                    &header.wire,
                    &mut header_names,
                    &mut errors,
                );
            }
            if let Some(body) = &operation.body
                && body.content == ContentKind::Empty
            {
                errors.push(ValidationError::InvalidValue {
                    path: owner.clone(),
                    message: format!(
                        "request body content {:?} cannot carry a body type",
                        body.content
                    ),
                });
            }
            self.visit_operation_refs(&owner, operation, &mut errors);

            for alternative in &operation.auth.alternatives {
                for scheme in alternative {
                    if !self.auth_schemes.contains_key(scheme) {
                        errors.push(ValidationError::UnknownAuthScheme {
                            owner: owner.clone(),
                            scheme: scheme.clone(),
                        });
                    }
                }
            }
            if let Some(provides) = &operation.provides
                && !self.auth_schemes.contains_key(&provides.scheme)
            {
                errors.push(ValidationError::UnknownAuthScheme {
                    owner: owner.clone(),
                    scheme: provides.scheme.clone(),
                });
            }

            let mut response_statuses = BTreeSet::new();
            let mut stream_responses = 0;
            for response in &operation.responses {
                if !(100..=599).contains(&response.status) {
                    errors.push(ValidationError::InvalidValue {
                        path: owner.clone(),
                        message: format!("invalid response status {}", response.status),
                    });
                }
                if !response_statuses.insert(response.status) {
                    errors.push(ValidationError::InvalidValue {
                        path: owner.clone(),
                        message: format!("duplicate response status {}", response.status),
                    });
                }
                let content_matches_body = match response.content {
                    ContentKind::Empty | ContentKind::Stream => response.body.is_none(),
                    _ => response.body.is_some(),
                };
                if !content_matches_body {
                    errors.push(ValidationError::InvalidValue {
                        path: owner.clone(),
                        message: format!(
                            "response {} content {:?} does not match its body",
                            response.status, response.content
                        ),
                    });
                }
                if response.content == ContentKind::Stream {
                    stream_responses += 1;
                }
                let mut response_header_names = BTreeSet::new();
                for header in &response.headers {
                    self.validate_header_name(
                        &owner,
                        &format!("response {} header", response.status),
                        &header.wire,
                        &mut response_header_names,
                        &mut errors,
                    );
                }
            }
            let stream_descriptions = usize::from(operation.stream.is_some());
            if stream_responses != stream_descriptions {
                errors.push(ValidationError::InvalidValue {
                    path: owner.clone(),
                    message: format!(
                        "operation has {stream_descriptions} stream descriptions and {stream_responses} stream responses"
                    ),
                });
            }
            for error in &operation.errors {
                self.validate_error(&owner, error, &mut errors);
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    fn validate_type(&self, id: &str, definition: &TypeDef, errors: &mut Vec<ValidationError>) {
        let owner = format!("type {id}");
        if id.is_empty()
            || definition.name.is_empty()
            || definition.name.iter().any(String::is_empty)
        {
            errors.push(ValidationError::InvalidValue {
                path: owner.clone(),
                message: "named types need a non-empty id and name".into(),
            });
        }
        definition.definition.visit_refs(&mut |reference| {
            self.validate_ref(&owner, reference, errors);
            self.validate_inline(&owner, reference, errors);
        });

        match &definition.definition {
            TypeKind::Struct { fields, .. } => self.validate_fields(&owner, fields, errors),
            TypeKind::Enum { repr, variants } => {
                if variants.is_empty() {
                    errors.push(ValidationError::InvalidValue {
                        path: owner.clone(),
                        message: "enum needs at least one variant".into(),
                    });
                }
                let mut names = BTreeSet::new();
                for variant in variants {
                    if !names.insert(&variant.wire) {
                        errors.push(ValidationError::InvalidValue {
                            path: owner.clone(),
                            message: format!("duplicate enum variant {}", variant.wire),
                        });
                    }
                    if matches!(repr, EnumRepr::String)
                        && !matches!(variant.payload, VariantPayload::Unit)
                    {
                        errors.push(ValidationError::InvalidValue {
                            path: owner.clone(),
                            message: "string enum variants must have unit payloads".into(),
                        });
                    }
                    let tag = match repr {
                        EnumRepr::Internal { tag } | EnumRepr::Adjacent { tag, .. } => Some(tag),
                        _ => None,
                    };
                    if let (Some(tag), VariantPayload::Struct { fields }) = (tag, &variant.payload)
                        && fields.iter().any(|field| field.wire == *tag)
                    {
                        errors.push(ValidationError::InvalidValue {
                            path: owner.clone(),
                            message: format!("variant {} uses enum tag field {tag}", variant.wire),
                        });
                    }
                    if let VariantPayload::Struct { fields } = &variant.payload {
                        self.validate_fields(&owner, fields, errors);
                    }
                }
            }
            TypeKind::Newtype { .. } => {}
        }
    }

    fn validate_fields(&self, owner: &str, fields: &[Field], errors: &mut Vec<ValidationError>) {
        let mut names = BTreeSet::new();
        for field in fields {
            if !names.insert(&field.wire) {
                errors.push(ValidationError::InvalidValue {
                    path: owner.into(),
                    message: format!("duplicate field {}", field.wire),
                });
            }
        }
    }

    fn validate_header_name(
        &self,
        owner: &str,
        context: &str,
        wire: &str,
        names: &mut BTreeSet<String>,
        errors: &mut Vec<ValidationError>,
    ) {
        if wire.is_empty() {
            errors.push(ValidationError::InvalidValue {
                path: owner.into(),
                message: format!("{context} name cannot be empty"),
            });
        } else if !wire
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte))
        {
            errors.push(ValidationError::InvalidValue {
                path: owner.into(),
                message: format!("{context} name {wire:?} is not a valid HTTP field name"),
            });
        } else if !names.insert(wire.to_ascii_lowercase()) {
            errors.push(ValidationError::InvalidValue {
                path: owner.into(),
                message: format!("duplicate {context} {wire}"),
            });
        }
    }

    fn validate_inline(&self, owner: &str, ty: &TypeRef, errors: &mut Vec<ValidationError>) {
        let TypeRef::Inline(ty) = ty else { return };
        match ty {
            InlineType::Int {
                bits,
                signed,
                minimum,
                maximum,
                multiple_of,
                ..
            } => {
                if ![8, 16, 32, 64, 128].contains(bits) {
                    errors.push(ValidationError::InvalidValue {
                        path: owner.into(),
                        message: format!("integer width must be 8, 16, 32, 64, or 128, got {bits}"),
                    });
                }
                for (name, value) in [
                    ("minimum", minimum),
                    ("maximum", maximum),
                    ("multiple_of", multiple_of),
                ] {
                    if value.as_ref().is_some_and(|value| !value.is_number()) {
                        errors.push(ValidationError::InvalidValue {
                            path: owner.into(),
                            message: format!("integer {name} must be a number"),
                        });
                    }
                }
                if let Some((native_minimum, native_maximum)) =
                    integer_native_bounds(*bits, *signed)
                {
                    if minimum
                        .as_ref()
                        .and_then(Value::as_f64)
                        .is_some_and(|value| value < native_minimum)
                    {
                        errors.push(ValidationError::InvalidValue {
                            path: owner.into(),
                            message: "integer minimum is below its native range".into(),
                        });
                    }
                    if maximum
                        .as_ref()
                        .and_then(Value::as_f64)
                        .is_some_and(|value| value > native_maximum)
                    {
                        errors.push(ValidationError::InvalidValue {
                            path: owner.into(),
                            message: "integer maximum is above its native range".into(),
                        });
                    }
                }
                if let (Some(minimum), Some(maximum)) = (minimum, maximum)
                    && minimum
                        .as_f64()
                        .zip(maximum.as_f64())
                        .is_some_and(|(min, max)| min > max)
                {
                    errors.push(ValidationError::InvalidValue {
                        path: owner.into(),
                        message: "integer minimum exceeds maximum".into(),
                    });
                }
                if multiple_of
                    .as_ref()
                    .is_some_and(|value| value.as_f64().is_none_or(|value| value <= 0.0))
                {
                    errors.push(ValidationError::InvalidValue {
                        path: owner.into(),
                        message: "integer multiple_of must be greater than zero".into(),
                    });
                }
            }
            InlineType::Float { bits } if ![32, 64].contains(bits) => {
                errors.push(ValidationError::InvalidValue {
                    path: owner.into(),
                    message: format!("float width must be 32 or 64, got {bits}"),
                });
            }
            InlineType::String {
                min_length: Some(minimum),
                max_length: Some(maximum),
                ..
            } if minimum > maximum => errors.push(ValidationError::InvalidValue {
                path: owner.into(),
                message: "string min_length exceeds max_length".into(),
            }),
            InlineType::Map { key, .. }
                if !matches!(key.as_ref(), TypeRef::Inline(InlineType::String { .. })) =>
            {
                errors.push(ValidationError::InvalidValue {
                    path: owner.into(),
                    message: "map keys must use an inline string type".into(),
                });
            }
            InlineType::Nullable { .. }
            | InlineType::Array { .. }
            | InlineType::Map { .. }
            | InlineType::Tuple { .. } => {}
            _ => {}
        }
    }

    fn visit_operation_refs(
        &self,
        owner: &str,
        operation: &Operation,
        errors: &mut Vec<ValidationError>,
    ) {
        operation.visit_refs(&mut |reference| {
            self.validate_ref(owner, reference, errors);
            self.validate_inline(owner, reference, errors);
        });
    }

    fn validate_error(
        &self,
        owner: &str,
        error: &ErrorResponse,
        errors: &mut Vec<ValidationError>,
    ) {
        if let ErrorStatus::Exact(status) = error.status
            && !(100..=599).contains(&status)
        {
            errors.push(ValidationError::InvalidValue {
                path: owner.into(),
                message: format!("invalid error status {status}"),
            });
        }
        if let ErrorStatus::Class(class) = error.status
            && !(1..=5).contains(&class)
        {
            errors.push(ValidationError::InvalidValue {
                path: owner.into(),
                message: format!("invalid HTTP status class {class}xx"),
            });
        }
        if (error.content == ErrorContent::Empty) == error.body.is_some() {
            errors.push(ValidationError::InvalidValue {
                path: owner.into(),
                message: format!("error content {:?} does not match its body", error.content),
            });
        }
    }

    fn visit_error_refs(
        &self,
        owner: &str,
        error: &ErrorResponse,
        errors: &mut Vec<ValidationError>,
    ) {
        if let Some(body) = &error.body {
            body.visit_refs(&mut |reference| {
                self.validate_ref(owner, reference, errors);
                self.validate_inline(owner, reference, errors);
            });
        }
    }

    fn validate_ref(&self, owner: &str, reference: &TypeRef, errors: &mut Vec<ValidationError>) {
        let TypeRef::Named(named) = reference else {
            return;
        };
        let Some(definition) = self.types.get(&named.id) else {
            errors.push(ValidationError::UnknownType {
                owner: owner.into(),
                reference: named.id.clone(),
            });
            return;
        };
        if let Some(variant) = &named.variant {
            let exists = matches!(
                &definition.definition,
                TypeKind::Enum { variants, .. } if variants.iter().any(|candidate| candidate.wire == *variant)
            );
            if !exists {
                errors.push(ValidationError::InvalidValue {
                    path: owner.into(),
                    message: format!("type {} has no enum variant {variant}", named.id),
                });
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Info {
    pub title: String,
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub servers: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub ext: BTreeMap<String, Value>,
}

fn integer_native_bounds(bits: u16, signed: bool) -> Option<(f64, f64)> {
    if !matches!(bits, 8 | 16 | 32 | 64 | 128) {
        return None;
    }
    let exponent = i32::from(bits) - 1;
    if signed {
        let maximum = 2_f64.powi(exponent) - 1.0;
        Some((-maximum - 1.0, maximum))
    } else {
        Some((0.0, 2_f64.powi(exponent + 1) - 1.0))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationError {
    UnsupportedVersion { found: String },
    DuplicateOperation { id: Vec<Vec<String>> },
    UnknownType { owner: String, reference: String },
    UnknownAuthScheme { owner: String, scheme: String },
    InvalidValue { path: String, message: String },
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedVersion { found } => write!(f, "unsupported IR version {found}"),
            Self::DuplicateOperation { id } => write!(f, "duplicate operation id {id:?}"),
            Self::UnknownType { owner, reference } => {
                write!(f, "{owner} refers to unknown type {reference}")
            }
            Self::UnknownAuthScheme { owner, scheme } => {
                write!(f, "{owner} refers to unknown auth scheme {scheme}")
            }
            Self::InvalidValue { path, message } => write!(f, "{path}: {message}"),
        }
    }
}

impl std::error::Error for ValidationError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rich_document_round_trips_and_validates() {
        let document: Document =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        document.validate().unwrap();
        let encoded = serde_json::to_value(&document).unwrap();
        assert_eq!(encoded["ext"]["source.note"]["kept"], true);
        assert_eq!(encoded["types"]["User"]["ext"]["source.type"], "kept");
        assert_eq!(encoded["auth_schemes"]["tenant"]["in"], "header");
        let decoded: Document = serde_json::from_value(encoded).unwrap();
        assert_eq!(document, decoded);
        assert_eq!(
            document.operations[0].errors[0].status,
            ErrorStatus::Class(4)
        );
    }

    #[test]
    fn validation_finds_nested_unknown_types_and_bad_contract_links() {
        let json = r#"{
            "ir_version": "0.1",
            "info": {"title": "Example", "version": "1"},
            "types": {"Items": {"kind": "struct", "name": ["items"], "direction": "both", "fields": [
                {"wire": "items", "ident": ["items"], "ty": {"kind": "array", "item": {"ref": "Missing"}}, "presence": "required"}
            ]}},
            "operations": [{
                "id": [["items"], ["get"]], "method": "GET", "path": [],
                "responses": [{"status": 200, "content": "empty", "headers": [
                    {"wire": "X-Missing", "ty": {"ref": "MissingHeader"}, "presence": "required"}
                ]}],
                "auth": {"alternatives": [["missing_scheme"]], "optional": false},
                "semantics": {"safe": true, "idempotent": true}
            }, {
                "id": [["items"], ["get"]], "method": "GET", "path": [],
                "auth": {"alternatives": [], "optional": false},
                "semantics": {"safe": true, "idempotent": true}
            }]
        }"#;

        let document: Document = serde_json::from_str(json).unwrap();
        let errors = document.validate().unwrap_err();
        assert!(errors.iter().any(|error| matches!(error, ValidationError::UnknownType { reference, .. } if reference == "Missing")));
        assert!(errors.iter().any(|error| matches!(error, ValidationError::UnknownType { reference, .. } if reference == "MissingHeader")));
        assert!(errors.iter().any(|error| matches!(error, ValidationError::UnknownAuthScheme { scheme, .. } if scheme == "missing_scheme")));
        assert!(
            errors
                .iter()
                .any(|error| matches!(error, ValidationError::DuplicateOperation { .. }))
        );
    }

    #[test]
    fn validation_rejects_inconsistent_payload_header_and_numeric_contracts() {
        let json = r#"{
            "ir_version": "0.1",
            "info": {"title": "Example", "version": "1"},
            "types": {
                "Bad": {
                    "kind": "struct",
                    "name": ["bad"],
                    "direction": "both",
                    "fields": [{
                        "wire": "value",
                        "ident": ["value"],
                        "ty": {
                            "kind": "int",
                            "bits": 32,
                            "signed": true,
                            "wire": "number",
                            "minimum": "zero",
                            "maximum": null,
                            "multiple_of": 0
                        },
                        "presence": "required"
                    }, {
                        "wire": "bounded",
                        "ident": ["bounded"],
                        "ty": {
                            "kind": "int",
                            "bits": 8,
                            "signed": false,
                            "wire": "number",
                            "minimum": -1,
                            "maximum": 300
                        },
                        "presence": "required"
                    }]
                },
                "Empty": {
                    "kind": "enum",
                    "name": ["empty"],
                    "direction": "both",
                    "repr": {"kind": "string"},
                    "variants": []
                }
            },
            "operations": [{
                "id": [["test"]],
                "method": "POST",
                "path": [],
                "headers": [
                    {"wire": "X-Trace", "ident": ["trace"], "ty": {"kind": "string"}, "presence": "optional"},
                    {"wire": "x-trace", "ident": ["trace"], "ty": {"kind": "string"}, "presence": "optional"},
                    {"wire": "bad header", "ident": ["bad"], "ty": {"kind": "string"}, "presence": "optional"}
                ],
                "body": {"ty": {"kind": "string"}, "content": "empty"},
                "responses": [
                    {"status": 200, "content": "json"},
                    {"status": 201, "content": "empty", "body": {"kind": "string"}},
                    {"status": 202, "content": "stream"}
                ],
                "errors": [{
                    "status": 400,
                    "name": ["bad"],
                    "content": "empty",
                    "body": {"kind": "string"}
                }],
                "auth": {"alternatives": [], "optional": false},
                "semantics": {"safe": false, "idempotent": false}
            }]
        }"#;

        let document: Document = serde_json::from_str(json).unwrap();
        let errors = document.validate().unwrap_err();
        let messages = errors
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(messages.contains("integer minimum must be a number"));
        assert!(messages.contains("integer multiple_of must be greater than zero"));
        assert!(messages.contains("integer minimum is below its native range"));
        assert!(messages.contains("integer maximum is above its native range"));
        assert!(messages.contains("enum needs at least one variant"));
        assert!(messages.contains("request body content Empty"));
        assert!(messages.contains("duplicate request header x-trace"));
        assert!(messages.contains("not a valid HTTP field name"));
        assert!(messages.contains("response 200 content Json"));
        assert!(messages.contains("response 201 content Empty"));
        assert!(messages.contains("0 stream descriptions and 1 stream responses"));
        assert!(messages.contains("error content Empty"));
    }

    #[test]
    fn status_defaults_and_classes_keep_their_wire_form() {
        let status: ErrorStatus = serde_json::from_str("\"default\"").unwrap();
        assert_eq!(serde_json::to_string(&status).unwrap(), "\"default\"");
        let status: ErrorStatus = serde_json::from_str("\"5xx\"").unwrap();
        assert_eq!(serde_json::to_string(&status).unwrap(), "\"5xx\"");
    }

    #[test]
    fn checked_in_schema_matches_the_rust_model() {
        let generated = serde_json::to_value(schemars::schema_for!(Document)).unwrap();
        let checked_in: Value =
            serde_json::from_str(include_str!("../schema/ir.schema.json")).unwrap();
        assert_eq!(generated, checked_in);
    }

    #[test]
    fn canonical_json_ignores_order_of_unordered_contract_members() {
        let document: Document =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        let mut shuffled = document.clone();
        shuffled.operations.reverse();
        if let TypeKind::Struct { fields, .. } =
            &mut shuffled.types.get_mut("User").unwrap().definition
        {
            fields.reverse();
        }

        assert_ne!(
            serde_json::to_string(&document).unwrap(),
            serde_json::to_string(&shuffled).unwrap()
        );
        assert_eq!(
            document.to_canonical_json().unwrap(),
            shuffled.to_canonical_json().unwrap()
        );
        assert_eq!(shuffled.operations[0].id[0], ["events"]);
    }
}
