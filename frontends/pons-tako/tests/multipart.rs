#![cfg(feature = "multipart")]

use std::collections::BTreeMap;

use pons_core::Schema;
use pons_ir::{ContentKind, InlineType, OpaqueWire, TypeKind, TypeRef};
use pons_tako::{DescribeHandler, DescribeRequest, Info, OperationCx, Router};
use serde::Deserialize;
use tako::extractors::FromRequest;
use tako::extractors::multipart::{InMemoryFile, TakoMultipart, TakoTypedMultipart};
use tako::responder::Responder;
use tako::types::{Request, Response};
use tako_rs_core::handler::Handler;

#[derive(Deserialize, Schema)]
struct UploadForm {
    title: String,
}

// NOTE: Tako's Handler requires one concrete extractor type, but these extractors borrow the
// request. The adapters keep the runtime extraction and give the description a nameable tuple.
#[derive(Clone)]
struct TypedUpload;

impl Handler<(TakoTypedMultipart<'static, UploadForm, InMemoryFile>,)> for TypedUpload {
    async fn call(self, mut request: Request) -> Response {
        match TakoTypedMultipart::<UploadForm, InMemoryFile>::from_request(&mut request).await {
            Ok(upload) => {
                let _ = upload.data.title;
                ().into_response()
            }
            Err(error) => error.into_response(),
        }
    }
}

impl DescribeHandler<(TakoTypedMultipart<'static, UploadForm, InMemoryFile>,)> for TypedUpload {
    fn describe(cx: &mut OperationCx) {
        <TakoTypedMultipart<'static, UploadForm, InMemoryFile> as DescribeRequest>::describe(cx);
    }
}

#[derive(Clone)]
struct UntypedUpload;

impl Handler<(TakoMultipart<'static>,)> for UntypedUpload {
    async fn call(self, mut request: Request) -> Response {
        match TakoMultipart::from_request(&mut request).await {
            Ok(_) => ().into_response(),
            Err(error) => error.into_response(),
        }
    }
}

impl DescribeHandler<(TakoMultipart<'static>,)> for UntypedUpload {
    fn describe(cx: &mut OperationCx) {
        <TakoMultipart<'static> as DescribeRequest>::describe(cx);
    }
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
fn typed_multipart_publishes_the_form_schema_and_names_the_file_type() {
    let mut router = Router::new(info());
    router.post("/upload", TypedUpload);
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

#[test]
fn untyped_multipart_is_an_opaque_body() {
    let mut router = Router::new(info());
    router.post("/upload", UntypedUpload);
    let document = router.into_document().unwrap();
    document.validate().unwrap();

    let operation = &document.operations[0];
    let body = operation.body.as_ref().expect("the upload has a body");
    assert_eq!(body.content, ContentKind::Multipart);
    assert_eq!(
        body.ty,
        TypeRef::Inline(InlineType::Opaque {
            codec: "tako.multipart".into(),
            wire: OpaqueWire::Unknown,
            ext: Default::default(),
        })
    );
    assert!(operation.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "PONS-W101" && diagnostic.message.contains("multipart fields are opaque")
    }));
}
