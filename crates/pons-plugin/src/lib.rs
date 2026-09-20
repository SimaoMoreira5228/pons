mod process;
mod protocol;

pub use process::{PluginError, PluginResult, invoke};
pub use protocol::{
    CapabilityWarning, DiagnosticSeverity, EmitRequest, EmitResponse, EmitterCapabilities,
    EmitterManifest, GeneratedFile, PROTOCOL_VERSION, PluginDiagnostic,
};
