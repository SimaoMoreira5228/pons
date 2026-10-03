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

const INPUT: &str = "input";

pub(crate) fn request_contract<T: Schema>(
    extensions: &axum::http::Extensions,
    carrier: &str,
) -> Result<(Arc<Document>, TypeRef), ViolationReport> {
    let Some(document) = extensions.get::<Arc<Document>>() else {
        return Err(violated(
            "a request carrying the document Router::into_axum installs",
            "a request carrying none".to_owned(),
        ));
    };
    Ok((document.clone(), request_type::<T>(carrier)))
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

pub(crate) fn declared_media_type<'a>(
    document: &'a Document,
    request: &axum::extract::Request,
    default: &'a str,
) -> &'a str {
    let Some(method) = method(request.method()) else {
        return default;
    };
    document
        .operations
        .iter()
        .filter(|operation| {
            operation.method == method && path_matches(&operation.path, request.uri().path())
        })
        .min_by_key(|operation| resolution_order(&operation.path))
        .and_then(|operation| operation.body.as_ref()?.media_type.as_deref())
        .unwrap_or(default)
}

fn resolution_order(segments: &[pons_ir::PathSegment]) -> Vec<u8> {
    segments
        .iter()
        .map(|segment| match segment {
            pons_ir::PathSegment::Literal { .. } => 0,
            pons_ir::PathSegment::Param { .. } => 1,
        })
        .collect()
}

fn method(method: &axum::http::Method) -> Option<pons_ir::Method> {
    Some(match *method {
        axum::http::Method::GET => pons_ir::Method::Get,
        axum::http::Method::POST => pons_ir::Method::Post,
        axum::http::Method::PUT => pons_ir::Method::Put,
        axum::http::Method::PATCH => pons_ir::Method::Patch,
        axum::http::Method::DELETE => pons_ir::Method::Delete,
        axum::http::Method::HEAD => pons_ir::Method::Head,
        axum::http::Method::OPTIONS => pons_ir::Method::Options,
        _ => return None,
    })
}

