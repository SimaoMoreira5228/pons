mod support;

use pons_ir::{Direction, TypeRef};
use serde_json::{Value, json};
use support::{
    checked, field_ref as field, inline, messages, named, struct_map as struct_type, text,
    violations,
};

fn narrowed(id: &str, variant: &str) -> TypeRef {
    TypeRef::Named(pons_ir::NamedRef {
        id: id.into(),
        variant: Some(variant.into()),
    })
}

fn enum_type(id: &str, repr: Value, variants: Value) -> Value {
    json!({
        id: {
            "kind": "enum",
            "name": [id],
            "direction": "both",
            "repr": repr,
            "variants": variants,
        }
    })
}

fn unit(wire: &str) -> Value {
    json!({"wire": wire, "ident": [wire], "payload": {"kind": "unit"}})
}

fn newtype_variant(wire: &str, ty: &TypeRef) -> Value {
    json!({"wire": wire, "ident": [wire], "payload": {"kind": "newtype", "ty": ty}})
}

#[test]
fn nullable_accepts_null_and_any_accepts_everything() {
    let types = json!({});
    let nullable = inline(
        json!({"kind": "nullable", "inner": {"kind": "int", "bits": 8, "signed": false, "wire": "number"}}),
    );
    checked(types.clone(), &nullable, json!(null));
    checked(types.clone(), &nullable, json!(1));
    assert_eq!(
        messages(violations(
            types.clone(),
            &nullable,
            Direction::Both,
            json!("1")
        )),
        ["expected a number, found a string"]
    );
    checked(types.clone(), &inline(json!({"kind": "null"})), json!(null));
    assert_eq!(
        messages(violations(
            types.clone(),
            &inline(json!({"kind": "null"})),
            Direction::Both,
            json!(0)
        )),
        ["expected null, found a number"]
    );
    for value in [
        json!(null),
        json!(1),
        json!("text"),
        json!([1]),
        json!({"a": 1}),
    ] {
        checked(types.clone(), &inline(json!({"kind": "any"})), value);
    }
}

#[test]
fn opaque_values_pass_because_the_codec_owns_them() {
    let types = json!({});
    let opaque = inline(json!({"kind": "opaque", "codec": "example", "wire": "json"}));
    for value in [json!(null), json!("anything"), json!({"nested": [1, 2]})] {
        checked(types.clone(), &opaque, value);
    }
}

#[test]
fn named_types_resolve_and_unknown_references_are_reported() {
    let types = struct_type(
        "User",
        "reject",
        json!([field("id", &text(None, None, None), "required")]),
        json!([]),
    );
    checked(types.clone(), &named("User"), json!({"id": "7"}));
    assert_eq!(
        messages(violations(
            types.clone(),
            &named("Ghost"),
            Direction::Both,
            json!({})
        )),
        ["unknown type Ghost"]
    );
    assert_eq!(
        messages(violations(
            types,
            &named("User"),
            Direction::Both,
            json!({"id": 7})
        )),
        ["/id: expected a string, found a number"]
    );
}

#[test]
fn newtypes_are_transparent() {
    let types = json!({
        "Name": {
            "kind": "newtype",
            "name": ["name"],
            "direction": "both",
            "inner": {"kind": "string", "min_length": 2},
        }
    });
    checked(types.clone(), &named("Name"), json!("ab"));
    assert_eq!(
        messages(violations(
            types,
            &named("Name"),
            Direction::Both,
            json!("a")
        )),
        ["string is shorter than the 2 code points it allows"]
    );
}

