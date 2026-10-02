use std::any::type_name;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, OnceLock};

use pons_core::{Schema, TypeCx};
use pons_ir::{
    DecimalWire, Direction, Document, InlineType, IntegerWire, TypeKind, TypeRef, Violation,
    validate_value,
};
use serde_json::Value;

use crate::ViolationReport;

pub(crate) fn request_contract<T: Schema>(
    extensions: &axum::http::Extensions,
) -> (Arc<Document>, TypeRef) {
    let document = extensions
        .get::<Arc<Document>>()
        .expect("a request extractor needs the document Router::into_axum installs")
        .clone();
    (document, request_type::<T>())
}

fn request_type<T: Schema>() -> TypeRef {
    static REQUEST_TYPES: OnceLock<Mutex<BTreeMap<&'static str, TypeRef>>> = OnceLock::new();
    let mut types = REQUEST_TYPES
        .get_or_init(Default::default)
        .lock()
        .expect("the request type registry is never poisoned");
    types
        .entry(type_name::<T>())
        .or_insert_with(|| T::schema(&mut TypeCx::new(), Direction::Input))
        .clone()
}

pub(crate) fn check(
    document: &Document,
    ty: &TypeRef,
    value: &Value,
) -> Result<(), ViolationReport> {
    validate_value(document, ty, Direction::Input, value).map_err(ViolationReport::rejected)
}

pub(crate) fn check_pairs(
    document: &Document,
    ty: &TypeRef,
    pairs: Vec<(String, String)>,
) -> Result<(), ViolationReport> {
    let (value, mut violations) = object_value(document, ty, pairs);
    if let Err(found) = validate_value(document, ty, Direction::Input, &value) {
        violations.extend(found);
    }
    match violations.is_empty() {
        true => Ok(()),
        false => Err(ViolationReport::rejected(violations)),
    }
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

pub(crate) fn object_value(
    document: &Document,
    ty: &TypeRef,
    pairs: Vec<(String, String)>,
) -> (Value, Vec<Violation>) {
    let mut declared = Vec::new();
    members(document, ty, &mut declared, &mut Vec::new());
    let mut entries = serde_json::Map::new();
    let mut violations = Vec::new();
    for (wire, text) in pairs {
        if entries.contains_key(&wire) {
            violations.push(Violation::DuplicateItem {
                path: pointer(&wire),
                item: 1,
                first: 0,
            });
            continue;
        }
        let value = match declared.iter().find(|(name, _)| name == &wire) {
            Some((_, field)) => token_value(document, field, &text),
            None => Value::String(text),
        };
        entries.insert(wire, value);
    }
    (Value::Object(entries), violations)
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
    let Some(TypeKind::Struct {
        fields, flattened, ..
    }) = document.types.get(&named.id).map(|found| &found.definition)
    else {
        return;
    };
    seen.push(named.id.clone());
    found.extend(fields.iter().map(|field| (field.wire.clone(), &field.ty)));
    flattened
        .iter()
        .for_each(|ty| members(document, ty, found, seen));
    seen.pop();
}

pub(crate) fn token_value(document: &Document, ty: &TypeRef, text: &str) -> Value {
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

fn pointer(wire: &str) -> String {
    format!("/{}", wire.replace('~', "~0").replace('/', "~1"))
}

fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    if !bytes.contains(&b'%') && !bytes.contains(&b'+') {
        return text.to_owned();
    }
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn contract() -> Document {
        serde_json::from_value(json!({
            "ir_version": "0.2",
            "info": {"title": "Validation", "version": "1"},
            "types": {
                "Filter#input": {
                    "name": ["filter"],
                    "direction": "input",
                    "kind": "struct",
                    "openness": "reject",
                    "fields": [
                        {
                            "wire": "slug",
                            "ident": ["slug"],
                            "ty": {"kind": "string", "pattern": "^[a-z]+$"},
                            "presence": "required",
                        },
                        {
                            "wire": "limit",
                            "ident": ["limit"],
                            "ty": {
                                "kind": "int",
                                "bits": 16,
                                "signed": false,
                                "wire": "number",
                                "minimum": 1,
                                "maximum": 10,
                            },
                            "presence": "optional",
                        },
                    ],
                },
            },
        }))
        .unwrap()
    }

    fn filter() -> TypeRef {
        TypeRef::Named(pons_ir::NamedRef {
            id: "Filter#input".into(),
            variant: None,
        })
    }

    fn found(pairs: &[(&str, &str)]) -> Vec<String> {
        let contract = contract();
        let pairs = pairs
            .iter()
            .map(|(wire, text)| ((*wire).to_owned(), (*text).to_owned()))
            .collect();
        check_pairs(&contract, &filter(), pairs)
            .err()
            .map(|report| {
                report
                    .violations
                    .iter()
                    .map(|violation| format!("{}: {}", violation.path, violation.reason))
                    .collect()
            })
            .unwrap_or_default()
    }

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
        assert_eq!(
            decode_pairs("bad=%zz"),
            [("bad".to_owned(), "%zz".to_owned())]
        );
    }

    #[test]
    fn text_tokens_are_read_in_the_wire_form_the_contract_publishes() {
        let contract = contract();
        let number = TypeRef::Inline(InlineType::Int {
            bits: 16,
            signed: false,
            wire: IntegerWire::Number,
            minimum: None,
            maximum: None,
            multiple_of: None,
        });
        let text = TypeRef::Inline(InlineType::String {
            format: None,
            min_length: None,
            max_length: None,
            pattern: None,
        });
        assert_eq!(token_value(&contract, &number, "7"), json!(7));
        assert_eq!(token_value(&contract, &number, "seven"), json!("seven"));
        assert_eq!(token_value(&contract, &text, "7"), json!("7"));
        assert_eq!(token_value(&contract, &filter(), "7"), json!("7"));
    }

    #[test]
    fn a_text_token_pair_is_checked_against_the_published_query_type() {
        assert_eq!(found(&[("slug", "ada")]), Vec::<String>::new());
        assert_eq!(
            found(&[("slug", "Ada")]),
            ["/slug: string does not match the pattern ^[a-z]+$"]
        );
        assert_eq!(
            found(&[("slug", "ada"), ("limit", "99")]),
            ["/limit: integer is above its maximum of 10"]
        );
        assert_eq!(
            found(&[("limit", "1")]),
            ["/slug: missing required field slug"]
        );
        assert_eq!(
            found(&[("slug", "ada"), ("page", "2")]),
            ["/page: unknown property page"]
        );
        assert_eq!(
            found(&[("slug", "ada"), ("limit", "1"), ("limit", "2")]),
            ["/limit: array item 1 repeats item 0"]
        );
    }
}
