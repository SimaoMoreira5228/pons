use std::any::type_name;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, OnceLock};

use actix_web::HttpRequest;
use pons_core::{Schema, TypeCx};
use pons_ir::{
    DecimalWire, Direction, Document, InlineType, IntegerWire, TypeKind, TypeRef, Violation,
    validate_value,
};
use serde_json::Value;

use crate::ViolationReport;

const INPUT: &str = "input";

pub(crate) fn request_contract<T: Schema>(
    request: &HttpRequest,
    carrier: &str,
) -> Result<(Arc<Document>, TypeRef), ViolationReport> {
    let Some(document) = request.app_data::<actix_web::web::Data<Arc<Document>>>() else {
        return Err(violated(
            "a request carrying the document Router::into_actix installs",
            "a request carrying none".to_owned(),
        ));
    };
    Ok((document.get_ref().clone(), request_type::<T>(carrier)))
}

fn request_type<T: Schema>(carrier: &str) -> TypeRef {
    static REQUEST_TYPES: OnceLock<Mutex<BTreeMap<(String, String), TypeRef>>> = OnceLock::new();
    let (id, role) = (type_name::<T>(), carrier.to_owned());
    let key = (id.to_owned(), role.clone());
    let mut types = REQUEST_TYPES
        .get_or_init(Default::default)
        .lock()
        .expect("the request type registry is never poisoned");
    types
        .entry(key)
        .or_insert_with(|| {
            let mut cx = TypeCx::new();
            match role.as_str() {
                INPUT => T::schema(&mut cx, Direction::Input),
                role => {
                    cx.register_as(format!("{id}#{role}"), |cx| T::schema(cx, Direction::Input))
                }
            }
        })
        .clone()
}

pub(crate) fn declared_media_type(
    document: &Document,
    request: &HttpRequest,
    default: &str,
) -> String {
    let Some(method) = method(request.method()) else {
        return default.to_owned();
    };
    document
        .operations
        .iter()
        .filter(|operation| {
            operation.method == method && path_matches(&operation.path, request.path())
        })
        .max_by_key(|operation| specificity(&operation.path))
        .and_then(|operation| operation.body.as_ref()?.media.as_ref())
        .map(ToString::to_string)
        .unwrap_or_else(|| default.to_owned())
}

fn specificity(segments: &[pons_ir::PathSegment]) -> (usize, Vec<u8>) {
    (
        segments.len(),
        segments
            .iter()
            .map(|segment| match segment {
                pons_ir::PathSegment::Literal { .. } => 1,
                pons_ir::PathSegment::Param { .. } => 0,
            })
            .collect(),
    )
}

fn method(method: &actix_web::http::Method) -> Option<pons_ir::Method> {
    Some(match method.as_str() {
        "GET" => pons_ir::Method::Get,
        "POST" => pons_ir::Method::Post,
        "PUT" => pons_ir::Method::Put,
        "PATCH" => pons_ir::Method::Patch,
        "DELETE" => pons_ir::Method::Delete,
        "HEAD" => pons_ir::Method::Head,
        "OPTIONS" => pons_ir::Method::Options,
        _ => return None,
    })
}

fn path_matches(segments: &[pons_ir::PathSegment], path: &str) -> bool {
    let mut parts = path_segments(path).into_iter();
    segments
        .iter()
        .enumerate()
        .all(|(index, segment)| match (segment, parts.next()) {
            (pons_ir::PathSegment::Param { .. }, Some(part)) => {
                index + 1 < segments.len() || !part.is_empty()
            }
            (pons_ir::PathSegment::Literal { lit }, Some(part)) => lit == part,
            _ => false,
        })
        && parts.next().is_none()
}

fn path_segments(path: &str) -> Vec<&str> {
    match path.strip_prefix('/').unwrap_or(path) {
        "" => Vec::new(),
        rest => rest.split('/').collect(),
    }
}

pub(crate) fn check(
    document: &Document,
    ty: &TypeRef,
    value: &Value,
) -> Result<(), ViolationReport> {
    validate_value(document, ty, Direction::Input, value).map_err(ViolationReport::rejected)
}

pub(crate) fn read_pairs<T: serde::de::DeserializeOwned>(
    document: &Document,
    ty: &TypeRef,
    pairs: Vec<(String, String)>,
) -> Result<T, ViolationReport> {
    let (value, mut violations) = object_value(document, ty, pairs);
    if let Err(found) = validate_value(document, ty, Direction::Input, &value) {
        violations.extend(found);
    }
    if !violations.is_empty() {
        return Err(ViolationReport::rejected(violations));
    }
    serde_json::from_value(value)
        .map_err(|error| violated("a value the request type reads", error.to_string()))
}

