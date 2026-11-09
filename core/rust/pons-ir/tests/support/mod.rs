#![allow(dead_code)]

use pons_ir::{Direction, Document, TypeKind, TypeRef, Violation, validate_value};
use serde_json::{Value, json};

pub fn contract(types: Value) -> Document {
    serde_json::from_value(json!({
        "ir_version": "0.3",
        "info": {"title": "Example", "version": "1"},
        "types": types,
    }))
    .unwrap()
}

pub fn request_body(types: Value) -> Document {
    let mut document = contract(types);
    document.operations.push(
        serde_json::from_value(json!({
            "id": [["envelope"], ["send"]],
            "method": "POST",
            "path": [{"lit": "envelope"}],
            "body": {"ty": {"ref": "Envelope"}, "content": "json", "required": true},
            "responses": [{"status": 200, "content": "json", "body": {"kind": "string"}}],
            "auth": {"alternatives": [], "optional": false},
            "semantics": {"safe": false, "idempotent": false}
        }))
        .unwrap(),
    );
    document
}

pub fn fixture() -> Document {
    serde_json::from_str(include_str!("../../../../../spec/fixtures/login.json")).unwrap()
}

pub fn rejections(document: &Document) -> Vec<String> {
    document
        .validate()
        .unwrap_err()
        .iter()
        .map(ToString::to_string)
        .collect()
}

pub fn refuses(document: &Document, message: &str) {
    let found = rejections(document);
    assert!(
        found.iter().any(|error| error.contains(message)),
        "{message:?} missing from {found:?}"
    );
}

pub fn field_type<'a>(document: &'a Document, id: &str, index: usize) -> &'a TypeRef {
    match &document.types[id].definition {
        TypeKind::Struct { fields, .. } => &fields[index].ty,
        other => panic!("{id} is not a struct: {other:?}"),
    }
}

pub const CARRIERS: [&str; 3] = ["query", "form", "path"];

pub fn carrier_document(types: Value, carrier: &str) -> Document {
    let mut document = contract(types);
    let mut operation = json!({
        "id": [["events"], ["list"]],
        "method": "POST",
        "path": [{"lit": "events"}],
        "responses": [{"status": 200, "content": "json", "body": {"kind": "string"}}],
        "auth": {"alternatives": [], "optional": false},
        "semantics": {"safe": false, "idempotent": false}
    });
    match carrier {
        "query" => operation["query"] = json!({"ref": "Carrier"}),
        "form" => {
            operation["body"] =
                json!({"ty": {"ref": "Carrier"}, "content": "form", "required": true})
        }
        "json" => {
            operation["body"] =
                json!({"ty": {"ref": "Carrier"}, "content": "json", "required": true})
        }
        "response" => operation["responses"][0]["body"] = json!({"ref": "Carrier"}),
        _ => {
            operation["path"] = json!([
                {"lit": "events"},
                {"param": "carrier", "ty": {"ref": "Carrier"}}
            ])
        }
    }
    document
        .operations
        .push(serde_json::from_value(operation).unwrap());
    document
}

pub fn verdict(document: &Document) -> Vec<String> {
    match document.validate() {
        Ok(()) => Vec::new(),
        Err(errors) => errors.iter().map(ToString::to_string).collect(),
    }
}

pub fn verdicts(types: Value) -> Vec<Vec<String>> {
    CARRIERS
        .iter()
        .map(|carrier| verdict(&carrier_document(types.clone(), carrier)))
        .collect()
}

pub fn agreed(types: Value) -> Vec<Vec<String>> {
    let verdicts = verdicts(types);
    let refused = verdicts.iter().filter(|found| !found.is_empty()).count();
    assert!(
        refused == 0 || refused == CARRIERS.len(),
        "one rule means every flat carrier answers a shape the same way: {verdicts:?}"
    );
    verdicts
}

