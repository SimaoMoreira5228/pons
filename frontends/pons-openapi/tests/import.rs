use pons_ir::{
    Document, Field, NamedRef, ObjectOpenness, Operation, TypeKind, TypeRef, operation_path,
};

#[path = "import/auth.rs"]
mod auth;
#[path = "import/components.rs"]
mod components;
#[path = "import/document.rs"]
mod document;
#[path = "import/info_and_servers.rs"]
mod info_and_servers;
#[path = "import/malformed_input.rs"]
mod malformed_input;
#[path = "import/openapi_30.rs"]
mod openapi_30;
#[path = "import/operations.rs"]
mod operations;
#[path = "import/references.rs"]
mod references;

const FIXTURE: &str = include_str!("../testdata/orders.json");
const REFS_FIXTURE: &str = include_str!("../testdata/orders-refs.json");
const FIXTURE_30: &str = include_str!("../testdata/orders-30.json");
const YAML_FIXTURE: &str = include_str!("../testdata/orders.yaml");

fn document() -> Document {
    pons_openapi::import(FIXTURE.as_bytes()).expect("the fixture imports")
}

fn refs_document() -> Document {
    pons_openapi::import(REFS_FIXTURE.as_bytes()).expect("the reference fixture imports")
}

fn yaml_document() -> Document {
    pons_openapi::import(YAML_FIXTURE.as_bytes()).expect("the YAML fixture imports")
}

fn document_30() -> Document {
    pons_openapi::import(FIXTURE_30.as_bytes()).expect("the 3.0 fixture imports")
}

fn operation<'a>(document: &'a Document, label: &str) -> &'a Operation {
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

fn structure(definition: &TypeKind) -> (&[Field], &ObjectOpenness) {
    match definition {
        TypeKind::Struct {
            fields, openness, ..
        } => (fields, openness),
        other => panic!("expected a struct, found {other:?}"),
    }
}