pub(crate) fn violated(expected: &str, found: String) -> ViolationReport {
    ViolationReport::rejected(vec![Violation::Type {
        path: String::new(),
        expected: expected.to_owned(),
        found,
    }])
}

pub(crate) fn decode_pairs(text: &str) -> Vec<(String, String)> {
    text.split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
            (decode(name), decode(value))
        })
        .collect()
}

fn decode(text: &str) -> String {
    decode_token(text, true)
}

fn decode_token(text: &str, plus_as_space: bool) -> String {
    let bytes = text.as_bytes();
    if !bytes.contains(&b'%') && !(plus_as_space && bytes.contains(&b'+')) {
        return text.to_owned();
    }
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' if plus_as_space => {
                decoded.push(b' ');
                index += 1;
            }
            b'%' if index + 2 < bytes.len() => {
                match (nibble(bytes[index + 1]), nibble(bytes[index + 2])) {
                    (Some(high), Some(low)) => {
                        decoded.push(high << 4 | low);
                        index += 3;
                    }
                    _ => {
                        decoded.push(b'%');
                        index += 1;
                    }
                }
            }
            byte => {
                decoded.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

fn nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

pub(crate) fn object_value(
    document: &Document,
    ty: &TypeRef,
    pairs: Vec<(String, String)>,
) -> (Value, Vec<Violation>) {
    let mut declared = Vec::new();
    members(document, ty, &mut declared, &mut Vec::new());
    let mut entries = serde_json::Map::new();
    let mut violations = Vec::new();
    for (wire, texts) in grouped(pairs) {
        let field = declared
            .iter()
            .find(|(name, _)| name == &wire)
            .map(|(_, field)| *field);
        let value = match field.map(|field| (field, array_item(document, field))) {
            Some((field, Some(_))) => array_value(document, field, &texts),
            Some((field, _)) if texts.len() > 1 => {
                violations.push(Violation::Type {
                    path: pointer(&wire),
                    expected: "one value for a field that is not an array".to_owned(),
                    found: format!("{} values", texts.len()),
                });
                token_value(document, field, &texts[texts.len() - 1])
            }
            Some((field, _)) => token_value(document, field, &texts[0]),
            None => Value::String(texts[texts.len() - 1].clone()),
        };
        entries.insert(wire, value);
    }
    (Value::Object(entries), violations)
}

fn array_value(document: &Document, field: &TypeRef, texts: &[String]) -> Value {
    let item = array_item(document, field).expect("an array field spells its item");
    Value::Array(
        texts
            .iter()
            .flat_map(|text| match spells_array(text) {
                Some(items) => items,
                None => vec![item_value(document, item, text)],
            })
            .collect(),
    )
}

fn item_value(document: &Document, item: &TypeRef, text: &str) -> Value {
    if carries_json(document, item) {
        return serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.to_owned()));
    }
    token_value(document, item, text)
}

fn spells_array(text: &str) -> Option<Vec<Value>> {
    match serde_json::from_str::<Value>(text) {
        Ok(Value::Array(items)) => Some(items),
        _ => None,
    }
}

fn carries_json(document: &Document, ty: &TypeRef) -> bool {
    match ty {
        TypeRef::Inline(InlineType::Any | InlineType::Map { .. } | InlineType::Tuple { .. }) => {
            true
        }
        TypeRef::Inline(InlineType::Nullable { inner }) => carries_json(document, inner),
        TypeRef::Named(named) => match document.types.get(&named.id).map(|found| &found.definition)
        {
            Some(TypeKind::Struct { .. }) => true,
            Some(TypeKind::Newtype { inner }) => carries_json(document, inner),
            _ => false,
        },
        _ => false,
    }
}

fn grouped(pairs: Vec<(String, String)>) -> Vec<(String, Vec<String>)> {
    let mut groups: Vec<(String, Vec<String>)> = Vec::new();
    for (wire, text) in pairs {
        match groups.iter_mut().find(|(name, _)| name == &wire) {
            Some((_, texts)) => texts.push(text),
            None => groups.push((wire, vec![text])),
        }
    }
    groups
}

pub(crate) fn path_value(document: &Document, ty: &TypeRef, pairs: Vec<(String, String)>) -> Value {
    match shape(document, ty) {
        Shape::Struct => object_value(document, ty, pairs).0,
        Shape::Tuple(items) => Value::Array(
            pairs
                .iter()
                .enumerate()
                .map(|(index, (_, text))| match items.get(index) {
                    Some(item) => token_value(document, item, text),
                    None => Value::String(text.clone()),
                })
                .collect(),
        ),
        Shape::Single => match pairs.first() {
            Some((_, text)) => token_value(document, ty, text),
            None => Value::Null,
        },
    }
}

enum Shape<'a> {
    Struct,
    Tuple(&'a [TypeRef]),
    Single,
}

