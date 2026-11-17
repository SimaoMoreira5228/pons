use pons_ir::{Direction, InlineType, NamedRef, PathSegment, TypeKind, TypeRef};
use serde_json::{Value, json};

use super::super::{fixture, named_type, string};

#[test]
fn escapes_path_literals_like_the_typescript_client() {
    let mut document = fixture();
    document.operations[0].path = vec![PathSegment::Literal {
        lit: "a b?c#dé%f".into(),
    }];
    document.operations[1].path = vec![PathSegment::Literal {
        lit: "a%20b".into(),
    }];
    let output: Value = serde_json::from_str(&crate::emit(&document).unwrap()).unwrap();
    let paths = output["paths"].as_object().unwrap();
    assert!(paths.contains_key("/a%20b%3Fc%23d%C3%A9%25f"), "{paths:?}");
    assert!(paths.contains_key("/a%2520b"), "{paths:?}");
}

#[test]
fn emits_every_float_bound() {
    let mut raw = serde_json::to_value(fixture()).unwrap();
    raw["types"]["Measurement"] = json!({
        "kind": "struct",
        "name": ["measurement"],
        "direction": "both",
        "openness": "reject",
        "fields": [{
            "wire": "range",
            "ident": ["range"],
            "ty": {
                "kind": "float",
                "bits": 64,
                "minimum": 0,
                "maximum": 100,
                "multiple_of": 0.5
            },
            "presence": "required"
        }, {
            "wire": "open_range",
            "ident": ["open", "range"],
            "ty": {
                "kind": "float",
                "bits": 64,
                "exclusive_minimum": 0,
                "exclusive_maximum": 100
            },
            "presence": "required"
        }]
    });
    let document: pons_ir::Document = serde_json::from_value(raw).unwrap();
    let output: Value = serde_json::from_str(&crate::emit(&document).unwrap()).unwrap();
    let properties = &output["components"]["schemas"]["Measurement"]["properties"];

    assert_eq!(properties["range"]["minimum"], 0);
    assert_eq!(properties["range"]["maximum"], 100);
    assert_eq!(properties["range"]["multipleOf"], 0.5);
    assert_eq!(properties["open_range"]["exclusiveMinimum"], 0);
    assert_eq!(properties["open_range"]["exclusiveMaximum"], 100);
}

#[test]
fn emits_recursive_selected_enum_variants() {
    let mut raw: Value = serde_json::to_value(fixture()).unwrap();
    raw["types"]["Chain"] = json!({
        "kind": "enum",
        "name": ["chain"],
        "direction": "both",
        "repr": {"kind": "external"},
        "variants": [
            {
                "wire": "next",
                "ident": ["next"],
                "payload": {"kind": "newtype", "ty": {"ref": "Chain", "variant": "next"}}
            },
            {"wire": "leaf", "ident": ["leaf"], "payload": {"kind": "unit"}}
        ]
    });
    raw["types"]["User"]["fields"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "wire": "chain",
            "ident": ["chain"],
            "ty": {"ref": "Chain", "variant": "next"},
            "presence": "required"
        }));
    let document: pons_ir::Document = serde_json::from_value(raw).unwrap();
    let output = crate::emit(&document).unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["openapi"], "3.1.1");
    assert!(
        value["components"]["schemas"]["Chain"]["oneOf"]
            .as_array()
            .unwrap()
            .iter()
            .any(|schema| schema
                .to_string()
                .contains("#/components/schemas/Chain/oneOf/"))
    );
}

#[test]
fn unwraps_transparent_query_types_and_rejects_ambiguous_fields() {
    let mut document = fixture();
    let mut wrapper = named_type("eventQueryInput");
    wrapper.direction = Direction::Input;
    wrapper.definition = TypeKind::Newtype {
        inner: TypeRef::Named(NamedRef {
            id: "EventQuery".into(),
            variant: None,
        }),
    };
    document.types.insert("EventQueryInput".into(), wrapper);
    let events = document
        .operations
        .iter_mut()
        .find(|operation| operation.stream.is_some())
        .unwrap();
    events.query = Some(TypeRef::Named(NamedRef {
        id: "EventQueryInput".into(),
        variant: None,
    }));

    let output: Value = serde_json::from_str(&crate::emit(&document).unwrap()).unwrap();
    assert_eq!(
        output["paths"]["/events/{cursor}"]["get"]["parameters"][1]["name"],
        "limit"
    );

    let TypeKind::Struct { fields, .. } =
        &mut document.types.get_mut("EventQuery").unwrap().definition
    else {
        unreachable!()
    };
    fields[0].ty = TypeRef::Inline(InlineType::Array {
        item: Box::new(string()),
        unique: None,
    });
    assert!(
        crate::emit(&document)
            .unwrap_err()
            .0
            .contains("query fields must use scalar types")
    );
}
