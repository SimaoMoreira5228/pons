use pons_ir::{
    ApiKeyLocation, AuthScheme, ContentKind, Direction, ErrorContent, ErrorStatus, InlineType,
    Method, NamedRef, OAuthFlowKind, ObjectOpenness, PathSegment, Presence, ResponseStatus,
    TypeKind, TypeRef, VariantPayload, operation_path,
};

const FIXTURE: &str = include_str!("../testdata/orders.json");
const REFS_FIXTURE: &str = include_str!("../testdata/orders-refs.json");
const FIXTURE_30: &str = include_str!("../testdata/orders-30.json");
const YAML_FIXTURE: &str = include_str!("../testdata/orders.yaml");

fn document() -> pons_ir::Document {
    pons_openapi::import(FIXTURE.as_bytes()).expect("the fixture imports")
}

fn refs_document() -> pons_ir::Document {
    pons_openapi::import(REFS_FIXTURE.as_bytes()).expect("the reference fixture imports")
}

fn yaml_document() -> pons_ir::Document {
    pons_openapi::import(YAML_FIXTURE.as_bytes()).expect("the YAML fixture imports")
}

fn document_30() -> pons_ir::Document {
    pons_openapi::import(FIXTURE_30.as_bytes()).expect("the 3.0 fixture imports")
}

fn operation<'a>(document: &'a pons_ir::Document, label: &str) -> &'a pons_ir::Operation {
    document
        .operations
        .iter()
        .find(|operation| operation_path(&operation.id) == label)
        .unwrap_or_else(|| panic!("no operation {label}"))
}

fn named(ty: &TypeRef) -> &NamedRef {
    match ty {
        TypeRef::Named(named) => named,
        TypeRef::Inline(inline) => panic!("expected a named reference, found {inline:?}"),
    }
}

fn structure(definition: &TypeKind) -> (&[pons_ir::Field], &ObjectOpenness) {
    match definition {
        TypeKind::Struct {
            fields, openness, ..
        } => (fields, openness),
        other => panic!("expected a struct, found {other:?}"),
    }
}

#[test]
fn imports_info_servers_and_extensions() {
    let document = document();
    assert_eq!(document.info.title, "Orders");
    assert_eq!(document.info.version, "1.0.0");
    assert_eq!(
        document.info.description.as_deref(),
        Some("Order management API.")
    );
    assert_eq!(
        document.info.servers,
        ["https://api.example.test", "https://sandbox.example.test"]
    );
    assert_eq!(document.info.ext["openapi.x.internal/v1"]["team"], "sales");
    assert_eq!(document.ext["openapi.x.audience/v1"], "public");
}

#[test]
fn maps_operations_paths_and_query_fields() {
    let document = document();
    let get = operation(&document, "orders.get");

    assert_eq!(get.method, Method::Get);
    assert_eq!(get.id, vec![vec!["orders"], vec!["get"]]);
    assert_eq!(get.path.len(), 2);
    match &get.path[0] {
        PathSegment::Literal { lit } => assert_eq!(lit, "orders"),
        other => panic!("expected a literal segment, found {other:?}"),
    }
    match &get.path[1] {
        PathSegment::Param { param, ty } => {
            assert_eq!(param, "orderId");
            let reference = named(ty);
            assert_eq!(reference.id, "openapi::orders.get::path-orderid#input");
            let definition = &document.types[&reference.id];
            assert_eq!(definition.name, ["order", "id"]);
            assert_eq!(definition.direction, Direction::Input);
            let TypeKind::Newtype { inner } = &definition.definition else {
                panic!("expected a newtype, found {:?}", definition.definition);
            };
            assert_eq!(
                inner,
                &TypeRef::Inline(InlineType::String {
                    format: Some(pons_ir::FieldFormat::Uuid),
                    min_length: None,
                    max_length: None,
                    pattern: None,
                })
            );
        }
        other => panic!("expected a parameter segment, found {other:?}"),
    }

    let query = get.query.as_ref().expect("GET declares query parameters");
    let reference = named(query);
    assert_eq!(reference.id, "openapi::orders.get::query#input");
    let definition = &document.types[&reference.id];
    assert_eq!(definition.direction, Direction::Input);
    let (fields, openness) = structure(&definition.definition);
    assert_eq!(openness, &ObjectOpenness::Reject);
    assert_eq!(fields.len(), 2);

    let expand = fields.iter().find(|field| field.wire == "expand").unwrap();
    assert_eq!(expand.presence, Presence::Optional);
    assert!(matches!(
        expand.ty,
        TypeRef::Inline(InlineType::String { .. })
    ));

    let include_items = fields
        .iter()
        .find(|field| field.wire == "includeItems")
        .unwrap();
    assert_eq!(include_items.presence, Presence::Required);
    assert_eq!(include_items.ty, TypeRef::Inline(InlineType::Bool));

    assert_eq!(get.headers.len(), 1);
    assert_eq!(get.headers[0].wire, "X-Request-ID");
    assert_eq!(get.headers[0].ident, ["x", "request", "id"]);
    assert_eq!(get.headers[0].presence, Presence::Required);

    let doc = get.doc.as_ref().expect("the operation carries a doc");
    assert_eq!(doc.summary.as_deref(), Some("Fetch an order"));
    assert_eq!(doc.description.as_deref(), Some("Fetch one order by id."));
    assert_eq!(doc.deprecated, Some(false));
    assert_eq!(doc.tags, ["orders"]);
}