fn shape<'a>(document: &'a Document, ty: &'a TypeRef) -> Shape<'a> {
    match ty {
        TypeRef::Inline(InlineType::Tuple { items }) => Shape::Tuple(items),
        TypeRef::Named(named) => match document.types.get(&named.id).map(|found| &found.definition)
        {
            Some(TypeKind::Newtype { inner }) => shape(document, inner),
            Some(TypeKind::Struct { .. }) => Shape::Struct,
            _ => Shape::Single,
        },
        _ => Shape::Single,
    }
}

fn members<'a>(
    document: &'a Document,
    ty: &'a TypeRef,
    found: &mut Vec<(String, &'a TypeRef)>,
    seen: &mut Vec<String>,
) {
    let TypeRef::Named(named) = ty else {
        return;
    };
    if seen.contains(&named.id) {
        return;
    }
    let Some(definition) = document.types.get(&named.id).map(|found| &found.definition) else {
        return;
    };
    seen.push(named.id.clone());
    match definition {
        TypeKind::Newtype { inner } => members(document, inner, found, seen),
        TypeKind::Struct {
            fields, flattened, ..
        } => {
            found.extend(fields.iter().map(|field| (field.wire.clone(), &field.ty)));
            flattened
                .iter()
                .for_each(|ty| members(document, ty, found, seen));
        }
        TypeKind::Enum { .. } => {}
    }
    seen.pop();
}

pub(crate) fn token_value(document: &Document, ty: &TypeRef, text: &str) -> Value {
    if let Some(item) = array_item(document, ty) {
        return Value::Array(match serde_json::from_str::<Value>(text) {
            Ok(Value::Array(items)) => items,
            _ => vec![token_value(document, item, text)],
        });
    }
    match ty {
        TypeRef::Named(named) => {
            match document.types.get(&named.id).map(|found| &found.definition) {
                Some(TypeKind::Newtype { inner }) => token_value(document, inner, text),
                _ => Value::String(text.to_owned()),
            }
        }
        TypeRef::Inline(InlineType::Nullable { inner }) => token_value(document, inner, text),
        TypeRef::Inline(InlineType::Bool) => match text {
            "true" => Value::Bool(true),
            "false" => Value::Bool(false),
            _ => Value::String(text.to_owned()),
        },
        TypeRef::Inline(
            InlineType::Int {
                wire: IntegerWire::Number,
                ..
            }
            | InlineType::Float { .. }
            | InlineType::Decimal {
                wire: DecimalWire::Number,
            },
        ) => match serde_json::from_str::<Value>(text) {
            Ok(number @ Value::Number(_)) => number,
            _ => Value::String(text.to_owned()),
        },
        _ => Value::String(text.to_owned()),
    }
}

fn array_item<'a>(document: &'a Document, ty: &'a TypeRef) -> Option<&'a TypeRef> {
    match ty {
        TypeRef::Named(named) => match document.types.get(&named.id).map(|found| &found.definition)
        {
            Some(TypeKind::Newtype { inner }) => array_item(document, inner),
            _ => None,
        },
        TypeRef::Inline(InlineType::Nullable { inner }) => array_item(document, inner),
        TypeRef::Inline(InlineType::Array { item, .. }) => Some(item),
        _ => None,
    }
}

fn pointer(wire: &str) -> String {
    format!("/{}", wire.replace('~', "~0").replace('/', "~1"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn urlencoded_pairs_are_decoded_like_a_form_submission() {
        assert_eq!(
            decode_pairs("name=Ada+Lovelace&tag=a%2Fb&flag&empty="),
            [
                ("name".to_owned(), "Ada Lovelace".to_owned()),
                ("tag".to_owned(), "a/b".to_owned()),
                ("flag".to_owned(), String::new()),
                ("empty".to_owned(), String::new()),
            ]
        );
        assert_eq!(decode_pairs(""), Vec::<(String, String)>::new());
    }

    fn empty_document() -> Document {
        Document {
            ir_version: pons_ir::IR_VERSION.to_owned(),
            info: pons_ir::Info {
                title: String::new(),
                version: String::new(),
                description: None,
                servers: Vec::new(),
                ext: Default::default(),
            },
            types: Default::default(),
            auth_schemes: Default::default(),
            common_errors: Vec::new(),
            operations: Vec::new(),
            ext: Default::default(),
            diagnostics: Vec::new(),
        }
    }

    #[test]
    fn number_tokens_are_read_as_numbers_and_text_stays_text() {
        let number = TypeRef::Inline(InlineType::Int {
            bits: 16,
            signed: false,
            wire: IntegerWire::Number,
            minimum: None,
            maximum: None,
            multiple_of: None,
        });
        assert_eq!(token_value(&empty_document(), &number, "7"), json!(7));
        assert_eq!(
            token_value(&empty_document(), &number, "seven"),
            json!("seven")
        );
    }
}
