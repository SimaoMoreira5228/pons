mod executable;
mod run;
mod termination;

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::PathBuf;

use pons_ir::Document;
use serde_json::Value;

use crate::pin::PluginPin;
use crate::protocol::{
    EmitMode, EmitRequest, EmitResponse, EmitterManifest, validate_response_for_document,
    validation_enabled,
};
use executable::{resolve_executable, resolve_pinned};
pub use run::log_tail;
use run::{ensure_success, run_command, with_emitter_log};

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
    search: &[PathBuf],
    pin: Option<&PluginPin>,
) -> Result<PluginResult, PluginError> {
    let mut canonical = ir.clone();
    canonical.canonicalize();
    canonical.validate().map_err(|errors| {
        PluginError(
            errors
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("\n"),
        )
    })?;
    let ir = &canonical;

    let resolved = match pin {
        Some(pin) => resolve_pinned(pin)?,
        None => resolve_executable(executable, search)?,
    };
    let manifest_output = run_command(&resolved.program, &resolved.command("describe"), None)?;
    ensure_success(
        executable,
        "describe",
        &manifest_output.status,
        &manifest_output.stderr,
    )?;
    let manifest: EmitterManifest =
        serde_json::from_slice(&manifest_output.stdout).map_err(|error| {
            with_emitter_log(
                &format!(
                    "{} returned an invalid manifest: {error}",
                    executable.to_string_lossy()
                ),
                &manifest_output.stderr,
            )
        })?;
    if manifest.name != target {
        return Err(PluginError(format!(
            "{} identifies itself as {}, expected {target}",
            executable.to_string_lossy(),
            manifest.name
        )));
    }
    manifest.validate(&ir.ir_version).map_err(PluginError)?;
    manifest.validate_options(&options).map_err(PluginError)?;
    let validation_requested = validation_enabled(&options);
    let protocol = manifest
        .protocol_version(validation_requested)
        .map_err(PluginError)?;
    let capability_warnings: Vec<String> = manifest
        .unsupported_features(ir)
        .into_iter()
        .chain(manifest.validation_gap(&options))
        .map(|warning| warning.0)
        .collect();
    if strict && !capability_warnings.is_empty() {
        return Err(with_emitter_log(
            &capability_warnings.join("\n"),
            &manifest_output.stderr,
        ));
    }

    let mode = if strict {
        EmitMode::Strict
    } else {
        EmitMode::Lossy
    };
    let request = EmitRequest {
        protocol,
        source_ir_version: ir.ir_version.clone(),
        mode,
        ir,
        options,
        target,
    };
    let input = serde_json::to_vec(&request)
        .map_err(|error| PluginError(format!("cannot encode emitter request: {error}")))?;
    let output = run_command(&resolved.program, &resolved.command("emit"), Some(input))?;
    ensure_success(executable, "emit", &output.status, &output.stderr)?;
    let mut logs = manifest_output.stderr;
    logs.extend(&output.stderr);
    let response: EmitResponse = serde_json::from_slice(&output.stdout).map_err(|error| {
        with_emitter_log(
            &format!(
                "{} returned an invalid response: {error}",
                executable.to_string_lossy()
            ),
            &logs,
        )
    })?;
    validate_response_for_document(
        &response,
        ir,
        mode,
        protocol,
        manifest.capabilities.opaque,
        validation_requested,
    )
    .map_err(|error| with_emitter_log(&error, &logs))?;
    Ok(PluginResult {
        manifest,
        response,
        capability_warnings,
        logs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_os = "linux")]
    use std::io::Write;
    #[cfg(target_os = "linux")]
    use std::process::{Command, Stdio};

    #[test]
    fn rejects_invalid_ir_before_starting_plugin() {
        let ir: Document = serde_json::from_str(
            r#"{"ir_version":"9.0","info":{"title":"Example","version":"1"}}"#,
        )
        .unwrap();
        let error = invoke(
            OsStr::new("plugin-does-not-exist"),
            "fixture",
            &ir,
            BTreeMap::new(),
            true,
            &[],
            None,
        )
        .unwrap_err();

        assert!(error.0.contains("unsupported IR version 9.0"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn an_unparsable_answer_keeps_the_emitters_explanation() {
        let document: Document =
            serde_json::from_str(include_str!("../../../../../spec/fixtures/login.json")).unwrap();

        let unparsable_manifest = stub_emitter(
            "unparsable-manifest",
            r#"#!/bin/sh
printf '%s\n' 'the range key is protokl' >&2
printf '%s' '{"name":"fixture","protokl":{"min":"3.0","max":"3.0"}}'
"#,
        );
        let error = invoke(
            unparsable_manifest.as_os_str(),
            "fixture",
            &document,
            BTreeMap::new(),
            false,
            &[],
            None,
        )
        .unwrap_err();
        assert!(error.0.contains("invalid manifest"), "{}", error.0);
        assert!(error.0.contains("unknown field `protokl`"), "{}", error.0);
        assert!(error.0.contains("the range key is protokl"), "{}", error.0);
        let _ = std::fs::remove_file(&unparsable_manifest);

        let unparsable_response = stub_emitter(
            "unparsable-response",
            r#"#!/bin/sh
if [ "$1" = describe ]; then
  printf '%s' '{"name":"fixture","version":"1.0.0","protocol":{"min":"3.0","max":"3.0"},"ir_version":{"min":"0.2","max":"0.2"},"capabilities":{"streaming":["sse"],"enum_reprs":["string","external","internal","adjacent","untagged"],"direction_split":true,"provides":true,"flattened":true,"opaque":true,"object_openness":["reject","preserve_typed","preserve_opaque"],"extensions":{"test.example.source/v1":"exact","test.example.large/v1":"exact"}}}'
  exit 0
fi
cat > /dev/null
printf '%s\n' 'the coverage key is coverag' >&2
printf '%s' '{"protocol":3,"coverag":{}}'
"#,
        );
        let error = invoke(
            unparsable_response.as_os_str(),
            "fixture",
            &document,
            BTreeMap::new(),
            false,
            &[],
            None,
        )
        .unwrap_err();
        assert!(error.0.contains("invalid response"), "{}", error.0);
        assert!(error.0.contains("unknown field `coverag`"), "{}", error.0);
        assert!(
            error.0.contains("the coverage key is coverag"),
            "{}",
            error.0
        );
        let _ = std::fs::remove_file(&unparsable_response);
    }

    #[cfg(target_os = "linux")]
    fn stub_emitter(name: &str, script: &str) -> std::path::PathBuf {
        use std::process;

        let path = std::env::temp_dir().join(format!("pons-plugin-{name}-{}", process::id()));

        let mut writer = Command::new("sh")
            .args(["-c", "cat > \"$1\" && chmod 755 \"$1\"", "sh"])
            .arg(&path)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        writer
            .stdin
            .take()
            .unwrap()
            .write_all(script.as_bytes())
            .unwrap();
        let status = writer.wait().unwrap();
        assert!(status.success(), "cannot write the {name} stub: {status}");
        path
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_refusing_emitter_gets_its_own_log_reported() {
        let document: Document =
            serde_json::from_str(include_str!("../../../../../spec/fixtures/login.json")).unwrap();

        let refusing = stub_emitter(
            "refusing",
            r#"#!/bin/sh
printf '%s\n' 'cannot model preserved objects' >&2
printf '%s' '{"name":"fixture","version":"1.0.0","protocol":{"min":"3.0","max":"3.0"},"ir_version":{"min":"0.2","max":"0.2"},"capabilities":{"streaming":["sse"],"enum_reprs":["string","external","internal","adjacent","untagged"],"direction_split":true,"provides":true,"flattened":true,"object_openness":["reject","preserve_typed","preserve_opaque"],"extensions":{"test.example.source/v1":"exact","test.example.large/v1":"exact"}}}'
"#,
        );
        let error = invoke(
            refusing.as_os_str(),
            "fixture",
            &document,
            BTreeMap::new(),
            true,
            &[],
            None,
        )
        .unwrap_err();
        assert!(error.0.contains("does not declare opaque value support"));
        assert!(
            error.0.contains("cannot model preserved objects"),
            "{}",
            error.0
        );
        let _ = std::fs::remove_file(&refusing);

        let unclassified = stub_emitter(
            "unclassified",
            r#"#!/bin/sh
if [ "$1" = describe ]; then
  printf '%s' '{"name":"fixture","version":"1.0.0","protocol":{"min":"3.0","max":"3.0"},"ir_version":{"min":"0.2","max":"0.2"},"capabilities":{"streaming":["sse"],"enum_reprs":["string","external","internal","adjacent","untagged"],"direction_split":true,"provides":true,"flattened":true,"opaque":true,"object_openness":["reject","preserve_typed","preserve_opaque"],"extensions":{"test.example.source/v1":"exact","test.example.large/v1":"exact"}}}'
  exit 0
fi
cat > /dev/null
printf '%s\n' 'gave up on types.User' >&2
printf '%s' '{"protocol":3,"files":[{"path":"client.ts","content":"generated"}],"coverage":{"exact":["types.EventQuery","types.LoginError"],"opaque":["operations.auth.login","operations.events.watch","types.LoginRequest"],"lossy":[],"unsupported":[]}}'
"#,
        );
        let error = invoke(
            unclassified.as_os_str(),
            "fixture",
            &document,
            BTreeMap::new(),
            true,
            &[],
            None,
        )
        .unwrap_err();
        assert!(error.0.contains("missing subjects"), "{}", error.0);
        assert!(error.0.contains("gave up on types.User"), "{}", error.0);
        let _ = std::fs::remove_file(&unclassified);
    }
}
