use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::io::Write;
use std::process::{Command, Stdio};

use pons_ir::Document;
use serde_json::Value;

use crate::protocol::{
    EmitRequest, EmitResponse, EmitterManifest, PROTOCOL_VERSION, validate_response,
};

#[derive(Debug)]
pub struct PluginError(pub String);

impl std::fmt::Display for PluginError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for PluginError {}

#[derive(Debug)]
pub struct PluginResult {
    pub manifest: EmitterManifest,
    pub response: EmitResponse,
    pub capability_warnings: Vec<String>,
    pub logs: Vec<u8>,
}

pub fn invoke(
    executable: &OsStr,
    target: &str,
    ir: &Document,
    options: BTreeMap<String, Value>,
    strict: bool,
) -> Result<PluginResult, PluginError> {
    let manifest_output = Command::new(executable)
        .arg("describe")
        .output()
        .map_err(|error| {
            PluginError(format!(
                "cannot start {}: {error}",
                executable.to_string_lossy()
            ))
        })?;
    ensure_success(
        executable,
        "describe",
        &manifest_output.status,
        &manifest_output.stderr,
    )?;
    let manifest: EmitterManifest =
        serde_json::from_slice(&manifest_output.stdout).map_err(|error| {
            PluginError(format!(
                "{} returned an invalid manifest: {error}",
                executable.to_string_lossy()
            ))
        })?;
    if manifest.name != target {
        return Err(PluginError(format!(
            "{} identifies itself as {}, expected {target}",
            executable.to_string_lossy(),
            manifest.name
        )));
    }
    manifest.validate(&ir.ir_version).map_err(PluginError)?;
    let capability_warnings: Vec<String> = manifest
        .unsupported_features(ir)
        .into_iter()
        .map(|warning| warning.0)
        .collect();
    if strict && !capability_warnings.is_empty() {
        return Err(PluginError(capability_warnings.join("\n")));
    }

    let request = EmitRequest {
        protocol: PROTOCOL_VERSION,
        ir,
        options,
        target,
    };
    let input = serde_json::to_vec(&request)
        .map_err(|error| PluginError(format!("cannot encode emitter request: {error}")))?;
    let mut child = Command::new(executable)
        .arg("emit")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| {
            PluginError(format!(
                "cannot start {}: {error}",
                executable.to_string_lossy()
            ))
        })?;
    child
        .stdin
        .take()
        .ok_or_else(|| PluginError("emitter stdin is unavailable".into()))?
        .write_all(&input)
        .map_err(|error| PluginError(format!("cannot send request to emitter: {error}")))?;
    let output = child
        .wait_with_output()
        .map_err(|error| PluginError(format!("cannot wait for emitter: {error}")))?;
    ensure_success(executable, "emit", &output.status, &output.stderr)?;
    let response: EmitResponse = serde_json::from_slice(&output.stdout).map_err(|error| {
        PluginError(format!(
            "{} returned an invalid response: {error}",
            executable.to_string_lossy()
        ))
    })?;
    validate_response(&response).map_err(PluginError)?;
    let mut logs = manifest_output.stderr;
    logs.extend(output.stderr);
    Ok(PluginResult {
        manifest,
        response,
        capability_warnings,
        logs,
    })
}

fn ensure_success(
    executable: &OsStr,
    command: &str,
    status: &std::process::ExitStatus,
    stderr: &[u8],
) -> Result<(), PluginError> {
    if status.success() {
        return Ok(());
    }
    let detail = String::from_utf8_lossy(stderr);
    let detail = detail.trim();
    Err(PluginError(if detail.is_empty() {
        format!(
            "{} {command} exited with {status}",
            executable.to_string_lossy()
        )
    } else {
        format!(
            "{} {command} exited with {status}: {detail}",
            executable.to_string_lossy()
        )
    }))
}
