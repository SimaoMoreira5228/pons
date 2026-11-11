#![cfg(feature = "multipart")]

use std::collections::BTreeMap;

use pons_core::Schema;
use pons_ir::{ContentKind, TypeKind, TypeRef};
use pons_tako::{Info, PonsMultipart, Router};
use serde::Deserialize;
use tako::extractors::multipart::InMemoryFile;

#[derive(Deserialize, Schema)]
struct UploadForm {
    title: String,
}

async fn upload(PonsMultipart { data, .. }: PonsMultipart<UploadForm, InMemoryFile>) {
    let _ = data.title;
}

fn info() -> Info {
    Info {
        title: "Uploads".into(),
        version: "1".into(),
        description: None,
        servers: Vec::new(),
        ext: BTreeMap::new(),
    }
}

#[test]
fn owned_multipart_registers_directly() {
    let mut router = Router::new(info());
    router.post("/upload", upload);
    let document = router.into_document().unwrap();
    document.validate().unwrap();

    let operation = &document.operations[0];
    let body = operation.body.as_ref().expect("the upload has a body");
    assert_eq!(body.content, ContentKind::Multipart);
    assert!(body.required);
    assert!(body.media.is_none());

    let TypeRef::Named(form) = &body.ty else {
        panic!("{:?} is not a named form type", body.ty)
    };
    assert_eq!(
        form.id,
        format!("{}#input", std::any::type_name::<UploadForm>())
    );
    assert!(matches!(
        document.types[&form.id].definition,
        TypeKind::Struct { .. }
    ));

    let diagnostic = operation
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "PONS-W101")
        .expect("the file field is diagnosed");
    assert!(
        diagnostic
            .message
            .contains(std::any::type_name::<InMemoryFile>()),
        "{} must name the file field type",
        diagnostic.message
    );
    assert!(diagnostic.message.contains("no IR form"));
}
