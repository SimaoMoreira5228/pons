use std::collections::BTreeMap;

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

pub(crate) fn check_key(key: &str) -> Result<(), String> {
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

pub(crate) fn check(owner: &str, ext: &BTreeMap<String, Value>, errors: &mut Vec<ValidationError>) {
    for key in ext.keys() {
        if let Err(refusal) = check_key(key) {
            errors.push(ValidationError::InvalidValue {
                path: owner.to_owned(),
                message: refusal,
            });
        }
    }
}