fn path_matches(segments: &[pons_ir::PathSegment], path: &str) -> bool {
    let mut parts = path.split('/').filter(|part| !part.is_empty());
    segments
        .iter()
        .all(|segment| match (segment, parts.next()) {
            (pons_ir::PathSegment::Param { .. }, Some(_)) => true,
            (pons_ir::PathSegment::Literal { lit }, Some(part)) => lit == part,
            _ => false,
        })
        && parts.next().is_none()
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
                        {
                            "wire": "tags",
                            "ident": ["tags"],
                            "ty": {
                                "kind": "array",
                                "item": {"kind": "string", "pattern": "^[a-z]+$"},
                            },
                            "presence": "optional",
                        },
                    ],
                },
                "Page#input": {
                    "name": ["page"],
                    "direction": "input",
                    "kind": "struct",
                    "openness": "reject",
                    "fields": [
                        {
                            "wire": "cursor",
                            "ident": ["cursor"],
                            "ty": {
                                "kind": "int",
                                "bits": 32,
                                "signed": false,
                                "wire": "number",
                                "maximum": 100,
                            },
                            "presence": "required",
                        },
                    ],
                },
                "Wrapped#input": {
                    "name": ["wrapped"],
                    "direction": "input",
                    "kind": "newtype",
                    "inner": {"ref": "Filter#input"},
                },
                "Paged#input": {
                    "name": ["paged"],
                    "direction": "input",
                    "kind": "struct",
                    "openness": "reject",
                    "flattened": [{"ref": "WrappedPage#input"}],
                },
                "WrappedPage#input": {
                    "name": ["wrapped", "page"],
                    "direction": "input",
                    "kind": "newtype",
                    "inner": {"ref": "Page#input"},
                },
                "Objects#input": {
                    "name": ["objects"],
                    "direction": "input",
                    "kind": "struct",
                    "openness": "reject",
                    "fields": [
                        {
                            "wire": "payload",
                            "ident": ["payload"],
                            "ty": {"kind": "array", "item": {"kind": "any"}},
                            "presence": "required",
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

    fn named(id: &str) -> TypeRef {
        TypeRef::Named(pons_ir::NamedRef {
            id: id.into(),
            variant: None,
        })
    }

    fn reported(
        contract: &Document,
        ty: &TypeRef,
        pairs: &[(&str, &str)],
    ) -> Result<Value, String> {
        let pairs = pairs
            .iter()
            .map(|(wire, text)| ((*wire).to_owned(), (*text).to_owned()))
            .collect();
        read_pairs::<Value>(contract, ty, pairs).map_err(|report| {
            report
                .violations
                .iter()
                .map(|violation| format!("{}: {}", violation.path, violation.reason))
                .collect::<Vec<_>>()
                .join(", ")
        })
    }

    fn found(pairs: &[(&str, &str)]) -> Vec<String> {
        let contract = contract();
        let pairs = pairs
            .iter()
            .map(|(wire, text)| ((*wire).to_owned(), (*text).to_owned()))
            .collect();
        read_pairs::<Value>(&contract, &filter(), pairs)
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

    fn segments(path: &str) -> Vec<pons_ir::PathSegment> {
        path.split('/')
            .map(|segment| match segment.strip_prefix('{') {
                Some(param) => pons_ir::PathSegment::Param {
                    param: param.trim_end_matches('}').to_owned(),
                    ty: TypeRef::Inline(InlineType::String {
                        format: None,
                        min_length: None,
                        max_length: None,
                        pattern: None,
                    }),
                },
                None => pons_ir::PathSegment::Literal {
                    lit: segment.to_owned(),
                },
            })
            .collect()
    }

    #[test]
    fn a_path_parameter_matches_one_segment_and_no_more() {
        let assets = segments("assets/{id}");
        assert!(path_matches(&assets, "/assets/1"));
        assert!(
            path_matches(&assets, "/assets/1/"),
            "empty segments are none"
        );
        assert!(
            !path_matches(&assets, "/assets/1/raw"),
            "a parameter covers one segment, so a deeper path is another operation's"
        );
        assert!(!path_matches(&assets, "/assets"));
        assert!(!path_matches(&assets, "/other/1"));
        assert!(
            path_matches(&[], "/"),
            "the root operation matches only the root"
        );

        let raw = segments("assets/{id}/raw");
        assert!(path_matches(&raw, "/assets/1/raw"));
        assert!(!path_matches(&raw, "/assets/1"));
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
            ["/limit: expected one value for a field that is not an array, found 2 values"]
        );
    }

    #[test]
    fn an_array_field_reads_every_value_the_request_spellings_carry() {
        assert_eq!(
            found(&[("slug", "ada"), ("tags", "[\"x\",\"y\"]")]),
            Vec::<String>::new()
        );

        assert_eq!(
            found(&[("slug", "ada"), ("tags", "x"), ("tags", "y")]),
            Vec::<String>::new()
        );
        assert_eq!(
            found(&[("slug", "ada"), ("tags", "x"), ("tags", "Y")]),
            ["/tags/1: string does not match the pattern ^[a-z]+$"]
        );
        assert_eq!(
            found(&[("slug", "ada"), ("tags", "[\"x\",\"Y\"]")]),
            ["/tags/1: string does not match the pattern ^[a-z]+$"]
        );
    }

    #[test]
    fn a_carrier_newtype_publishes_the_members_of_the_struct_it_wraps() {
        let contract = contract();
        assert_eq!(
            reported(
                &contract,
                &named("Wrapped#input"),
                &[("slug", "ada"), ("limit", "2")]
            ),
            Ok(json!({"slug": "ada", "limit": 2}))
        );
        assert_eq!(
            reported(
                &contract,
                &named("Wrapped#input"),
                &[("slug", "ada"), ("limit", "99")]
            ),
            Err("/limit: integer is above its maximum of 10".to_owned())
        );
    }

    #[test]
    fn a_newtype_wrapped_flattened_member_publishes_its_own_members() {
        let contract = contract();
        assert_eq!(
            reported(&contract, &named("Paged#input"), &[("cursor", "7")]),
            Ok(json!({"cursor": 7}))
        );
        assert_eq!(
            reported(&contract, &named("Paged#input"), &[("cursor", "900")]),
            Err("/cursor: integer is above its maximum of 100".to_owned())
        );
        assert_eq!(
            reported(&contract, &named("Paged#input"), &[("cursor", "seven")]),
            Err("/cursor: expected a number, found a string".to_owned())
        );
    }

    #[test]
    fn every_published_method_is_read_and_no_other_is_folded_onto_one() {
        for (sent, expected) in [
            ("GET", pons_ir::Method::Get),
            ("POST", pons_ir::Method::Post),
            ("PUT", pons_ir::Method::Put),
            ("PATCH", pons_ir::Method::Patch),
            ("DELETE", pons_ir::Method::Delete),
            ("HEAD", pons_ir::Method::Head),
            ("OPTIONS", pons_ir::Method::Options),
        ] {
            let sent = axum::http::Method::from_bytes(sent.as_bytes()).unwrap();
            assert_eq!(method(&sent), Some(expected), "{sent} is published");
        }
        for sent in [
            axum::http::Method::CONNECT,
            axum::http::Method::TRACE,
            axum::http::Method::from_bytes(b"PROPFIND").unwrap(),
        ] {
            assert_eq!(
                method(&sent),
                None,
                "{sent} is routable but no operation can publish it, so it reads no operation's body"
            );
        }
    }

    #[test]
    fn a_repeated_key_reads_the_json_a_value_carries() {
        let contract = contract();
        let value = json!({"payload": [{"a": 1}, {"b": 2}]});
        assert_eq!(
            reported(
                &contract,
                &named("Objects#input"),
                &[("payload", r#"[{"a":1},{"b":2}]"#)]
            ),
            Ok(value.clone()),
            "one token that is a JSON array carries the whole field"
        );
        assert_eq!(
            reported(
                &contract,
                &named("Objects#input"),
                &[("payload", r#"{"a":1}"#), ("payload", r#"{"b":2}"#)]
            ),
            Ok(value),
            "a repeated key reads the same value, not the text it was spelled with"
        );
    }
}
