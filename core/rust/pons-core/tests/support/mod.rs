#![allow(dead_code)]

use pons_core::TypeCx;
use pons_ir::{DiagnosticSeverity, Document, IR_VERSION, Info, TypeKind, TypeRef};
use std::collections::BTreeMap;

#[derive(pons_core::Schema)]
#[doc = "Request payload for an account login."]
pub struct LoginRequest {
    #[doc = "Account email address."]
    pub email: String,
    pub nickname: Option<String>,
}

#[derive(pons_core::Schema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LoginState {
    Ready,
    Locked { retry_after: u32 },
}

#[derive(pons_core::Schema)]
pub enum ExternalShape {
    Plain,
    #[serde(skip_serializing)]
    InputOnly {
        value: String,
    },
}

#[derive(pons_core::Schema)]
pub struct DirectionalFields {
    #[serde(
        rename(serialize = "from_server", deserialize = "to_server"),
        alias = "legacy_server"
    )]
    pub value: String,
    #[serde(default)]
    pub count: u8,
    #[serde(skip_serializing)]
    pub input_only: bool,
}

#[derive(pons_core::Schema)]
pub struct AuditFields {
    pub created_by: String,
}

#[derive(pons_core::Schema)]
pub struct FlattenedStruct {
    pub id: u64,
    #[serde(flatten)]
    pub audit: AuditFields,
}

#[derive(pons_core::Schema)]
pub struct FlattenedMap {
    #[serde(flatten)]
    pub attributes: std::collections::HashMap<String, String>,
}

#[derive(pons_core::Schema)]
#[allow(dead_code)]
pub struct SharedBoth {
    value: String,
    count: u8,
}

pub fn document(
    types: BTreeMap<String, pons_ir::TypeDef>,
    diagnostics: Vec<pons_ir::Diagnostic>,
) -> Document {
    Document {
        ir_version: IR_VERSION.to_owned(),
        info: Info {
            title: "Schema test".into(),
            version: "1".into(),
            description: None,
            servers: Vec::new(),
            ext: BTreeMap::new(),
        },
        types,
        auth_schemes: BTreeMap::new(),
        common_errors: Vec::new(),
        operations: Vec::new(),
        ext: BTreeMap::new(),
        diagnostics,
    }
}

pub fn struct_shape<'cx>(
    cx: &'cx TypeCx,
    reference: &TypeRef,
) -> (&'cx [pons_ir::Field], &'cx [TypeRef]) {
    let TypeRef::Named(named) = reference else {
        panic!()
    };
    let TypeKind::Struct {
        fields, flattened, ..
    } = &cx.definitions()[&named.id].definition
    else {
        panic!()
    };
    (fields, flattened)
}

pub fn error_coded<'a>(cx: &'a TypeCx, code: &str, needle: &str) -> &'a pons_ir::Diagnostic {
    cx.diagnostics()
        .iter()
        .find(|diagnostic| {
            diagnostic.code == code
                && diagnostic.severity == DiagnosticSeverity::Error
                && diagnostic.message.contains(needle)
        })
        .unwrap_or_else(|| panic!("{code} mentioning {needle} in {:?}", cx.diagnostics()))
}
