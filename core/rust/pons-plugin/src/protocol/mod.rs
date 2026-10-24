mod capabilities;
mod diagnostic;
mod manifest;
mod response;

use std::collections::BTreeMap;

use pons_ir::Document;
use serde::Serialize;
use serde_json::Value;

pub const PROTOCOL_VERSION: u32 = 3;
pub const LEGACY_PROTOCOL_VERSION: u32 = 2;

pub use capabilities::{CapabilityWarning, validation_enabled};
pub use diagnostic::{DiagnosticSeverity, PluginDiagnostic};
pub use manifest::{
    EmitterCapabilities, EmitterManifest, OptionKind, OptionSchema, OptionSpec,
    ValidationCapabilities,
};
pub(crate) use response::validate_response_for_document;
pub use response::{EmitResponse, GeneratedFile, validate_response};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EmitMode {
    Strict,
    Lossy,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EmitRequest<'a> {
    pub protocol: u32,
    pub source_ir_version: String,
    pub mode: EmitMode,
    pub ir: &'a Document,
    pub options: BTreeMap<String, Value>,
    pub target: &'a str,
}
