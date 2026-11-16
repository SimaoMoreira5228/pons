use pons_ir::{ErrorContent, ErrorStatus, InlineType, NamedRef, Presence, ResponseStatus, TypeRef};

use super::{named, operation, refs_document, structure};

#[test]
fn follows_component_references_across_operations() {
    let document = refs_document();
    document
        .validate()
        .expect("the imported reference document is valid");

    let list = operation(&document, "widgets.list");
    let create = operation(&document, "widgets.create");

    let query = |operation: &pons_ir::Operation| {
        let reference = named(operation.query.as_ref().expect("the query is present"));
        let (fields, _) = structure(&document.types[&reference.id].definition);
        fields
            .iter()
            .find(|field| field.wire == "page")
            .expect("the shared page parameter resolves")
            .clone()
    };
    let listed_page = query(list);
    let created_page = query(create);
    assert_eq!(listed_page.presence, Presence::Optional);
    assert_eq!(created_page.presence, Presence::Optional);
    assert!(matches!(
        listed_page.ty,
        TypeRef::Inline(InlineType::Int { .. })
    ));

    let not_found = |operation: &pons_ir::Operation| {
        operation
            .errors
            .iter()
            .find(|error| error.status == ErrorStatus::Exact(404))
            .expect("the shared NotFound response resolves")
            .clone()
    };
    for error in [not_found(list), not_found(create)] {
        assert_eq!(error.content, ErrorContent::Json);
        assert_eq!(
            error.body,
            Some(TypeRef::Named(NamedRef {
                id: "openapi::Error#output".into(),
                variant: None,
            }))
        );
        assert_eq!(error.headers.len(), 1);
        assert_eq!(error.headers[0].wire, "Retry-After");
        assert_eq!(error.headers[0].presence, Presence::Optional);
    }

    let body = create
        .body
        .as_ref()
        .expect("the shared request body resolves");
    assert!(body.required);
    assert_eq!(
        body.ty,
        TypeRef::Named(NamedRef {
            id: "openapi::NewWidget#input".into(),
            variant: None,
        })
    );

    let created = create
        .responses
        .first()
        .expect("the created response is present");
    assert_eq!(created.status, ResponseStatus::Exact(201));
    assert_eq!(
        created.body,
        Some(TypeRef::Named(NamedRef {
            id: "openapi::Widget#output".into(),
            variant: None,
        }))
    );
    assert_eq!(created.headers.len(), 1);
    assert_eq!(created.headers[0].wire, "Location");

    for label in ["widgets_status.get", "status.get"] {
        let status = operation(&document, label);
        assert_eq!(status.responses.len(), 1);
        assert_eq!(
            status.responses[0].body,
            Some(TypeRef::Named(NamedRef {
                id: "openapi::Status#output".into(),
                variant: None,
            })),
            "{label} comes from the referenced path item"
        );
    }

    let loops = operation(&document, "loops.get");
    assert!(loops.query.is_none(), "the cyclic parameter is dropped");
    assert!(document.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "PONS-W204"
            && diagnostic.message.contains("loops.get")
            && diagnostic.message.contains("#/components/parameters/LoopA")
            && diagnostic.message.contains("cycle")
    }));
}

#[test]
fn reports_missing_and_non_local_references() {
    let raw = serde_json::json!({
        "openapi": "3.1.0",
        "info": {"title": "x", "version": "1"},
        "paths": {
            "/things": {
                "get": {
                    "parameters": [
                        {"$ref": "#/components/parameters/Missing"},
                        {"$ref": "other.yaml#/components/parameters/Remote"}
                    ],
                    "responses": {"200": {"description": "ok"}}
                }
            }
        }
    });
    let document = pons_openapi::import(&serde_json::to_vec(&raw).unwrap())
        .expect("the broken references do not abort the import");
    document
        .validate()
        .expect("the rest of the document is valid");

    let get = operation(&document, "things.get");
    assert!(get.query.is_none());

    let reported = |needle: &str| {
        document.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "PONS-W204"
                && diagnostic.message.contains("things.get")
                && diagnostic.message.contains(needle)
        })
    };
    assert!(reported("#/components/parameters/Missing"));
    assert!(reported("other.yaml#/components/parameters/Remote"));
}

