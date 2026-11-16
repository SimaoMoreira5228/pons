use pons_ir::{Direction, InlineType, TypeKind, TypeRef};

use super::{definition, field, lower_root};

#[test]
fn a_number_schema_keeps_every_numeric_bound() {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "inclusive": {
                "type": "number",
                "minimum": 0,
                "maximum": 10,
                "multipleOf": 0.5
            },
            "exclusive": {
                "type": "number",
                "exclusiveMinimum": 1,
                "exclusiveMaximum": 9
            }
        },
        "required": ["inclusive", "exclusive"],
        "additionalProperties": false
    });
    let (cx, reference) = lower_root(schema, "acme::Numbers", "Numbers", Direction::Both);

    let TypeKind::Struct { fields, .. } = &definition(&cx, &reference).definition else {
        panic!()
    };
    let TypeRef::Inline(InlineType::Float {
        bits,
        minimum,
        maximum,
        multiple_of,
        ..
    }) = &field(fields, "inclusive").ty
    else {
        panic!(
            "expected a float inclusive bound, found {:?}",
            field(fields, "inclusive").ty
        )
    };
    assert_eq!(*bits, 64);
    assert_eq!(minimum.as_ref(), Some(&serde_json::json!(0)));
    assert_eq!(maximum.as_ref(), Some(&serde_json::json!(10)));
    assert_eq!(multiple_of.as_ref(), Some(&serde_json::json!(0.5)));

    let TypeRef::Inline(InlineType::Float {
        exclusive_minimum,
        exclusive_maximum,
        ..
    }) = &field(fields, "exclusive").ty
    else {
        panic!(
            "expected a float exclusive bound, found {:?}",
            field(fields, "exclusive").ty
        )
    };
    assert_eq!(exclusive_minimum.as_ref(), Some(&serde_json::json!(1)));
    assert_eq!(exclusive_maximum.as_ref(), Some(&serde_json::json!(9)));
}