#[test]
fn maps_the_json_body_and_the_created_response() {
    let document = document();
    let post = operation(&document, "orders.create");
    assert_eq!(post.method, Method::Post);

    let body = post.body.as_ref().expect("POST declares a body");
    assert_eq!(body.content, ContentKind::Json);
    assert!(body.required);
    assert!(body.media.is_none());
    assert_eq!(
        body.ty,
        TypeRef::Named(NamedRef {
            id: "openapi::CreateOrder#input".into(),
            variant: None,
        })
    );

    assert_eq!(post.responses.len(), 1);
    let response = &post.responses[0];
    assert_eq!(response.status, ResponseStatus::Exact(201));
    assert_eq!(response.content, ContentKind::Json);
    assert_eq!(
        response.body,
        Some(TypeRef::Named(NamedRef {
            id: "openapi::Order#output".into(),
            variant: None,
        }))
    );

    let create = &document.types["openapi::CreateOrder#input"];
    let (fields, _) = structure(&create.definition);
    let customer = fields
        .iter()
        .find(|field| field.wire == "customer")
        .unwrap();
    assert_eq!(
        customer.ty,
        TypeRef::Named(NamedRef {
            id: "openapi::Customer#input".into(),
            variant: None,
        })
    );
    assert!(
        document.types.contains_key("openapi::Customer#input"),
        "a component used by a request is lowered in the input direction"
    );
}

#[test]
fn maps_errors_status_names_and_headers() {
    let document = document();
    let get = operation(&document, "orders.get");
    assert_eq!(get.errors.len(), 1);
    let error = &get.errors[0];

    assert_eq!(error.status, ErrorStatus::Exact(404));
    assert_eq!(error.name, ["get", "error", "not_found"]);
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
    assert_eq!(
        error
            .doc
            .as_ref()
            .and_then(|doc| doc.description.as_deref()),
        Some("No order has that id.")
    );
}

#[test]
fn maps_auth_alternatives_and_scopes() {
    let document = document();
    let get = operation(&document, "orders.get");
    assert_eq!(get.auth.alternatives, vec![vec!["bearerAuth"]]);
    assert!(get.auth.scopes.is_empty());
    assert!(!get.auth.optional);

    let cancel = operation(&document, "orders.cancel");
    assert_eq!(
        cancel.auth.alternatives,
        vec![vec!["bearerAuth"], vec!["oauth"]]
    );
    assert_eq!(cancel.auth.scopes["oauth"], ["orders:read"]);
    assert!(!cancel.auth.optional);

    let check = operation(&document, "orders.check");
    assert!(check.auth.alternatives.is_empty());
    assert!(!check.auth.optional);

    assert_eq!(
        document.auth_schemes["bearerAuth"],
        AuthScheme::Bearer {
            format: Some("JWT".into()),
        }
    );
    let AuthScheme::OAuth2 { flows } = &document.auth_schemes["oauth"] else {
        panic!("expected an oauth2 scheme");
    };
    assert_eq!(flows.len(), 1);
    assert_eq!(flows[0].kind, OAuthFlowKind::ClientCredentials);
    assert_eq!(
        flows[0].token_url.as_deref(),
        Some("https://auth.example.test/token")
    );
    assert_eq!(flows[0].scopes["orders:read"], "Read orders");
    assert_eq!(
        document.auth_schemes.get("missing"),
        None,
        "the fixture only declares the schemes it uses"
    );
}

