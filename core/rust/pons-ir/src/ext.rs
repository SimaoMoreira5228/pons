use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use crate::ValidationError;

pub const RESERVED_PREFIX: &str = "x-pons";

pub const ALIAS: &str = "com.pons.alias/v1";
pub const OPEN_VARIANT: &str = "com.pons.open_variant/v1";

pub(crate) fn catch_all(variant: &crate::Variant) -> bool {
    matches!(variant.ext.get(OPEN_VARIANT), Some(Value::Bool(true)))
}

fn is_label(label: &str) -> bool {
    !label.is_empty()
        && !label.starts_with('-')
        && !label.ends_with('-')
        && label.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-' || byte == b'_'
        })
}

fn is_version(segment: &str) -> bool {
    let Some(digits) = segment.strip_prefix('v') else {
        return false;
    };
    !digits.is_empty()
        && digits.bytes().all(|byte| byte.is_ascii_digit())
        && (digits.len() == 1 || !digits.starts_with('0'))
}

pub fn validate_extension_key(key: &str) -> Result<(), String> {
    if key.to_ascii_lowercase().starts_with(RESERVED_PREFIX) {
        return Err(format!(
            "extension key {key:?} starts with {RESERVED_PREFIX:?}, which this project reserves for itself"
        ));
    }
    let mut segments = key.split('/');
    let (name, version) = (segments.next(), segments.next());
    if segments.next().is_some()
        || !name.is_some_and(is_namespaced)
        || !version.is_some_and(is_version)
    {
        return Err(format!(
            "extension key {key:?} is not shaped reverse-dns.namespace/vN"
        ));
    }
    Ok(())
}

fn is_namespaced(name: &str) -> bool {
    let labels: Vec<_> = name.split('.').collect();
    labels.len() >= 3 && labels.iter().all(|label| is_label(label))
}

pub(crate) fn document_namespaces(document: &crate::Document) -> BTreeSet<String> {
    let mut namespaces = BTreeSet::new();
    collect(&document.ext, &mut namespaces);
    collect(&document.info.ext, &mut namespaces);
    document
        .diagnostics
        .iter()
        .for_each(|diagnostic| collect(&diagnostic.ext, &mut namespaces));
    document.types.values().for_each(|definition| {
        collect(&definition.ext, &mut namespaces);
        definition
            .diagnostics
            .iter()
            .for_each(|diagnostic| collect(&diagnostic.ext, &mut namespaces));
        match &definition.definition {
            crate::TypeKind::Struct {
                fields,
                flattened,
                openness,
            } => {
                fields
                    .iter()
                    .for_each(|field| collect_field(field, &mut namespaces));
                flattened
                    .iter()
                    .for_each(|ty| collect_type_ref(ty, &mut namespaces));
                if let crate::ObjectOpenness::PreserveTyped { value } = openness {
                    collect_type_ref(value, &mut namespaces);
                }
            }
            crate::TypeKind::Enum { variants, .. } => variants.iter().for_each(|variant| {
                collect(&variant.ext, &mut namespaces);
                variant
                    .diagnostics
                    .iter()
                    .for_each(|diagnostic| collect(&diagnostic.ext, &mut namespaces));
                match &variant.payload {
                    crate::VariantPayload::Unit => {}
                    crate::VariantPayload::Newtype { ty } => {
                        collect_type_ref(ty, &mut namespaces);
                    }
                    crate::VariantPayload::Tuple { items } => items
                        .iter()
                        .for_each(|item| collect_type_ref(item, &mut namespaces)),
                    crate::VariantPayload::Struct { fields } => fields
                        .iter()
                        .for_each(|field| collect_field(field, &mut namespaces)),
                }
            }),
            crate::TypeKind::Newtype { inner } => collect_type_ref(inner, &mut namespaces),
        }
    });
    document.auth_schemes.values().for_each(|scheme| {
        if let crate::AuthScheme::Custom { ext, .. } = scheme {
            collect(ext, &mut namespaces);
        }
        if let crate::AuthScheme::OAuth2 { flows } = scheme {
            flows
                .iter()
                .for_each(|flow| collect(&flow.ext, &mut namespaces));
        }
    });
    document
        .common_errors
        .iter()
        .for_each(|error| collect_error(error, &mut namespaces));
    document.operations.iter().for_each(|operation| {
        collect(&operation.ext, &mut namespaces);
        operation
            .diagnostics
            .iter()
            .for_each(|diagnostic| collect(&diagnostic.ext, &mut namespaces));
        collect(&operation.auth.ext, &mut namespaces);
        if let Some(provides) = &operation.provides {
            collect(&provides.ext, &mut namespaces);
        }
        if let Some(query) = &operation.query {
            collect_type_ref(query, &mut namespaces);
        }
        operation.headers.iter().for_each(|header| {
            collect(&header.ext, &mut namespaces);
            collect_type_ref(&header.ty, &mut namespaces);
        });
        if let Some(body) = &operation.body {
            collect(&body.ext, &mut namespaces);
            collect_type_ref(&body.ty, &mut namespaces);
        }
        operation.responses.iter().for_each(|response| {
            collect(&response.ext, &mut namespaces);
            if let Some(body) = &response.body {
                collect_type_ref(body, &mut namespaces);
            }
            response.headers.iter().for_each(|header| {
                collect(&header.ext, &mut namespaces);
                collect_type_ref(&header.ty, &mut namespaces);
            });
        });
        operation
            .errors
            .iter()
            .for_each(|error| collect_error(error, &mut namespaces));
    });
    namespaces
}

fn collect_field(field: &crate::Field, namespaces: &mut BTreeSet<String>) {
    collect(&field.ext, namespaces);
    field
        .diagnostics
        .iter()
        .for_each(|diagnostic| collect(&diagnostic.ext, namespaces));
    collect_type_ref(&field.ty, namespaces);
}

fn collect_error(error: &crate::ErrorResponse, namespaces: &mut BTreeSet<String>) {
    collect(&error.ext, namespaces);
    if let Some(body) = &error.body {
        collect_type_ref(body, namespaces);
    }
    error.headers.iter().for_each(|header| {
        collect(&header.ext, namespaces);
        collect_type_ref(&header.ty, namespaces);
    });
}

fn collect_type_ref(ty: &crate::TypeRef, namespaces: &mut BTreeSet<String>) {
    let crate::TypeRef::Inline(inline) = ty else {
        return;
    };
    match inline {
        crate::InlineType::Opaque { ext, .. } => collect(ext, namespaces),
        crate::InlineType::Nullable { inner } => collect_type_ref(inner, namespaces),
        crate::InlineType::Array { item, .. } => collect_type_ref(item, namespaces),
        crate::InlineType::Map { key, value } => {
            collect_type_ref(key, namespaces);
            collect_type_ref(value, namespaces);
        }
        crate::InlineType::Tuple { items } => items
            .iter()
            .for_each(|item| collect_type_ref(item, namespaces)),
        _ => {}
    }
}

fn collect(ext: &BTreeMap<String, Value>, namespaces: &mut BTreeSet<String>) {
    namespaces.extend(ext.keys().cloned());
}

pub(crate) fn check(owner: &str, ext: &BTreeMap<String, Value>, errors: &mut Vec<ValidationError>) {
    for key in ext.keys() {
        if let Err(refusal) = validate_extension_key(key) {
            errors.push(ValidationError::InvalidValue {
                path: owner.to_owned(),
                message: refusal,
            });
        }
    }
}