#[test]
fn openness_policies_govern_extra_properties() {
    let id = field("id", &text(None, None, None), "required");
    let count = json!({"kind": "int", "bits": 8, "signed": false, "wire": "number"});
    let types = json!({
        "Counts": {
            "kind": "newtype",
            "name": ["counts"],
            "direction": "both",
            "inner": {"kind": "map", "key": {"kind": "string"}, "value": count},
        },
        "Opaque": {
            "kind": "struct",
            "name": ["opaque"],
            "direction": "both",
            "openness": "preserve_opaque",
            "fields": [id.clone()],
        },
        "Typed": {
            "kind": "struct",
            "name": ["typed"],
            "direction": "both",
            "openness": {"preserve_typed": {"value": {"ref": "Counts"}}},
            "fields": [json!({
                "wire": "id", "ident": ["id"], "ty": count, "presence": "required"
            })],
        },
        "Closed": {
            "kind": "struct",
            "name": ["closed"],
            "direction": "both",
            "openness": "reject",
            "fields": [id],
        },
    });
    checked(
        types.clone(),
        &named("Opaque"),
        json!({"id": "7", "extra": {"any": true}}),
    );
    checked(types.clone(), &named("Typed"), json!({"id": 3, "hits": 3}));
    assert_eq!(
        messages(violations(
            types.clone(),
            &named("Typed"),
            Direction::Both,
            json!({"id": 3, "hits": 300})
        )),
        ["/hits: integer is outside the u8 range"]
    );
    assert_eq!(
        messages(violations(
            types.clone(),
            &named("Typed"),
            Direction::Both,
            json!({"id": 3, "hits": {"a": 1}})
        )),
        ["/hits: expected a number, found an object"]
    );
    assert_eq!(
        messages(violations(
            types,
            &named("Closed"),
            Direction::Both,
            json!({"id": "7", "role": "admin"})
        )),
        ["/role: unknown property role"]
    );
}

#[test]
fn string_enums_name_their_variant() {
    let types = enum_type(
        "State",
        json!({"kind": "string"}),
        json!([unit("open"), unit("closed")]),
    );
    checked(types.clone(), &named("State"), json!("open"));
    assert_eq!(
        messages(violations(
            types.clone(),
            &named("State"),
            Direction::Both,
            json!("gone")
        )),
        ["unknown enum variant gone"]
    );
    assert_eq!(
        messages(violations(
            types,
            &named("State"),
            Direction::Both,
            json!(1)
        )),
        ["expected an enum variant name, found a number"]
    );
}

#[test]
fn external_enums_carry_their_payload_beside_the_variant() {
    let count = inline(json!({"kind": "int", "bits": 8, "signed": false, "wire": "number"}));
    let types = enum_type(
        "Shape",
        json!({"kind": "external"}),
        json!([unit("point"), newtype_variant("circle", &count)]),
    );
    checked(types.clone(), &named("Shape"), json!({"circle": 3}));
    checked(types.clone(), &named("Shape"), json!("point"));
    assert_eq!(
        messages(violations(
            types.clone(),
            &named("Shape"),
            Direction::Both,
            json!({"circle": 300})
        )),
        ["/circle: integer is outside the u8 range"]
    );
    assert_eq!(
        messages(violations(
            types.clone(),
            &named("Shape"),
            Direction::Both,
            json!({"circle": 1, "point": null})
        )),
        ["expected an object with one variant, found an object"]
    );
    assert_eq!(
        messages(violations(
            types,
            &named("Shape"),
            Direction::Both,
            json!({"blob": 1})
        )),
        ["unknown enum variant blob"]
    );
}

#[test]
fn internal_and_adjacent_enums_read_their_tag() {
    let text_type = inline(json!({"kind": "string"}));
    let variants = json!([
        unit("invalid"),
        {
            "wire": "locked",
            "ident": ["locked"],
            "payload": {
                "kind": "struct",
                "fields": [field("retry_after", &inline(json!({"kind": "int", "bits": 32, "signed": false, "wire": "number"})), "required")],
            }
        },
        newtype_variant("hint", &text_type),
    ]);
    let internal = enum_type(
        "Internal",
        json!({"kind": "internal", "tag": "code"}),
        variants.clone(),
    );
    let adjacent = enum_type(
        "Adjacent",
        json!({"kind": "adjacent", "tag": "code", "content": "detail"}),
        variants,
    );
    let types = json!({
        "Internal": internal["Internal"].clone(),
        "Adjacent": adjacent["Adjacent"].clone(),
    });
    checked(
        types.clone(),
        &named("Internal"),
        json!({"code": "invalid"}),
    );
    checked(
        types.clone(),
        &named("Internal"),
        json!({"code": "locked", "retry_after": 30}),
    );
    assert_eq!(
        messages(violations(
            types.clone(),
            &named("Internal"),
            Direction::Both,
            json!({"code": "locked"})
        )),
        ["/retry_after: missing required field retry_after"]
    );
    assert_eq!(
        messages(violations(
            types.clone(),
            &named("Internal"),
            Direction::Both,
            json!({"state": "invalid"})
        )),
        ["/code: missing required field code"]
    );
    checked(
        types.clone(),
        &named("Adjacent"),
        json!({"code": "invalid"}),
    );
    checked(
        types.clone(),
        &named("Adjacent"),
        json!({"code": "hint", "detail": "try later"}),
    );
    assert_eq!(
        messages(violations(
            types,
            &named("Adjacent"),
            Direction::Both,
            json!({"code": "hint"})
        )),
        ["/detail: missing required field detail"]
    );
}