#[test]
fn lowers_components_per_direction() {
    let document = document();
    let order = &document.types["openapi::Order#output"];
    assert_eq!(order.name, ["order"]);
    assert_eq!(order.direction, Direction::Output);
    let (fields, openness) = structure(&order.definition);
    assert_eq!(openness, &ObjectOpenness::Reject);

    let customer = fields
        .iter()
        .find(|field| field.wire == "customer")
        .unwrap();
    assert_eq!(
        customer.ty,
        TypeRef::Named(NamedRef {
            id: "openapi::Customer#output".into(),
            variant: None,
        })
    );

    let note = fields.iter().find(|field| field.wire == "note").unwrap();
    assert_eq!(note.presence, Presence::Optional);
    let TypeRef::Inline(InlineType::Nullable { inner }) = &note.ty else {
        panic!("expected a nullable type, found {:?}", note.ty);
    };
    assert!(matches!(
        inner.as_ref(),
        TypeRef::Inline(InlineType::String { .. })
    ));

    let reference = fields
        .iter()
        .find(|field| field.wire == "reference")
        .unwrap();
    assert_eq!(reference.presence, Presence::Optional);
    assert!(matches!(
        reference.ty,
        TypeRef::Inline(InlineType::String { .. })
    ));

    let payment = fields.iter().find(|field| field.wire == "payment").unwrap();
    let TypeRef::Inline(InlineType::Opaque { codec, .. }) = &payment.ty else {
        panic!("expected an opaque type, found {:?}", payment.ty);
    };
    assert_eq!(codec, "json-schema:oneOf");

    let status = &document.types["openapi::Status#output"];
    let TypeKind::Enum { repr, variants } = &status.definition else {
        panic!("expected an enum, found {:?}", status.definition);
    };
    assert_eq!(repr, &pons_ir::EnumRepr::String);
    assert!(
        variants
            .iter()
            .all(|variant| matches!(variant.payload, VariantPayload::Unit))
    );
    let wires = variants
        .iter()
        .map(|variant| variant.wire.as_str())
        .collect::<Vec<_>>();
    assert_eq!(wires, ["cancelled", "pending", "shipped"]);
}

#[test]
fn keeps_an_api_key_scheme_its_location() {
    let mut raw: serde_json::Value = serde_json::from_str(FIXTURE).unwrap();
    raw["components"]["securitySchemes"]["key"] = serde_json::json!({
        "type": "apiKey",
        "in": "header",
        "name": "X-API-Key"
    });
    raw["components"]["securitySchemes"]["alias"] = serde_json::json!({
        "$ref": "#/components/securitySchemes/key"
    });
    let document = pons_openapi::import(&serde_json::to_vec(&raw).unwrap()).unwrap();
    assert_eq!(
        document.auth_schemes["key"],
        AuthScheme::ApiKey {
            location: ApiKeyLocation::Header,
            name: "X-API-Key".into(),
        }
    );
    assert_eq!(document.auth_schemes["alias"], document.auth_schemes["key"]);
}

#[test]
fn records_dropped_constructs_and_opaque_types() {
    let document = document();
    let diagnostics = &document.diagnostics;

    let dropped = |needle: &str| {
        diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "PONS-W204"
                && diagnostic.message.contains("orders.cancel")
                && diagnostic.message.contains(needle)
        })
    };
    assert!(dropped("callbacks"), "{diagnostics:#?}");
    assert!(dropped("deepObject"), "{diagnostics:#?}");

    let order_diagnostics = &document.types["openapi::Order#output"].diagnostics;
    assert!(
        order_diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "PONS-W202"),
        "{order_diagnostics:#?}"
    );
}

#[test]
fn the_imported_document_is_valid() {
    let document = document();
    document.validate().expect("the imported document is valid");
}

