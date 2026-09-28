use std::fs;
use std::path::Path;
use std::process::Command;

use serde_json::Value;

const FIXTURE: &str = include_str!("../../../fixtures/login.json");

fn pons() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pons"))
}

fn wide_number_input(directory: &Path) -> std::path::PathBuf {
    let path = directory.join("api.json");
    let mut contract: Value = serde_json::from_str(FIXTURE).unwrap();
    contract["types"]["User"]["fields"][0]["ty"]["wire"] = serde_json::json!("number");
    fs::write(&path, serde_json::to_vec(&contract).unwrap()).unwrap();
    path
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
    let input = FIXTURE.replace("\"ir_version\": \"0.2\"", "\"ir_version\": \"9.0\"");
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
fn diff_returns_nonzero_for_idempotency_changes() {
    let directory = tempfile::tempdir().unwrap();
    let old = directory.path().join("old.json");
    let new = directory.path().join("new.json");
    fs::write(&old, FIXTURE).unwrap();
    let mut input: Value = serde_json::from_str(FIXTURE).unwrap();
    input["operations"][0]["semantics"]["idempotent"] = serde_json::json!(true);
    fs::write(&new, serde_json::to_vec(&input).unwrap()).unwrap();

    let output = pons().arg("diff").arg(&old).arg(&new).output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .contains("BREAKING operations.auth.login.semantics: operation idempotency changed")
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

#[test]
fn emit_selects_the_throwing_typescript_strategy() {
    let directory = tempfile::tempdir().unwrap();
    let output_dir = directory.path().join("generated");
    let input_path = directory.path().join("api.json");
    fs::write(&input_path, FIXTURE).unwrap();
    let output = pons()
        .args(["emit", "--target", "ts"])
        .arg(&input_path)
        .arg(&output_dir)
        .args(["--errors", "throw"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let client = fs::read_to_string(output_dir.join("client.ts")).unwrap();
    assert!(client.contains("requestOrThrow(request<"));
    assert!(!client.contains("from \"neverthrow\""));
}

#[test]
fn emit_strict_uses_bigint_for_wide_typescript_numbers() {
    let directory = tempfile::tempdir().unwrap();
    let input = wide_number_input(directory.path());
    let output_dir = directory.path().join("generated");
    let output = pons()
        .args(["emit", "--target", "ts"])
        .arg(&input)
        .arg(&output_dir)
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let types = fs::read_to_string(output_dir.join("types.ts")).unwrap();
    assert!(types.contains("\"id\": bigint"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("wide numeric integers"));
}

#[test]
fn emit_lossy_writes_wide_typescript_numbers_with_a_warning() {
    let directory = tempfile::tempdir().unwrap();
    let input = wide_number_input(directory.path());
    let output_dir = directory.path().join("generated");
    let output = pons()
        .args(["emit", "--target", "ts"])
        .arg(&input)
        .arg(&output_dir)
        .arg("--lossy")
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("wide numeric integers"));
    assert!(output_dir.join("types.ts").exists());
    assert!(output_dir.join("client.ts").exists());
}

#[cfg(target_os = "linux")]
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
  printf '%s' '{"name":"fixture","version":"1.0.0","protocol":{"min":"1.0","max":"1.0"},"ir_version":{"min":"0.2","max":"0.2"},"capabilities":{"streaming":["sse"],"enum_reprs":["string","external","internal","adjacent","untagged"],"direction_split":true,"provides":true,"flattened":true,"object_openness":["reject","preserve_typed","preserve_opaque"]}}'
elif [ "$1" = emit ]; then
  i=0
  while [ "$i" -lt 2000 ]; do
    printf '%*s' 1000 ''
    i=$((i + 1))
  done
  request=$(cat)
  printf '%s' "$request" > "$PONS_REQUEST"
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
    let request_path = directory.path().join("request.json");
    let mut contract: Value = serde_json::from_str(FIXTURE).unwrap();
    contract["ext"]["large"] = Value::String("x".repeat(100_000));
    contract["operations"][1]["auth"]["alternatives"] =
        serde_json::json!([["tenant"], ["session"]]);
    contract["operations"].as_array_mut().unwrap().reverse();
    contract["types"]["User"]["fields"]
        .as_array_mut()
        .unwrap()
        .reverse();
    fs::write(&input, serde_json::to_vec(&contract).unwrap()).unwrap();

    let output = Command::new("timeout")
        .args(["10s", env!("CARGO_BIN_EXE_pons")])
        .args(["emit", "--target", "fixture"])
        .arg(&input)
        .arg(&output_dir)
        .env("PATH", &path)
        .env("PONS_REQUEST", &request_path)
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

    let request: Value = serde_json::from_slice(&fs::read(request_path).unwrap()).unwrap();
    let actual: pons_ir::Document = serde_json::from_value(request["ir"].clone()).unwrap();
    let mut expected: pons_ir::Document = serde_json::from_value(contract).unwrap();
    expected.canonicalize();
    assert_eq!(actual, expected);

    let wide_input = wide_number_input(directory.path());
    let strict_output_dir = directory.path().join("strict-generated");
    let strict_request = directory.path().join("strict-request.json");
    let strict_output = Command::new("timeout")
        .args(["10s", env!("CARGO_BIN_EXE_pons")])
        .args(["emit", "--target", "fixture"])
        .arg(&wide_input)
        .arg(&strict_output_dir)
        .env("PATH", &path)
        .env("PONS_REQUEST", &strict_request)
        .output()
        .unwrap();
    assert!(!strict_output.status.success());
    assert!(
        String::from_utf8_lossy(&strict_output.stderr).contains("lossless wide integer support")
    );
    assert!(!strict_request.exists());
    assert!(!strict_output_dir.exists());

    let lossy_output_dir = directory.path().join("lossy-generated");
    let lossy_request = directory.path().join("lossy-request.json");
    let lossy_output = Command::new("timeout")
        .args(["10s", env!("CARGO_BIN_EXE_pons")])
        .args(["emit", "--target", "fixture"])
        .arg(&wide_input)
        .arg(&lossy_output_dir)
        .arg("--lossy")
        .env("PATH", &path)
        .env("PONS_REQUEST", &lossy_request)
        .output()
        .unwrap();
    assert!(
        lossy_output.status.success(),
        "{}",
        String::from_utf8_lossy(&lossy_output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&lossy_output.stderr).contains("lossless wide integer support")
    );
    assert!(lossy_request.exists());
    assert!(lossy_output_dir.join("nested/client.ts").exists());
}

#[test]
fn openapi_emit_failure_does_not_create_output() {
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("api.json");
    let output_dir = directory.path().join("generated");
    let mut contract: Value = serde_json::from_str(FIXTURE).unwrap();
    contract["operations"][0]["path"] = serde_json::json!([
        {"lit": "events"},
        {"param": "id", "ty": {"kind": "string"}}
    ]);
    contract["operations"][1]["path"] = serde_json::json!([
        {"lit": "events"},
        {"param": "name", "ty": {"kind": "string"}}
    ]);
    fs::write(&input, serde_json::to_vec(&contract).unwrap()).unwrap();

    let output = pons()
        .args(["emit", "--target", "openapi"])
        .arg(&input)
        .arg(&output_dir)
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("OpenAPI route pattern /events/{} uses conflicting parameter names")
    );
    assert!(!output_dir.exists());
}

#[test]
fn emit_writes_an_openapi_31_document() {
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("api.json");
    let output_dir = directory.path().join("generated");
    fs::write(&input, FIXTURE).unwrap();

    let output = pons()
        .args(["emit", "--target", "openapi"])
        .arg(&input)
        .arg(&output_dir)
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let document: Value =
        serde_json::from_slice(&fs::read(output_dir.join("openapi.json")).unwrap()).unwrap();
    assert_eq!(document["openapi"], "3.1.1");
    assert!(document["paths"]["/auth/login"]["post"].is_object());
}
