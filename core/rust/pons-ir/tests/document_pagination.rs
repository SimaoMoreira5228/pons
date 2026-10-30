mod support;

use pons_ir::Document;
use serde_json::{Value, json};
use support::refuses;

fn paginated(request: Value, pagination: Value) -> Document {
    serde_json::from_value(json!({
        "ir_version": "0.2",
        "info": {"title": "Example", "version": "1"},
        "types": {
            "EventQuery": {
                "kind": "struct", "name": ["event", "query"], "direction": "input", "openness": "reject",
                "fields": [
                    {"wire": "cursor", "ident": ["cursor"], "ty": {"kind": "string"}, "presence": "optional"},
                    {"wire": "offset", "ident": ["offset"], "ty": {"kind": "int", "bits": 32, "signed": false, "wire": "number"}, "presence": "optional"},
                    {"wire": "limit", "ident": ["limit"], "ty": {"kind": "int", "bits": 16, "signed": false, "wire": "number"}, "presence": "optional"}
                ]
            },
            "EventPage": {
                "kind": "struct", "name": ["event", "page"], "direction": "output", "openness": "reject",
                "fields": [
                    {"wire": "items", "ident": ["items"], "ty": {"kind": "array", "item": {"kind": "string"}}, "presence": "required"},
                    {"wire": "next", "ident": ["next"], "ty": {"kind": "nullable", "inner": {"kind": "string"}}, "presence": "optional"}
                ]
            }
        },
        "operations": [{
            "id": [["events"], ["list"]],
            "method": "GET",
            "path": [{"lit": "events"}],
            "query": request,
            "responses": [{"status": 200, "content": "json", "body": {"ref": "EventPage"}}],
            "auth": {"alternatives": [], "optional": false},
            "semantics": {"safe": true, "idempotent": true, "pagination": pagination}
        }]
    }))
    .unwrap()
}

#[test]
fn a_cursor_page_records_its_request_field_and_locators() {
    let cursor = paginated(
        json!({"ref": "EventQuery"}),
        json!({
            "request": {"kind": "cursor", "field": "cursor"},
            "items": "/items",
            "next": "/next",
            "page_size": {"field": "limit", "default": 20, "maximum": 100}
        }),
    );
    cursor.validate().unwrap();

    let offset = paginated(
        json!({"ref": "EventQuery"}),
        json!({
            "request": {"kind": "offset", "field": "offset"},
            "items": "/items"
        }),
    );
    offset.validate().unwrap();
}

#[test]
fn a_page_fields_must_name_the_operation_query_or_body() {
    let missing = paginated(
        json!({"ref": "EventQuery"}),
        json!({
            "request": {"kind": "cursor", "field": "after"},
            "items": "/items",
            "next": "/next"
        }),
    );
    refuses(
        &missing,
        "pagination field \"after\" is not declared on the operation query or body",
    );

    let missing_size = paginated(
        json!({"ref": "EventQuery"}),
        json!({
            "request": {"kind": "offset", "field": "offset"},
            "items": "/items",
            "page_size": {"field": "per_page"}
        }),
    );
    refuses(
        &missing_size,
        "pagination field \"per_page\" is not declared on the operation query or body",
    );
}

#[test]
fn pagination_locators_are_json_pointers() {
    let bad_items = paginated(
        json!({"ref": "EventQuery"}),
        json!({
            "request": {"kind": "cursor", "field": "cursor"},
            "items": "items",
            "next": "/next"
        }),
    );
    refuses(
        &bad_items,
        "pagination item locator: a JSON pointer must be empty or start with '/'",
    );

    let bad_next = paginated(
        json!({"ref": "EventQuery"}),
        json!({
            "request": {"kind": "cursor", "field": "cursor"},
            "items": "/items",
            "next": "/next~2"
        }),
    );
    refuses(
        &bad_next,
        "pagination next locator: a JSON pointer has an invalid escape",
    );

    let empty = paginated(
        json!({"ref": "EventQuery"}),
        json!({
            "request": {"kind": "cursor", "field": "cursor"},
            "items": "",
            "next": "/next"
        }),
    );
    empty.validate().unwrap();
}

