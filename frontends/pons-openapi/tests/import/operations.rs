use pons_ir::{
    ContentKind, Direction, ErrorContent, ErrorStatus, InlineType, Method, NamedRef,
    ObjectOpenness, PathSegment, Presence, ResponseStatus, TypeKind, TypeRef,
};

use super::{document, named, operation, structure};

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
