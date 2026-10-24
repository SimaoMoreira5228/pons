use pons_ir::{Document, NamedRef, TypeKind, TypeRef};
use serde_json::{Value, json};

fn fixture() -> Value {
    serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap()
}

fn string() -> Value {
    json!({ "kind": "string" })
}

fn count() -> Value {
    json!({ "kind": "int", "bits": 16, "signed": false, "wire": "number" })
}

fn ty(value: Value) -> TypeRef {
    serde_json::from_value(value).unwrap()
}

fn with_types(extra: Value) -> Document {
    let mut raw = fixture();
    for (id, definition) in extra.as_object().expect("a map of named types") {
        raw["types"][id] = definition.clone();
    }
    serde_json::from_value(raw).unwrap()
}

fn with_query(document: Document, query: TypeRef) -> Document {
    let mut document = document;
    document.operations[1].query = Some(query);
    document
}

fn ref_to(id: &str) -> TypeRef {
    TypeRef::Named(NamedRef {
        id: id.into(),
        variant: None,
    })
}

fn with_query_field(document: Document, field: TypeRef) -> Document {
    let mut document = document;
    let TypeKind::Struct { fields, .. } =
        &mut document.types.get_mut("EventQuery").unwrap().definition
    else {
        panic!("EventQuery is a struct")
    };
    fields[0].ty = field;
    document
}

fn with_required_query_field(document: Document, field: TypeRef) -> Document {
    let mut document = with_query_field(document, field);
    let TypeKind::Struct { fields, .. } =
        &mut document.types.get_mut("EventQuery").unwrap().definition
    else {
        panic!("EventQuery is a struct")
    };
    fields[0].presence = pons_ir::Presence::Required;
    document
}

fn rejects(name: &str, document: Document) {
    let openapi = pons_emit_openapi::emit(&document);
    let typescript = pons_emit_ts::emit(&document);
    assert!(
        openapi.is_err() && typescript.is_err(),
        "{name}: both emitters must reject (openapi={:?}, typescript={:?})",
        openapi.as_ref().err().map(|error| error.to_string()),
        typescript.as_ref().err().map(|error| error.to_string()),
    );
}

fn accepts(name: &str, document: Document) {
    let openapi = pons_emit_openapi::emit(&document);
    let typescript = pons_emit_ts::emit(&document);
    assert!(
        openapi.is_ok() && typescript.is_ok(),
        "{name}: both emitters must accept (openapi={:?}, typescript={:?})",
        openapi.as_ref().err().map(|error| error.to_string()),
        typescript.as_ref().err().map(|error| error.to_string()),
    );
}

#[test]
fn emitters_agree_on_query_fields_with_a_wire_encoding() {
    let fixture = || with_types(json!({}));
    accepts(
        "bool",
        with_query_field(fixture(), ty(json!({ "kind": "bool" }))),
    );
    accepts("int", with_query_field(fixture(), ty(count())));
    accepts(
        "float",
        with_query_field(fixture(), ty(json!({ "kind": "float", "bits": 64 }))),
    );
    accepts(
        "decimal",
        with_query_field(
            fixture(),
            ty(json!({ "kind": "decimal", "wire": "string" })),
        ),
    );
    accepts("string", with_query_field(fixture(), ty(string())));
    accepts(
        "optional nullable",
        with_query_field(
            fixture(),
            ty(json!({ "kind": "nullable", "inner": string() })),
        ),
    );
    for encoding in ["base64", "base64url", "hex"] {
        accepts(
            encoding,
            with_query_field(
                fixture(),
                ty(json!({ "kind": "bytes", "encoding": encoding })),
            ),
        );
    }
    accepts(
        "string enum",
        with_query_field(
            with_types(json!({
                "Sort": {
                    "kind": "enum",
                    "name": ["sort"],
                    "direction": "input",
                    "repr": { "kind": "string" },
                    "variants": [
                        { "wire": "asc", "ident": ["asc"], "payload": { "kind": "unit" } },
                        { "wire": "desc", "ident": ["desc"], "payload": { "kind": "unit" } }
                    ]
                }
            })),
            ref_to("Sort"),
        ),
    );
    accepts(
        "narrowed string enum variant",
        with_query_field(
            with_types(json!({
                "Sort": {
                    "kind": "enum",
                    "name": ["sort"],
                    "direction": "input",
                    "repr": { "kind": "string" },
                    "variants": [
                        { "wire": "asc", "ident": ["asc"], "payload": { "kind": "unit" } },
                        { "wire": "desc", "ident": ["desc"], "payload": { "kind": "unit" } }
                    ]
                }
            })),
            TypeRef::Named(NamedRef {
                id: "Sort".into(),
                variant: Some("asc".into()),
            }),
        ),
    );
    accepts(
        "newtype over string",
        with_query_field(
            with_types(json!({
                "Slug": {
                    "kind": "newtype",
                    "name": ["slug"],
                    "direction": "input",
                    "inner": string()
                }
            })),
            ref_to("Slug"),
        ),
    );
    accepts(
        "newtype over bytes",
        with_query_field(
            with_types(json!({
                "Digest": {
                    "kind": "newtype",
                    "name": ["digest"],
                    "direction": "input",
                    "inner": { "kind": "bytes", "encoding": "base64" }
                }
            })),
            ref_to("Digest"),
        ),
    );
    accepts(
        "nested newtype over string",
        with_query_field(
            with_types(json!({
                "Slug": {
                    "kind": "newtype",
                    "name": ["slug"],
                    "direction": "input",
                    "inner": string()
                },
                "SlugAlias": {
                    "kind": "newtype",
                    "name": ["slug", "alias"],
                    "direction": "input",
                    "inner": { "ref": "Slug" }
                }
            })),
            ref_to("SlugAlias"),
        ),
    );
}