#[test]
fn json_and_yaml_fixtures_import_the_same_document() {
    let json = document();
    let yaml = yaml_document();

    let operation_ids = |document: &pons_ir::Document| {
        document
            .operations
            .iter()
            .map(|operation| operation.id.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(operation_ids(&json), operation_ids(&yaml));

    let type_ids =
        |document: &pons_ir::Document| document.types.keys().cloned().collect::<Vec<String>>();
    assert_eq!(type_ids(&json), type_ids(&yaml));

    assert_eq!(json.diagnostics, yaml.diagnostics);
    assert_eq!(
        json, yaml,
        "orders.json and orders.yaml must describe the same document"
    );
}

#[test]
fn translates_a_30_document() {
    let document = document_30();
    document
        .validate()
        .expect("the imported 3.0 document is valid");

    let order = &document.types["openapi::Order#output"];
    let (fields, _) = structure(&order.definition);

    let note = fields.iter().find(|field| field.wire == "note").unwrap();
    let TypeRef::Inline(InlineType::Nullable { inner }) = &note.ty else {
        panic!("expected a nullable note, found {:?}", note.ty);
    };
    assert!(matches!(
        inner.as_ref(),
        TypeRef::Inline(InlineType::String { .. })
    ));

    let customer = fields
        .iter()
        .find(|field| field.wire == "customer")
        .unwrap();
    let TypeRef::Inline(InlineType::Nullable { inner }) = &customer.ty else {
        panic!("expected a nullable customer, found {:?}", customer.ty);
    };
    assert_eq!(
        inner.as_ref(),
        &TypeRef::Named(NamedRef {
            id: "openapi::Customer#output".into(),
            variant: None,
        })
    );

    let quantity = fields
        .iter()
        .find(|field| field.wire == "quantity")
        .unwrap();
    let TypeRef::Inline(InlineType::Int {
        bits,
        signed,
        minimum,
        ..
    }) = &quantity.ty
    else {
        panic!("expected an integer quantity, found {:?}", quantity.ty);
    };
    assert_eq!(*bits, 32);
    assert!(signed);
    assert_eq!(minimum.as_ref(), Some(&serde_json::json!(2)));

    let attachment = fields
        .iter()
        .find(|field| field.wire == "attachment")
        .unwrap();
    let TypeRef::Inline(InlineType::Opaque { codec, .. }) = &attachment.ty else {
        panic!("expected an opaque attachment, found {:?}", attachment.ty);
    };
    assert_eq!(codec, "json-schema:type:file");

    let diagnostics = &document.diagnostics;
    let warned = |needle: &str| {
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "PONS-W205" && diagnostic.message.contains(needle))
    };
    assert!(warned("properties.total"), "{diagnostics:#?}");
    assert!(warned("properties.attachment"), "{diagnostics:#?}");
}

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
fn refuses_what_it_cannot_read() {
    let error = pons_openapi::import(b"[]").unwrap_err();
    assert!(error.to_string().contains("a JSON array"), "{error}");

    let error = pons_openapi::import(br#"{"openapi":"2.0"}"#).unwrap_err();
    assert!(error.to_string().contains("2.0"), "{error}");
    assert!(error.to_string().contains("3.0.x and 3.1.x"), "{error}");

    let error = pons_openapi::import(br#"{"info":{"title":"x","version":"1"}}"#).unwrap_err();
    assert!(error.to_string().contains("openapi"), "{error}");
}

#[test]
fn refuses_malformed_json_and_yaml_and_names_both_formats() {
    let error = pons_openapi::import(b"openapi: 3.1.0\ninfo: {title: x\n").unwrap_err();
    assert!(
        matches!(error, pons_openapi::ImportError::Read(_)),
        "{error:?}"
    );
    let message = error.to_string();
    assert!(
        message.contains("the input is not OpenAPI 3.0 or 3.1 JSON or YAML"),
        "{message}"
    );
    assert!(message.contains("JSON:"), "{message}");
    assert!(message.contains("YAML:"), "{message}");
}

#[test]
fn refuses_a_path_parameter_that_is_not_declared() {
    let raw = serde_json::json!({
        "openapi": "3.1.0",
        "info": {"title": "x", "version": "1"},
        "paths": {
            "/things/{id}": {
                "get": {
                    "responses": {"200": {"description": "ok"}}
                }
            }
        }
    });
    let error = pons_openapi::import(&serde_json::to_vec(&raw).unwrap()).unwrap_err();
    assert!(error.to_string().contains("not declared"), "{error}");
}
