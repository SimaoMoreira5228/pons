use std::fs;
use std::process::Command;

use serde_json::Value;

const FIXTURE: &str = include_str!("../../../fixtures/login.json");

fn pons() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pons"))
}

#[test]
fn check_accepts_a_valid_contract() {
    let output = pons()
        .args(["check", "../../fixtures/login.json"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("valid:"));
}

#[test]
fn fmt_rewrites_valid_contracts_in_canonical_order() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("api.json");
    let mut input: Value = serde_json::from_str(FIXTURE).unwrap();
    input["operations"].as_array_mut().unwrap().reverse();
    fs::write(&path, serde_json::to_vec(&input).unwrap()).unwrap();

    let output = pons().arg("fmt").arg(&path).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let formatted = fs::read_to_string(path).unwrap();
    let parsed: Value = serde_json::from_str(&formatted).unwrap();
    assert_eq!(parsed["operations"][0]["id"][0][0], "auth");
    assert!(formatted.ends_with('\n'));
}

#[test]
fn fmt_leaves_invalid_input_untouched() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("api.json");
    let input = FIXTURE.replace("\"ir_version\": \"0.1\"", "\"ir_version\": \"9.0\"");
    fs::write(&path, &input).unwrap();

    let output = pons().arg("fmt").arg(&path).output().unwrap();
    assert!(!output.status.success());
    assert_eq!(fs::read_to_string(path).unwrap(), input);
}

#[test]
fn diff_returns_nonzero_for_removed_operations() {
    let directory = tempfile::tempdir().unwrap();
    let old = directory.path().join("old.json");
    let new = directory.path().join("new.json");
    fs::write(&old, FIXTURE).unwrap();
    let mut input: Value = serde_json::from_str(FIXTURE).unwrap();
    input["operations"] = serde_json::json!([]);
    fs::write(&new, serde_json::to_vec(&input).unwrap()).unwrap();

    let output = pons().arg("diff").arg(&old).arg(&new).output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .contains("BREAKING operations.auth.login: operation removed")
    );
}

#[test]
fn diff_accepts_identical_contracts() {
    let directory = tempfile::tempdir().unwrap();
    let old = directory.path().join("old.json");
    let new = directory.path().join("new.json");
    fs::write(&old, FIXTURE).unwrap();
    fs::write(&new, FIXTURE).unwrap();

    let output = pons().arg("diff").arg(&old).arg(&new).output().unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "No changes."
    );
}

#[test]
fn emit_writes_a_typed_typescript_client() {
    let directory = tempfile::tempdir().unwrap();
    let output_dir = directory.path().join("generated");
    let input_path = directory.path().join("api.json");
    let input: Value = serde_json::from_str(FIXTURE).unwrap();
    fs::write(&input_path, serde_json::to_vec(&input).unwrap()).unwrap();
    let output = pons()
        .arg("emit")
        .arg("--target")
        .arg("ts")
        .arg(input_path)
        .arg(&output_dir)
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let types = fs::read_to_string(output_dir.join("types.ts")).unwrap();
    let client = fs::read_to_string(output_dir.join("client.ts")).unwrap();
    assert!(types.contains("export type User"));
    assert!(client.contains("export function createClient"));
    assert!(client.contains("neverthrow"));
    assert!(client.contains("EventsWatchEvent"));
}

#[cfg(unix)]
#[test]
fn emit_runs_an_external_target_and_writes_its_files() {
    use std::os::unix::fs::PermissionsExt;

    let directory = tempfile::tempdir().unwrap();
    let bin = directory.path().join("bin");
    fs::create_dir(&bin).unwrap();
    let plugin = bin.join("pons-emit-fixture");
    fs::write(
        &plugin,
        r##"#!/bin/sh
if [ "$1" = describe ]; then
  printf '%s' '{"name":"fixture","version":"1.0.0","protocol":{"min":"1.0","max":"1.0"},"ir_version":{"min":"0.1","max":"0.1"},"capabilities":{"streaming":["sse"],"enum_reprs":["string","external","internal","adjacent","untagged"],"direction_split":true,"provides":true,"flattened":true}}'
elif [ "$1" = emit ]; then
  request=$(cat)
  printf '%s\n' 'plugin log' >&2
  case "$request" in
    *'"target":"fixture"'*) printf '%s' '{"protocol":1,"files":[{"path":"nested/client.ts","content":"generated"}]}' ;;
    *) exit 3 ;;
  esac
else
  exit 2
fi
"##,
    )
    .unwrap();
    let mut permissions = fs::metadata(&plugin).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&plugin, permissions).unwrap();
    let path = std::env::join_paths(std::iter::once(bin).chain(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    )))
    .unwrap();
    let input = directory.path().join("api.json");
    let output_dir = directory.path().join("generated");
    fs::write(&input, FIXTURE).unwrap();

    let output = pons()
        .args(["emit", "--target", "fixture"])
        .arg(&input)
        .arg(&output_dir)
        .arg("--strict")
        .env("PATH", path)
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read_to_string(output_dir.join("nested/client.ts")).unwrap(),
        "generated"
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("plugin log"));
}
