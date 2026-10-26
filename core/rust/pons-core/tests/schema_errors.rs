mod support;

use pons_core::TypeCx;
use pons_ir::{TypeKind, TypeRef};

#[derive(pons_core::Schema, pons_core::ApiError)]
#[serde(tag = "code", rename_all = "snake_case")]
enum LoginError {
    #[pons(status = 401)]
    InvalidCredentials,
    #[pons(status = 423, retryable = true)]
    Locked { retry_after: u32 },
}

#[derive(pons_core::Schema, pons_core::ApiError)]
enum ContentError {
    #[pons(status = 400, content = "text")]
    Text,
    #[pons(status = 400, content = "empty")]
    Empty,
}

#[derive(pons_core::Schema, pons_core::ApiError)]
enum StatusError {
    #[pons(status = "4xx")]
    Client,
    #[pons(status = "default")]
    Fallback,
}

#[test]
fn api_errors_register_typed_variant_responses() {
    let mut cx = TypeCx::new();
    let errors = <LoginError as pons_core::ErrorSpec>::errors(&mut cx);
    assert_eq!(errors.len(), 2);
    assert_eq!(errors[0].status, pons_ir::ErrorStatus::Exact(401));
    assert_eq!(errors[0].name, ["invalid", "credentials"]);
    assert_eq!(errors[1].status, pons_ir::ErrorStatus::Exact(423));
    assert_eq!(errors[1].name, ["locked"]);
    assert_eq!(errors[1].retryable, Some(true));
    let TypeRef::Named(body) = errors[1].body.as_ref().unwrap() else {
        panic!()
    };
    assert_eq!(body.variant.as_deref(), Some("locked"));
    assert!(matches!(
        &cx.definitions()[&body.id].definition,
        TypeKind::Enum { variants, .. } if variants.len() == 2
    ));
    let _ = LoginError::InvalidCredentials;
    let LoginError::Locked { retry_after } = (LoginError::Locked { retry_after: 1 }) else {
        panic!()
    };
    assert_eq!(retry_after, 1);
}

#[test]
fn api_error_honors_declared_content() {
    let mut cx = TypeCx::new();
    let errors = <ContentError as pons_core::ErrorSpec>::errors(&mut cx);
    assert_eq!(errors[0].content, pons_ir::ErrorContent::Text);
    assert!(errors[0].body.is_some());
    assert_eq!(errors[1].content, pons_ir::ErrorContent::Empty);
    assert!(errors[1].body.is_none());
    let _ = ContentError::Text;
    let _ = ContentError::Empty;
}

#[test]
fn api_error_accepts_status_classes_and_default() {
    let mut cx = TypeCx::new();
    let errors = <StatusError as pons_core::ErrorSpec>::errors(&mut cx);
    assert_eq!(errors[0].status, pons_ir::ErrorStatus::Class(4));
    assert_eq!(errors[1].status, pons_ir::ErrorStatus::Default);
    let _ = StatusError::Client;
    let _ = StatusError::Fallback;
}