#[test]
fn untagged_enums_accept_the_first_variant_that_fits() {
    let text_type = inline(json!({"kind": "string"}));
    let count = inline(json!({"kind": "int", "bits": 8, "signed": false, "wire": "number"}));
    let types = enum_type(
        "Value",
        json!({"kind": "untagged"}),
        json!([
            unit("nothing"),
            newtype_variant("text", &text_type),
            newtype_variant("count", &count),
        ]),
    );
    checked(types.clone(), &named("Value"), json!(null));
    checked(types.clone(), &named("Value"), json!("text"));
    checked(types.clone(), &named("Value"), json!(3));
    assert_eq!(
        messages(violations(
            types,
            &named("Value"),
            Direction::Both,
            json!(300)
        )),
        ["value matches none of the enum variants"]
    );
}

#[test]
fn a_narrowed_reference_validates_only_its_variant() {
    let text_type = inline(json!({"kind": "string"}));
    let types = enum_type(
        "Shape",
        json!({"kind": "external"}),
        json!([unit("point"), newtype_variant("label", &text_type)]),
    );
    checked(
        types.clone(),
        &narrowed("Shape", "point"),
        json!({"point": null}),
    );
    assert_eq!(
        messages(violations(
            types.clone(),
            &narrowed("Shape", "point"),
            Direction::Both,
            json!({"label": "x"})
        )),
        ["expected enum variant point, found label"]
    );
    assert_eq!(
        messages(violations(
            types,
            &narrowed("Shape", "ghost"),
            Direction::Both,
            json!({"point": null})
        )),
        ["expected enum variant ghost, found point"]
    );
}

#[test]
fn recursive_types_terminate() {
    let types = json!({
        "Node": {
            "kind": "struct",
            "name": ["node"],
            "direction": "both",
            "openness": "reject",
            "fields": [
                field("value", &inline(json!({"kind": "int", "bits": 8, "signed": false, "wire": "number"})), "required"),
                field("next", &inline(json!({"kind": "nullable", "inner": {"ref": "Node"}})), "required"),
            ],
        },
        "Deaf": {
            "kind": "struct",
            "name": ["deaf"],
            "direction": "both",
            "openness": "reject",
            "fields": [field("child", &named("Deaf"), "required")],
        },
        "Wrapped": {
            "kind": "newtype",
            "name": ["wrapped"],
            "direction": "both",
            "inner": {"ref": "Node"},
        }
    });
    let chain = json!({"value": 1, "next": {"value": 2, "next": {"value": 3, "next": null}}});
    checked(types.clone(), &named("Node"), chain);
    assert_eq!(
        messages(violations(
            types.clone(),
            &named("Node"),
            Direction::Both,
            json!({"value": 1, "next": {"value": 2}})
        )),
        ["/next/next: missing required field next"]
    );
    assert_eq!(
        messages(violations(
            types.clone(),
            &named("Deaf"),
            Direction::Both,
            json!({"child": {"child": {}}})
        )),
        ["/child/child/child: missing required field child"]
    );
    checked(
        types.clone(),
        &named("Wrapped"),
        json!({"value": 1, "next": null}),
    );
}

#[test]
fn a_catch_all_variant_still_reads_as_the_unit_payload_it_is() {
    let types = enum_type(
        "State",
        json!({"kind": "string"}),
        json!([
            unit("open"),
            json!({
                "wire": "other",
                "ident": ["other"],
                "payload": {"kind": "unit"},
                "ext": {"com.pons.open_variant/v1": true}
            }),
        ]),
    );
    checked(types.clone(), &named("State"), json!("other"));
    assert_eq!(
        messages(violations(
            types,
            &named("State"),
            Direction::Both,
            json!("gone")
        )),
        ["unknown enum variant gone"]
    );
}
