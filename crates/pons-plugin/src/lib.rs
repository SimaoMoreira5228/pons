mod pin;
mod process;
mod protocol;

pub use pin::PluginPin;
pub use pons_ir::{Coverage, coverage_for_document, operation_subject, strict_violation_message};
pub use process::{PluginError, PluginResult, invoke, log_tail};
pub use protocol::{
    CapabilityWarning, DiagnosticSeverity, EmitMode, EmitRequest, EmitResponse,
    EmitterCapabilities, EmitterManifest, GeneratedFile, LEGACY_PROTOCOL_VERSION, OptionKind,
    OptionSchema, OptionSpec, PROTOCOL_VERSION, PluginDiagnostic, ValidationCapabilities,
    validate_response, validation_enabled,
};
