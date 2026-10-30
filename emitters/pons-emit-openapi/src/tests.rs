mod coverage;
mod document;
mod naming;
mod pagination;

use pons_ir::{Direction, InlineType, ObjectOpenness, TypeDef, TypeKind, TypeRef};

fn fixture() -> pons_ir::Document {
    serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap()
}

fn paginated_fixture() -> pons_ir::Document {
    serde_json::from_str(include_str!("../../../spec/fixtures/paginated.json")).unwrap()
}

fn string() -> TypeRef {
    TypeRef::Inline(InlineType::String {
        format: None,
        min_length: None,
        max_length: None,
        pattern: None,
    })
}

fn named_type(name: &str) -> TypeDef {
    TypeDef {
        name: vec![name.into()],
        direction: Direction::Both,
        definition: TypeKind::Struct {
            fields: Vec::new(),
            flattened: Vec::new(),
            openness: ObjectOpenness::Reject,
        },
        origin: None,
        doc: None,
        diagnostics: Vec::new(),
        ext: Default::default(),
    }
}
