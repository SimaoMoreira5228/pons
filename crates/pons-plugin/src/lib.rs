mod process;
mod protocol;

pub use process::{PluginError, PluginResult, invoke};
pub use protocol::{
    CapabilityWarning, Coverage, DiagnosticSeverity, EmitMode, EmitRequest, EmitResponse,
    EmitterCapabilities, EmitterManifest, GeneratedFile, PROTOCOL_VERSION, PluginDiagnostic,
    coverage_for_document, operation_subject,
};
