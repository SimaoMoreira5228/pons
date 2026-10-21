mod process;
mod protocol;

pub use pons_ir::{Coverage, coverage_for_document, operation_subject, strict_violation_message};
pub use process::{PluginError, PluginResult, invoke, log_tail};
pub use protocol::{
    CapabilityWarning, DiagnosticSeverity, EmitMode, EmitRequest, EmitResponse,
    EmitterCapabilities, EmitterManifest, GeneratedFile, OptionKind, OptionSchema, OptionSpec,
    PROTOCOL_VERSION, PluginDiagnostic, ValidationCapabilities, validation_enabled,
};