#[test]
fn emitters_agree_on_query_fields_without_a_wire_encoding() {
    let fixture = || with_types(json!({}));
    rejects(
        "array",
        with_query_field(
            fixture(),
            ty(json!({ "kind": "array", "item": string(), "unique": null })),
        ),
    );
    rejects(
        "map",
        with_query_field(
            fixture(),
            ty(json!({ "kind": "map", "key": string(), "value": string() })),
        ),
    );
    rejects(
        "tuple",
        with_query_field(
            fixture(),
            ty(json!({ "kind": "tuple", "items": [string()] })),
        ),
    );
    rejects(
        "required nullable",
        with_required_query_field(
            fixture(),
            ty(json!({ "kind": "nullable", "inner": string() })),
        ),
    );
    rejects(
        "null",
        with_query_field(fixture(), ty(json!({ "kind": "null" }))),
    );
    rejects(
        "any",
        with_query_field(fixture(), ty(json!({ "kind": "any" }))),
    );
    rejects(
        "opaque",
        with_query_field(
            fixture(),
            ty(json!({ "kind": "opaque", "codec": "json", "wire": "json" })),
        ),
    );
    rejects(
        "array-encoded bytes",
        with_query_field(
            fixture(),
            ty(json!({ "kind": "bytes", "encoding": "array" })),
        ),
    );
    rejects("named struct", with_query_field(fixture(), ref_to("User")));
    rejects(
        "internal-tagged enum",
        with_query_field(fixture(), ref_to("LoginError")),
    );
    rejects(
        "narrowed non-string enum variant",
        with_query_field(
            fixture(),
            TypeRef::Named(NamedRef {
                id: "LoginError".into(),
                variant: Some("locked".into()),
            }),
        ),
    );
    rejects(
        "untagged enum",
        with_query_field(
            with_types(json!({
                "Blob": {
                    "kind": "enum",
                    "name": ["blob"],
                    "direction": "input",
                    "repr": { "kind": "untagged" },
                    "variants": [{ "wire": "raw", "ident": ["raw"], "payload": { "kind": "unit" } }]
                }
            })),
            ref_to("Blob"),
        ),
    );
    rejects(
        "newtype over a struct",
        with_query_field(
            with_types(json!({
                "UserNewtype": {
                    "kind": "newtype",
                    "name": ["user", "newtype"],
                    "direction": "input",
                    "inner": { "ref": "User" }
                }
            })),
            ref_to("UserNewtype"),
        ),
    );
    rejects(
        "self-recursive newtype",
        with_query_field(
            with_types(json!({
                "Loop": {
                    "kind": "newtype",
                    "name": ["loop"],
                    "direction": "input",
                    "inner": { "ref": "Loop" }
                }
            })),
            ref_to("Loop"),
        ),
    );
    rejects(
        "mutually recursive newtype",
        with_query_field(
            with_types(json!({
                "Ping": {
                    "kind": "newtype",
                    "name": ["ping"],
                    "direction": "input",
                    "inner": { "ref": "Pong" }
                },
                "Pong": {
                    "kind": "newtype",
                    "name": ["pong"],
                    "direction": "input",
                    "inner": { "ref": "Ping" }
                }
            })),
            ref_to("Ping"),
        ),
    );
}