#[test]
fn a_cursor_request_without_a_next_pointer_is_refused() {
    let document = paginated(
        json!({"ref": "EventQuery"}),
        json!({
            "request": {"kind": "cursor", "field": "cursor"},
            "items": "/items"
        }),
    );
    refuses(&document, "a cursor request needs a next pointer");
}

#[test]
fn a_cursor_request_with_a_next_pointer_is_accepted() {
    let document = paginated(
        json!({"ref": "EventQuery"}),
        json!({
            "request": {"kind": "cursor", "field": "cursor"},
            "items": "/items",
            "next": "/next"
        }),
    );
    document.validate().unwrap();
}

fn offset_field(ty: Value) -> Document {
    let mut document = paginated(
        json!({"ref": "EventQuery"}),
        json!({
            "request": {"kind": "offset", "field": "offset"},
            "items": "/items"
        }),
    );
    let pons_ir::TypeKind::Struct { fields, .. } =
        &mut document.types.get_mut("EventQuery").unwrap().definition
    else {
        unreachable!()
    };
    let field = fields
        .iter_mut()
        .find(|field| field.wire == "offset")
        .unwrap();
    field.ty = serde_json::from_value(ty).unwrap();
    document
}

#[test]
fn an_offset_request_cannot_declare_a_next_pointer() {
    let mut document = offset_field(json!({
        "kind": "int", "bits": 32, "signed": false, "wire": "number"
    }));
    document.operations[0]
        .semantics
        .pagination
        .as_mut()
        .unwrap()
        .next = Some(pons_ir::JsonPointer::new("/next"));
    refuses(&document, "an offset request cannot declare a next pointer");
}

#[test]
fn an_offset_field_can_be_a_plain_integer() {
    let document = offset_field(json!({
        "kind": "int", "bits": 32, "signed": false, "wire": "number"
    }));
    document.validate().unwrap();
}

#[test]
fn an_offset_field_can_be_a_newtype_over_an_integer() {
    let mut document = offset_field(json!({"ref": "Offset"}));
    document.types.insert(
        "Offset".into(),
        serde_json::from_value(json!({
            "kind": "newtype",
            "name": ["offset"],
            "direction": "input",
            "inner": {"kind": "int", "bits": 64, "signed": false, "wire": "string"}
        }))
        .unwrap(),
    );
    document.validate().unwrap();
}

#[test]
fn an_offset_field_cannot_be_a_string() {
    let document = offset_field(json!({"kind": "string"}));
    refuses(
        &document,
        "offset pagination field \"offset\" must be an integer",
    );
}

#[test]
fn an_offset_field_cannot_be_a_nullable_integer() {
    let document = offset_field(json!({
        "kind": "nullable",
        "inner": {"kind": "int", "bits": 32, "signed": false, "wire": "number"}
    }));
    refuses(
        &document,
        "offset pagination field \"offset\" must be an integer",
    );
}

#[test]
fn a_page_size_default_cannot_exceed_its_maximum() {
    let invalid = paginated(
        json!({"ref": "EventQuery"}),
        json!({
            "request": {"kind": "offset", "field": "offset"},
            "items": "/items",
            "page_size": {"field": "limit", "default": 50, "maximum": 10}
        }),
    );
    refuses(
        &invalid,
        "pagination page size default 50 exceeds its maximum 10",
    );

    let zero = paginated(
        json!({"ref": "EventQuery"}),
        json!({
            "request": {"kind": "offset", "field": "offset"},
            "items": "/items",
            "page_size": {"field": "limit", "default": 0}
        }),
    );
    refuses(&zero, "pagination page size default must be positive");
}

#[test]
fn a_pagination_field_can_live_on_the_request_body() {
    let mut document = paginated(
        json!({"ref": "EventQuery"}),
        json!({
            "request": {"kind": "offset", "field": "offset"},
            "items": "/items"
        }),
    );
    {
        let operation = &mut document.operations[0];
        operation.method = pons_ir::Method::Post;
        operation.query = None;
        operation.body = Some(
            serde_json::from_value(json!({
                "ty": {"ref": "EventQuery"},
                "content": "json",
                "required": true
            }))
            .unwrap(),
        );
    }
    document.validate().unwrap();
}
