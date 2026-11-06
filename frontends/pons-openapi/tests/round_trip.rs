use pons_ir::{CredentialLocator, Document, JsonPointer, PaginationRequest, operation_path};

const PAGINATED: &str = include_str!("../../../spec/fixtures/paginated.json");
const LOGIN: &str = include_str!("../../../spec/fixtures/login.json");

fn fixture(source: &str) -> Document {
    serde_json::from_str(source).expect("the fixture decodes")
}

fn round_trip(document: &Document) -> Document {
    let emitted = pons_emit_openapi::emit(document).expect("the fixture emits");
    pons_openapi::import(emitted.as_bytes()).expect("the emitted document imports")
}

fn operation<'a>(document: &'a Document, label: &str) -> &'a pons_ir::Operation {
    document
        .operations
        .iter()
        .find(|operation| operation_path(&operation.id) == label)
        .unwrap_or_else(|| panic!("no operation {label}"))
}

#[test]
fn restores_pagination_and_semantics() {
    let imported = round_trip(&fixture(PAGINATED));
    let operation = operation(&imported, "users.list");
    let pagination = operation
        .semantics
        .pagination
        .as_ref()
        .expect("pagination survives the round trip");

    assert_eq!(
        pagination.request,
        PaginationRequest::Cursor {
            field: "cursor".into()
        }
    );
    assert_eq!(pagination.items.as_str(), "/items");
    assert_eq!(
        pagination.next.as_ref().map(JsonPointer::as_str),
        Some("/next_cursor")
    );
    let page_size = pagination
        .page_size
        .as_ref()
        .expect("the page size survives the round trip");
    assert_eq!(page_size.field, "limit");
    assert_eq!(page_size.default, Some(20));
    assert_eq!(page_size.maximum, Some(100));
    assert!(operation.semantics.safe);
    assert!(operation.semantics.idempotent);

    imported.validate().expect("the imported document is valid");
}

#[test]
fn restores_provides_and_auth_from_x_pons() {
    let imported = round_trip(&fixture(LOGIN));
    let login = operation(&imported, "auth.login");
    let provides = login
        .provides
        .as_ref()
        .expect("the credential provision survives the round trip");

    assert_eq!(provides.scheme, "session");
    assert_eq!(
        provides.value,
        CredentialLocator::Body {
            pointer: "/token".into()
        }
    );
    assert!(provides.expires_at.is_none());
    assert_eq!(
        imported.auth_schemes["session"],
        pons_ir::AuthScheme::Bearer {
            format: Some("jwt".into())
        }
    );

    let events = operation(&imported, "events.watch");
    assert_eq!(events.auth.alternatives, vec![vec!["tenant".to_owned()]]);
    assert!(!events.auth.optional);

    imported.validate().expect("the imported document is valid");
}

#[test]
fn restores_operation_ids_and_error_inheritance() {
    let imported = round_trip(&fixture(LOGIN));
    let ids = imported
        .operations
        .iter()
        .map(|operation| operation.id.clone())
        .collect::<Vec<_>>();
    assert!(ids.contains(&vec![vec!["auth".to_owned()], vec!["login".to_owned()]]));
    assert!(ids.contains(&vec![vec!["events".to_owned()], vec!["watch".to_owned()]]));
    assert!(
        imported
            .operations
            .iter()
            .all(|operation| operation.errors_inherit)
    );

    let paginated = round_trip(&fixture(PAGINATED));
    assert!(!operation(&paginated, "users.list").errors_inherit);

    imported.validate().expect("the imported document is valid");
    paginated
        .validate()
        .expect("the imported document is valid");
}

#[test]
fn restores_exact_operation_id_words() {
    let mut document = fixture(PAGINATED);
    document.operations[0].id = vec![
        vec!["users".into(), "page".into()],
        vec!["list".into(), "all".into()],
    ];
    let imported = round_trip(&document);

    assert_eq!(
        imported.operations[0].id,
        vec![
            vec!["users".to_owned(), "page".to_owned()],
            vec!["list".to_owned(), "all".to_owned()]
        ]
    );
    imported.validate().expect("the imported document is valid");
}

#[test]
fn reports_stream_metadata_it_cannot_restore() {
    let imported = round_trip(&fixture(LOGIN));

    assert!(imported.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "PONS-W204"
            && diagnostic.message.contains("events.watch")
            && diagnostic
                .message
                .contains("stream metadata was not restored")
    }));
}