#[test]
fn emitters_agree_on_rejected_query_types() {
    let fixture = || with_types(json!({}));
    rejects("inline scalar", with_query(fixture(), ty(string())));
    rejects(
        "inline nullable",
        with_query(
            fixture(),
            ty(json!({ "kind": "nullable", "inner": string() })),
        ),
    );
    rejects(
        "inline map",
        with_query(
            fixture(),
            ty(json!({ "kind": "map", "key": string(), "value": string() })),
        ),
    );
    rejects("enum", with_query(fixture(), ref_to("LoginError")));
    rejects(
        "narrowed enum variant",
        with_query(
            fixture(),
            TypeRef::Named(NamedRef {
                id: "LoginError".into(),
                variant: Some("locked".into()),
            }),
        ),
    );
    rejects(
        "newtype over a scalar",
        with_query(
            with_types(json!({
                "ScalarQuery": {
                    "kind": "newtype",
                    "name": ["scalar", "query"],
                    "direction": "input",
                    "inner": string()
                }
            })),
            ref_to("ScalarQuery"),
        ),
    );
    rejects(
        "newtype over a nullable struct",
        with_query(
            with_types(json!({
                "NullableQuery": {
                    "kind": "newtype",
                    "name": ["nullable", "query"],
                    "direction": "input",
                    "inner": { "kind": "nullable", "inner": { "ref": "EventQuery" } }
                }
            })),
            ref_to("NullableQuery"),
        ),
    );
    rejects(
        "self-recursive transparent query",
        with_query(
            with_types(json!({
                "LoopQuery": {
                    "kind": "newtype",
                    "name": ["loop", "query"],
                    "direction": "input",
                    "inner": { "ref": "LoopQuery" }
                }
            })),
            ref_to("LoopQuery"),
        ),
    );
    for (name, openness) in [
        (
            "open typed query object",
            json!({ "preserve_typed": { "value": string() } }),
        ),
        ("open opaque query object", json!("preserve_opaque")),
    ] {
        rejects(
            name,
            with_query(
                with_types(json!({
                    "OpenQuery": {
                        "kind": "struct",
                        "name": ["open", "query"],
                        "direction": "input",
                        "openness": openness,
                        "fields": [{ "wire": "limit", "ident": ["limit"], "ty": count(), "presence": "optional" }]
                    }
                })),
                ref_to("OpenQuery"),
            ),
        );
    }
    rejects(
        "flattened open query object",
        with_query(
            with_types(json!({
                "OpenMeta": {
                    "kind": "struct",
                    "name": ["open", "meta"],
                    "direction": "input",
                    "openness": "preserve_opaque",
                    "fields": []
                },
                "FlatQuery": {
                    "kind": "struct",
                    "name": ["flat", "query"],
                    "direction": "input",
                    "openness": "reject",
                    "fields": [{ "wire": "own", "ident": ["own"], "ty": string(), "presence": "required" }],
                    "flattened": [{ "ref": "OpenMeta" }]
                }
            })),
            ref_to("FlatQuery"),
        ),
    );
    rejects(
        "flattened field without a wire encoding",
        with_query(
            with_types(json!({
                "BadPage": {
                    "kind": "struct",
                    "name": ["bad", "page"],
                    "direction": "input",
                    "openness": "reject",
                    "fields": [{ "wire": "tags", "ident": ["tags"], "ty": { "kind": "array", "item": string(), "unique": null }, "presence": "optional" }]
                },
                "PagedQuery": {
                    "kind": "struct",
                    "name": ["paged", "query"],
                    "direction": "input",
                    "openness": "reject",
                    "fields": [{ "wire": "limit", "ident": ["limit"], "ty": count(), "presence": "optional" }],
                    "flattened": [{ "ref": "BadPage" }]
                }
            })),
            ref_to("PagedQuery"),
        ),
    );
}

