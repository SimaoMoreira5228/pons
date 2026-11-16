use super::{document, yaml_document};

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