pub fn carried_by_every_carrier(types: Value) {
    for (carrier, found) in CARRIERS.iter().zip(agreed(types.clone())) {
        assert!(
            found.is_empty(),
            "the {carrier} carrier refused a shape every flat carrier carries: {found:?}"
        );
    }
}

pub fn refused_by_every_carrier(types: Value, blamed: &str) {
    for (carrier, found) in CARRIERS.iter().zip(agreed(types.clone())) {
        assert!(
            found
                .iter()
                .any(|message| message.contains(blamed) && message.contains(carrier)),
            "the {carrier} carrier must name what it refused: {found:?}"
        );
    }
}

pub fn field(wire: &str, ty: Value, presence: &str) -> Value {
    json!({"wire": wire, "ident": [wire], "ty": ty, "presence": presence})
}

pub fn field_ref(wire: &str, ty: &TypeRef, presence: &str) -> Value {
    json!({"wire": wire, "ident": [wire], "ty": ty, "presence": presence})
}

pub fn string() -> Value {
    json!({"kind": "string"})
}

pub fn with(mut types: Value, extra: Value) -> Value {
    types
        .as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    types
}

pub fn carrier_struct(openness: Value, fields: Vec<Value>, flattened: Vec<&str>) -> Value {
    json!({
        "Carrier": {
            "kind": "struct", "name": ["carrier"], "direction": "input", "openness": openness,
            "fields": fields,
            "flattened": flattened.iter().map(|id| json!({"ref": id})).collect::<Vec<_>>()
        }
    })
}

pub fn access_carrier(
    direction: &str,
    presence: &str,
    access: &str,
    default: Option<Value>,
) -> Value {
    let mut field = field("tenant", string(), presence);
    field["access"] = json!(access);
    if let Some(default) = default {
        field["default"] = default;
    }
    let mut types = carrier_struct(json!("reject"), vec![field], vec![]);
    types["Carrier"]["direction"] = json!(direction);
    types
}

pub fn struct_type(
    name: &str,
    fields: Vec<Value>,
    flattened: Vec<Value>,
    openness: Value,
) -> Value {
    json!({
        "kind": "struct", "name": [name], "direction": "both",
        "openness": openness, "fields": fields, "flattened": flattened
    })
}

pub fn struct_map(id: &str, openness: &str, fields: Value, flattened: Value) -> Value {
    json!({
        id: {
            "kind": "struct",
            "name": [id],
            "direction": "both",
            "openness": openness,
            "fields": fields,
            "flattened": flattened,
        }
    })
}

pub fn violations(
    types: Value,
    ty: &TypeRef,
    direction: Direction,
    value: Value,
) -> Vec<Violation> {
    let document = contract(types);
    document.validate().unwrap();
    validate_value(&document, ty, direction, &value).unwrap_err()
}

pub fn messages(found: Vec<Violation>) -> Vec<String> {
    found.iter().map(ToString::to_string).collect()
}

pub fn checked(types: Value, ty: &TypeRef, value: Value) {
    checked_in(types, ty, Direction::Both, value)
}

pub fn checked_in(types: Value, ty: &TypeRef, direction: Direction, value: Value) {
    let document = contract(types);
    document.validate().unwrap();
    if let Err(found) = validate_value(&document, ty, direction, &value) {
        panic!("{value} was rejected: {}", messages(found).join(", "));
    }
}

pub fn inline(extra: Value) -> TypeRef {
    serde_json::from_value(extra).unwrap()
}

pub fn text(minimum: Option<u64>, maximum: Option<u64>, pattern: Option<&str>) -> TypeRef {
    serde_json::from_value(json!({
        "kind": "string",
        "min_length": minimum,
        "max_length": maximum,
        "pattern": pattern,
    }))
    .unwrap()
}

pub fn named(id: &str) -> TypeRef {
    TypeRef::Named(pons_ir::NamedRef {
        id: id.into(),
        variant: None,
    })
}