#[test]
fn both_emitters_carry_flattened_query_fields() {
    let document = with_query(
        with_types(json!({
            "PageMeta": {
                "kind": "struct",
                "name": ["page", "meta"],
                "direction": "input",
                "openness": "reject",
                "fields": [{ "wire": "cursor", "ident": ["cursor"], "ty": string(), "presence": "optional" }]
            },
            "PagedQuery": {
                "kind": "newtype",
                "name": ["paged", "query"],
                "direction": "input",
                "inner": { "ref": "PagedQueryInner" }
            },
            "PagedQueryInner": {
                "kind": "struct",
                "name": ["paged", "query", "inner"],
                "direction": "input",
                "openness": "reject",
                "fields": [{ "wire": "limit", "ident": ["limit"], "ty": count(), "presence": "optional" }],
                "flattened": [{ "ref": "PageMeta" }]
            }
        })),
        ref_to("PagedQuery"),
    );
    accepts("flattened query object behind a newtype", document.clone());

    let openapi: Value =
        serde_json::from_str(&pons_emit_openapi::emit(&document).unwrap()).unwrap();
    let names: Vec<&str> = openapi["paths"]["/events/{cursor}"]["get"]["parameters"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|parameter| parameter["in"] == "query")
        .map(|parameter| parameter["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["limit", "cursor"]);
}

#[test]
fn both_emitters_reject_duplicate_query_wire_names() {
    rejects(
        "own field shadowed by a flattened field",
        with_query(
            with_types(json!({
                "PageMeta": {
                    "kind": "struct",
                    "name": ["page", "meta"],
                    "direction": "input",
                    "openness": "reject",
                    "fields": [{ "wire": "limit", "ident": ["limit"], "ty": count(), "presence": "optional" }]
                },
                "PagedQuery": {
                    "kind": "struct",
                    "name": ["paged", "query"],
                    "direction": "input",
                    "openness": "reject",
                    "fields": [{ "wire": "limit", "ident": ["limit"], "ty": count(), "presence": "optional" }],
                    "flattened": [{ "ref": "PageMeta" }]
                }
            })),
            ref_to("PagedQuery"),
        ),
    );
    rejects(
        "two flattened structs sharing a wire name",
        with_query(
            with_types(json!({
                "FirstMeta": {
                    "kind": "struct",
                    "name": ["first", "meta"],
                    "direction": "input",
                    "openness": "reject",
                    "fields": [{ "wire": "cursor", "ident": ["cursor"], "ty": string(), "presence": "optional" }]
                },
                "SecondMeta": {
                    "kind": "struct",
                    "name": ["second", "meta"],
                    "direction": "input",
                    "openness": "reject",
                    "fields": [{ "wire": "cursor", "ident": ["cursor"], "ty": string(), "presence": "optional" }]
                },
                "PagedQuery": {
                    "kind": "struct",
                    "name": ["paged", "query"],
                    "direction": "input",
                    "openness": "reject",
                    "fields": [{ "wire": "limit", "ident": ["limit"], "ty": count(), "presence": "optional" }],
                    "flattened": [{ "ref": "FirstMeta" }, { "ref": "SecondMeta" }]
                }
            })),
            ref_to("PagedQuery"),
        ),
    );
    rejects(
        "the same flattened type inlined twice",
        with_query(
            with_types(json!({
                "PageMeta": {
                    "kind": "struct",
                    "name": ["page", "meta"],
                    "direction": "input",
                    "openness": "reject",
                    "fields": [{ "wire": "cursor", "ident": ["cursor"], "ty": string(), "presence": "optional" }]
                },
                "PagedQuery": {
                    "kind": "struct",
                    "name": ["paged", "query"],
                    "direction": "input",
                    "openness": "reject",
                    "fields": [{ "wire": "limit", "ident": ["limit"], "ty": count(), "presence": "optional" }],
                    "flattened": [{ "ref": "PageMeta" }, { "ref": "PageMeta" }]
                }
            })),
            ref_to("PagedQuery"),
        ),
    );

    accepts(
        "distinct wire names behind one ident",
        with_query(
            with_types(json!({
                "PageMeta": {
                    "kind": "struct",
                    "name": ["page", "meta"],
                    "direction": "input",
                    "openness": "reject",
                    "fields": [{ "wire": "cursor", "ident": ["limit"], "ty": string(), "presence": "optional" }]
                },
                "PagedQuery": {
                    "kind": "struct",
                    "name": ["paged", "query"],
                    "direction": "input",
                    "openness": "reject",
                    "fields": [{ "wire": "limit", "ident": ["limit"], "ty": count(), "presence": "optional" }],
                    "flattened": [{ "ref": "PageMeta" }]
                }
            })),
            ref_to("PagedQuery"),
        ),
    );
}