#[test]
fn decodes_percent_escapes_in_reference_tokens() {
    let raw = serde_json::json!({
        "openapi": "3.1.0",
        "info": {"title": "x", "version": "1"},
        "paths": {
            "/things": {
                "get": {
                    "operationId": "things.list",
                    "parameters": [
                        {"$ref": "#/components/parameters/Page%20Size"},
                        {"$ref": "#/components/parameters/Caf%C3%A9"}
                    ],
                    "responses": {"200": {"description": "ok"}}
                }
            }
        },
        "components": {
            "parameters": {
                "Page Size": {
                    "name": "pageSize",
                    "in": "query",
                    "required": false,
                    "schema": {"type": "integer", "format": "int32"}
                },
                "Café": {
                    "name": "cafe",
                    "in": "query",
                    "required": false,
                    "schema": {"type": "string"}
                }
            }
        }
    });
    let document = pons_openapi::import(&serde_json::to_vec(&raw).unwrap())
        .expect("the percent-encoded references resolve");
    document.validate().expect("the imported document is valid");

    let get = operation(&document, "things.list");
    let reference = named(get.query.as_ref().expect("the query is present"));
    let (fields, _) = structure(&document.types[&reference.id].definition);
    let wires = fields
        .iter()
        .map(|field| field.wire.as_str())
        .collect::<Vec<_>>();
    assert_eq!(wires, ["cafe", "pageSize"], "{fields:#?}");
}

#[test]
fn reports_malformed_reference_escapes() {
    let raw = serde_json::json!({
        "openapi": "3.1.0",
        "info": {"title": "x", "version": "1"},
        "paths": {
            "/things": {
                "get": {
                    "operationId": "things.list",
                    "parameters": [
                        {"$ref": "#/components/parameters/Bad%ZZ"},
                        {"$ref": "#/components/parameters/Bad~2"},
                        {"$ref": "#/components/parameters/Bad%FF"}
                    ],
                    "responses": {"200": {"description": "ok"}}
                }
            }
        },
        "components": {"parameters": {}}
    });
    let document = pons_openapi::import(&serde_json::to_vec(&raw).unwrap())
        .expect("malformed references do not abort the import");
    document
        .validate()
        .expect("the rest of the document is valid");

    let get = operation(&document, "things.list");
    assert!(get.query.is_none(), "the malformed references are dropped");

    let reported = |needle: &str| {
        document.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "PONS-W204"
                && diagnostic.message.contains("things.list")
                && diagnostic.message.contains(needle)
        })
    };
    assert!(reported("Bad%ZZ"), "{:#?}", document.diagnostics);
    assert!(reported("Bad~2"), "{:#?}", document.diagnostics);
    assert!(reported("Bad%FF"), "{:#?}", document.diagnostics);
    assert!(
        reported("malformed percent escape"),
        "{:#?}",
        document.diagnostics
    );
    assert!(
        reported("malformed pointer escape"),
        "{:#?}",
        document.diagnostics
    );
    assert!(
        reported("does not decode to UTF-8"),
        "{:#?}",
        document.diagnostics
    );
}

#[test]
fn reports_a_path_item_reference_cycle_once() {
    let raw = serde_json::json!({
        "openapi": "3.1.0",
        "info": {"title": "x", "version": "1"},
        "paths": {
            "/cycle": {"$ref": "#/components/pathItems/CycleA"},
            "/things": {
                "get": {
                    "operationId": "things.list",
                    "responses": {"200": {"description": "ok"}}
                }
            }
        },
        "components": {
            "pathItems": {
                "CycleA": {"$ref": "#/components/pathItems/CycleB"},
                "CycleB": {"$ref": "#/components/pathItems/CycleA"}
            }
        }
    });
    let document = pons_openapi::import(&serde_json::to_vec(&raw).unwrap())
        .expect("the path item cycle does not abort the import");
    document
        .validate()
        .expect("the rest of the document is valid");

    let get = operation(&document, "things.list");
    assert_eq!(get.responses.len(), 1);

    let cycles = document
        .diagnostics
        .iter()
        .filter(|diagnostic| {
            diagnostic.code == "PONS-W204" && diagnostic.message.contains("reference cycle")
        })
        .collect::<Vec<_>>();
    assert_eq!(cycles.len(), 1, "{:#?}", document.diagnostics);
    assert!(cycles[0].message.contains("/cycle"));
    assert!(cycles[0].message.contains("CycleA"));
}

#[test]
fn reports_reference_targets_that_are_not_objects() {
    let raw = serde_json::json!({
        "openapi": "3.1.0",
        "info": {"title": "x", "version": "1"},
        "paths": {
            "/broken": {"$ref": "#/components/pathItems/Broken"},
            "/things": {
                "get": {
                    "operationId": "things.list",
                    "responses": {
                        "200": {"$ref": "#/components/responses/Broken"},
                        "500": {"description": "Server error."}
                    }
                }
            }
        },
        "components": {
            "pathItems": {"Broken": "not an object"},
            "responses": {"Broken": ["not", "an", "object"]}
        }
    });
    let document = pons_openapi::import(&serde_json::to_vec(&raw).unwrap())
        .expect("non-object reference targets do not abort the import");
    document
        .validate()
        .expect("the rest of the document is valid");

    let reported = |needle: &str| {
        document
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "PONS-W204" && diagnostic.message.contains(needle))
    };
    assert!(
        reported("path item \"/broken\" is not an object"),
        "{:#?}",
        document.diagnostics
    );
    assert!(
        reported("the 200 response is not an object"),
        "{:#?}",
        document.diagnostics
    );
}
