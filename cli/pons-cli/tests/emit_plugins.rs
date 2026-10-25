mod support;

use std::fs;
use std::process::Command;

use serde_json::Value;

#[cfg(target_os = "linux")]
use support::on_path;
use support::{FIXTURE, PRESERVED_SUBJECTS, custom_auth_input, fixture, pons, wide_number_input};

#[cfg(target_os = "linux")]
#[test]
fn emit_strict_rejects_a_target_that_cannot_carry_preserved_objects() {
    use std::os::unix::fs::PermissionsExt;

    let directory = tempfile::tempdir().unwrap();
    let bin = directory.path().join("bin");
    fs::create_dir(&bin).unwrap();
    let plugin = bin.join("pons-emit-fixture");
    fs::write(
        &plugin,
        r##"#!/bin/sh
if [ "$1" = describe ]; then
  printf '%s' '{"name":"fixture","version":"1.0.0","protocol":{"min":"3.0","max":"3.0"},"ir_version":{"min":"0.2","max":"0.2"},"capabilities":{"streaming":["sse"],"enum_reprs":["string","external","internal","adjacent","untagged"],"direction_split":true,"provides":true,"flattened":true,"object_openness":["reject","preserve_typed","preserve_opaque"],"extensions":{"test.example.source/v1":"exact","test.example.large/v1":"exact"}}}'
elif [ "$1" = emit ]; then
  request=$(cat)
  printf '%s' "$request" > "$PONS_REQUEST"
  printf '%s' '{"protocol":3,"files":[{"path":"nested/client.ts","content":"generated"}],"coverage":{"exact":["types.EventQuery","types.LoginError"],"opaque":["operations.auth.login","operations.events.watch","types.LoginRequest","types.User"],"lossy":[],"unsupported":[]}}'
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
    fs::write(&input, FIXTURE).unwrap();
    let request_path = directory.path().join("request.json");
    let output_dir = directory.path().join("strict-generated");
    let output = pons()
        .args(["emit", "--target", "fixture"])
        .arg(&input)
        .arg(&output_dir)
        .env("PATH", &path)
        .env("PONS_REQUEST", &request_path)
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("does not declare opaque value support"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!request_path.exists(), "emit must not run in strict mode");
    assert!(!output_dir.exists());

    let lossy_dir = directory.path().join("lossy-generated");
    let lossy_request = directory.path().join("lossy-request.json");
    let output = pons()
        .args(["emit", "--target", "fixture"])
        .arg(&input)
        .arg(&lossy_dir)
        .arg("--lossy")
        .env("PATH", &path)
        .env("PONS_REQUEST", &lossy_request)
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("does not declare opaque value support")
    );
    assert!(lossy_request.exists());
    assert!(lossy_dir.join("nested/client.ts").exists());
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
  printf '%s' '{"name":"fixture","version":"1.0.0","protocol":{"min":"3.0","max":"3.0"},"ir_version":{"min":"0.2","max":"0.2"},"capabilities":{"streaming":["sse"],"enum_reprs":["string","external","internal","adjacent","untagged"],"direction_split":true,"provides":true,"flattened":true,"opaque":true,"object_openness":["reject","preserve_typed","preserve_opaque"],"extensions":{"test.example.source/v1":"exact","test.example.large/v1":"exact"}}}'
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
    *'"bits":64,"signed":false,"wire":"number"'*'"target":"fixture"'*) printf '%s' '{"protocol":3,"files":[{"path":"nested/client.ts","content":"generated"}],"coverage":{"exact":["types.EventQuery","types.LoginError"],"opaque":[],"lossy":["operations.auth.login","operations.events.watch","types.LoginRequest","types.User"],"unsupported":[]}}' ;;
    *'"target":"fixture"'*) printf '%s' '{"protocol":3,"files":[{"path":"nested/client.ts","content":"generated"}],"coverage":{"exact":["types.EventQuery","types.LoginError"],"opaque":["operations.auth.login","operations.events.watch","types.LoginRequest","types.User"],"lossy":[],"unsupported":[]}}' ;;
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
    contract["ext"]["test.example.large/v1"] = Value::String("x".repeat(100_000));
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

    let custom_input = custom_auth_input(directory.path());
    let strict_output_dir = directory.path().join("custom-strict-generated");
    let strict_request = directory.path().join("custom-strict-request.json");
    let strict_output = Command::new("timeout")
        .args(["10s", env!("CARGO_BIN_EXE_pons")])
        .args(["emit", "--target", "fixture"])
        .arg(&custom_input)
        .arg(&strict_output_dir)
        .env("PATH", &path)
        .env("PONS_REQUEST", &strict_request)
        .output()
        .unwrap();
    assert!(!strict_output.status.success());
    assert!(
        String::from_utf8_lossy(&strict_output.stderr)
            .contains("custom authentication scheme support")
    );
    assert!(!strict_request.exists());
    assert!(!strict_output_dir.exists());

    let lossy_output_dir = directory.path().join("custom-lossy-generated");
    let lossy_request = directory.path().join("custom-lossy-request.json");
    let lossy_output = Command::new("timeout")
        .args(["10s", env!("CARGO_BIN_EXE_pons")])
        .args(["emit", "--target", "fixture"])
        .arg(&custom_input)
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
        String::from_utf8_lossy(&lossy_output.stderr)
            .contains("custom authentication scheme support")
    );
    assert!(lossy_request.exists());
    assert!(lossy_output_dir.join("nested/client.ts").exists());
}

#[cfg(target_os = "linux")]
#[test]
fn emit_lossy_warns_about_coverage_an_external_target_reports_below_exact() {
    use std::os::unix::fs::PermissionsExt;

    let directory = tempfile::tempdir().unwrap();
    let bin = directory.path().join("bin");
    fs::create_dir(&bin).unwrap();
    let plugin = bin.join("pons-emit-python");
    fs::write(
        &plugin,
        r##"#!/bin/sh
if [ "$1" = describe ]; then
  printf '%s' '{"name":"python","version":"1.0.0","protocol":{"min":"3.0","max":"3.0"},"ir_version":{"min":"0.2","max":"0.2"},"capabilities":{"streaming":["sse"],"enum_reprs":["string","external","internal","adjacent","untagged"],"direction_split":true,"provides":true,"flattened":true,"opaque":true,"object_openness":["reject","preserve_typed","preserve_opaque"],"extensions":{"test.example.source/v1":"exact","test.example.large/v1":"exact"}}}'
elif [ "$1" = emit ]; then
  cat > /dev/null
  printf '%s' '{"protocol":3,"files":[{"path":"client.py","content":"generated"}],"coverage":{"exact":["types.EventQuery","types.LoginError"],"opaque":["operations.auth.login","operations.events.watch","types.LoginRequest"],"lossy":["types.User"],"unsupported":[]}}'
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
    fs::write(&input, FIXTURE).unwrap();
    let output_dir = directory.path().join("lossy-generated");
    let output = pons()
        .args(["emit", "--target", "python"])
        .arg(&input)
        .arg(&output_dir)
        .arg("--lossy")
        .env("PATH", &path)
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "warning: python emits opaque coverage at operations.auth.login, operations.events.watch, types.LoginRequest\nwarning: python emits lossy coverage at types.User\n"
    );
    assert_eq!(
        fs::read_to_string(output_dir.join("client.py")).unwrap(),
        "generated"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn emit_writes_both_coverage_blocks_for_a_validating_plugin() {
    use std::os::unix::fs::PermissionsExt;

    let directory = tempfile::tempdir().unwrap();
    let bin = directory.path().join("bin");
    fs::create_dir(&bin).unwrap();
    let plugin = bin.join("pons-emit-python");
    fs::write(
        &plugin,
        r##"#!/bin/sh
if [ "$1" = describe ]; then
  printf '%s' '{"name":"python","version":"1.0.0","protocol":{"min":"3.0","max":"3.0"},"ir_version":{"min":"0.2","max":"0.2"},"capabilities":{"streaming":["sse"],"enum_reprs":["string","external","internal","adjacent","untagged"],"direction_split":true,"provides":true,"flattened":true,"opaque":true,"object_openness":["reject","preserve_typed","preserve_opaque"],"extensions":{"test.example.source/v1":"exact","test.example.large/v1":"exact"},"validation":{"backends":["pydantic"],"capabilities":{"validation.scalars":"exact"}}},"option_schema":{"options":{"validation":{"type":"object"}}}}'
elif [ "$1" = emit ]; then
  cat > /dev/null
  printf '%s' '{"protocol":3,"files":[{"path":"client.py","content":"generated"}],"coverage":{"exact":["types.EventQuery","types.LoginError"],"opaque":["operations.auth.login","operations.events.watch","types.LoginRequest","types.User"],"lossy":[],"unsupported":[]},"validation_coverage":{"exact":["operations.auth.login","operations.events.watch","types.EventQuery","types.LoginError","types.LoginRequest","types.User"],"lossy":[],"unsupported":[]}}'
else
  exit 2
fi
"##,
    )
    .unwrap();
    let mut permissions = fs::metadata(&plugin).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&plugin, permissions).unwrap();

    fs::write(directory.path().join("api.json"), FIXTURE).unwrap();
    fs::write(
        directory.path().join("pons.toml"),
        "[emit.python]\nout_dir = \"generated\"\nvalidation = { backend = \"pydantic\" }\n",
    )
    .unwrap();
    let output = pons()
        .args([
            "emit",
            "--target",
            "python",
            "api.json",
            "--coverage",
            "coverage.json",
        ])
        .current_dir(directory.path())
        .env("PATH", on_path(&bin))
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_str(
        &fs::read_to_string(directory.path().join("generated/coverage.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        report["coverage"]["opaque"],
        serde_json::json!(PRESERVED_SUBJECTS)
    );
    assert_eq!(
        report["validation_coverage"],
        serde_json::json!({
            "exact": [
                "operations.auth.login",
                "operations.events.watch",
                "types.EventQuery",
                "types.LoginError",
                "types.LoginRequest",
                "types.User"
            ],
            "lossy": [],
            "unsupported": []
        })
    );
}

#[cfg(target_os = "linux")]
#[test]
fn emit_refuses_a_validating_plugin_that_omits_validation_coverage() {
    use std::os::unix::fs::PermissionsExt;

    let directory = tempfile::tempdir().unwrap();
    let bin = directory.path().join("bin");
    fs::create_dir(&bin).unwrap();
    let plugin = bin.join("pons-emit-python");
    fs::write(
        &plugin,
        r##"#!/bin/sh
if [ "$1" = describe ]; then
  printf '%s' '{"name":"python","version":"1.0.0","protocol":{"min":"3.0","max":"3.0"},"ir_version":{"min":"0.2","max":"0.2"},"capabilities":{"streaming":["sse"],"enum_reprs":["string","external","internal","adjacent","untagged"],"direction_split":true,"provides":true,"flattened":true,"opaque":true,"object_openness":["reject","preserve_typed","preserve_opaque"],"extensions":{"test.example.source/v1":"exact","test.example.large/v1":"exact"},"validation":{"backends":["pydantic"],"capabilities":{"validation.scalars":"exact"}}},"option_schema":{"options":{"validation":{"type":"object"}}}}'
elif [ "$1" = emit ]; then
  cat > /dev/null
  printf '%s' '{"protocol":3,"files":[{"path":"client.py","content":"generated"}],"coverage":{"exact":["types.EventQuery","types.LoginError"],"opaque":["operations.auth.login","operations.events.watch","types.LoginRequest","types.User"],"lossy":[],"unsupported":[]}}'
else
  exit 2
fi
"##,
    )
    .unwrap();
    let mut permissions = fs::metadata(&plugin).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&plugin, permissions).unwrap();

    fs::write(directory.path().join("api.json"), FIXTURE).unwrap();
    fs::write(
        directory.path().join("pons.toml"),
        "[emit.python]\nout_dir = \"generated\"\nvalidation = { backend = \"pydantic\" }\n",
    )
    .unwrap();
    let output = pons()
        .args([
            "emit",
            "--target",
            "python",
            "api.json",
            "--coverage",
            "coverage.json",
        ])
        .current_dir(directory.path())
        .env("PATH", on_path(&bin))
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("missing validation coverage while validation is enabled"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!directory.path().join("generated").exists());
}

#[cfg(target_os = "linux")]
#[test]
fn emit_writes_an_empty_validation_block_when_the_plugin_disables_validation() {
    use std::os::unix::fs::PermissionsExt;

    let directory = tempfile::tempdir().unwrap();
    let bin = directory.path().join("bin");
    fs::create_dir(&bin).unwrap();
    let plugin = bin.join("pons-emit-python");
    fs::write(
        &plugin,
        r##"#!/bin/sh
if [ "$1" = describe ]; then
  printf '%s' '{"name":"python","version":"1.0.0","protocol":{"min":"3.0","max":"3.0"},"ir_version":{"min":"0.2","max":"0.2"},"capabilities":{"streaming":["sse"],"enum_reprs":["string","external","internal","adjacent","untagged"],"direction_split":true,"provides":true,"flattened":true,"opaque":true,"object_openness":["reject","preserve_typed","preserve_opaque"],"extensions":{"test.example.source/v1":"exact","test.example.large/v1":"exact"}}}'
elif [ "$1" = emit ]; then
  cat > /dev/null
  printf '%s' '{"protocol":3,"files":[{"path":"client.py","content":"generated"}],"coverage":{"exact":["types.EventQuery","types.LoginError"],"opaque":["operations.auth.login","operations.events.watch","types.LoginRequest","types.User"],"lossy":[],"unsupported":[]}}'
else
  exit 2
fi
"##,
    )
    .unwrap();
    let mut permissions = fs::metadata(&plugin).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&plugin, permissions).unwrap();

    fs::write(directory.path().join("api.json"), FIXTURE).unwrap();
    fs::write(
        directory.path().join("pons.toml"),
        "[emit.python]\nout_dir = \"generated\"\n",
    )
    .unwrap();
    let output = pons()
        .args([
            "emit",
            "--target",
            "python",
            "api.json",
            "--coverage",
            "coverage.json",
        ])
        .current_dir(directory.path())
        .env("PATH", on_path(&bin))
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_str(
        &fs::read_to_string(directory.path().join("generated/coverage.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        report["validation_coverage"],
        serde_json::json!({ "exact": [], "lossy": [], "unsupported": [] })
    );
}

#[cfg(target_os = "linux")]
#[test]
fn emit_lossy_warns_about_a_validation_gap() {
    use std::os::unix::fs::PermissionsExt;

    let directory = tempfile::tempdir().unwrap();
    let bin = directory.path().join("bin");
    fs::create_dir(&bin).unwrap();
    let plugin = bin.join("pons-emit-python");
    fs::write(
        &plugin,
        r##"#!/bin/sh
if [ "$1" = describe ]; then
  printf '%s' '{"name":"python","version":"1.0.0","protocol":{"min":"3.0","max":"3.0"},"ir_version":{"min":"0.2","max":"0.2"},"capabilities":{"streaming":["sse"],"enum_reprs":["string","external","internal","adjacent","untagged"],"direction_split":true,"provides":true,"flattened":true,"opaque":true,"object_openness":["reject","preserve_typed","preserve_opaque"],"extensions":{"test.example.source/v1":"exact","test.example.large/v1":"exact"}},"option_schema":{"options":{"validation":{"type":"object"}}}}'
elif [ "$1" = emit ]; then
  cat > /dev/null
  printf '%s' '{"protocol":3,"files":[{"path":"client.py","content":"generated"}],"coverage":{"exact":["types.EventQuery","types.LoginError"],"opaque":["operations.auth.login","operations.events.watch","types.LoginRequest","types.User"],"lossy":[],"unsupported":[]},"validation_coverage":{"exact":["operations.auth.login","operations.events.watch","types.EventQuery","types.LoginError","types.LoginRequest","types.User"],"lossy":[],"unsupported":[]}}'
else
  exit 2
fi
"##,
    )
    .unwrap();
    let mut permissions = fs::metadata(&plugin).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&plugin, permissions).unwrap();

    fs::write(directory.path().join("api.json"), FIXTURE).unwrap();
    fs::write(
        directory.path().join("pons.toml"),
        "[emit.python]\nout_dir = \"generated\"\nvalidation = { backend = \"pydantic\" }\n",
    )
    .unwrap();
    let output = pons()
        .args(["emit", "--target", "python", "api.json", "--lossy"])
        .current_dir(directory.path())
        .env("PATH", on_path(&bin))
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("does not declare validation support for the pydantic backend"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(target_os = "linux")]
#[test]
fn a_fixture_a_target_cannot_build_is_reported_unsupported() {
    use std::os::unix::fs::PermissionsExt;

    let directory = tempfile::tempdir().unwrap();
    let bin = directory.path().join("bin");
    fs::create_dir(&bin).unwrap();
    let plugin = bin.join("pons-emit-python");
    fs::write(
        &plugin,
        r##"#!/bin/sh
if [ "$1" = describe ]; then
  printf '%s' '{"name":"python","version":"1.0.0","protocol":{"min":"3.0","max":"3.0"},"ir_version":{"min":"0.2","max":"0.2"},"capabilities":{"streaming":["sse"],"enum_reprs":["string","external","internal","adjacent","untagged"],"direction_split":true,"provides":true,"flattened":true,"opaque":true,"object_openness":["reject","preserve_typed","preserve_opaque"],"extensions":{"test.example.source/v1":"exact","test.example.large/v1":"exact"}}}'
elif [ "$1" = emit ]; then
  request=$(cat)
  printf '%s' "$request" > "$PONS_REQUEST"
  printf '%s' '{"protocol":3,"files":[{"path":"client.py","content":"generated"}],"coverage":{"exact":["types.EventQuery","types.LoginError"],"opaque":["operations.auth.login","types.LoginRequest","types.User"],"lossy":[],"unsupported":["operations.events.watch"]}}'
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
    fs::write(&input, fixture("unsupported.json")).unwrap();
    let request_path = directory.path().join("request.json");
    let output_dir = directory.path().join("strict-generated");
    let output = pons()
        .args(["emit", "--target", "python"])
        .arg(&input)
        .arg(&output_dir)
        .env("PATH", &path)
        .env("PONS_REQUEST", &request_path)
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stderr).trim(),
        "pons: python does not declare support for websocket streams in events.watch"
    );
    assert!(!request_path.exists(), "emit must not run in strict mode");
    assert!(!output_dir.exists());

    let lossy_dir = directory.path().join("lossy-generated");
    let lossy_request = directory.path().join("lossy-request.json");
    let output = pons()
        .args(["emit", "--target", "python"])
        .arg(&input)
        .arg(&lossy_dir)
        .arg("--lossy")
        .env("PATH", &path)
        .env("PONS_REQUEST", &lossy_request)
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "warning: python does not declare support for websocket streams in events.watch\n\
         warning: python emits opaque coverage at operations.auth.login, types.LoginRequest, types.User\n\
         warning: python emits unsupported coverage at operations.events.watch\n"
    );
    assert!(lossy_request.exists());
    assert_eq!(
        fs::read_to_string(lossy_dir.join("client.py")).unwrap(),
        "generated"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn emit_bounds_the_log_it_echoes_from_an_external_target() {
    use std::os::unix::fs::PermissionsExt;

    let directory = tempfile::tempdir().unwrap();
    let bin = directory.path().join("bin");
    fs::create_dir(&bin).unwrap();
    let plugin = bin.join("pons-emit-python");
    fs::write(
        &plugin,
        r##"#!/bin/sh
if [ "$1" = describe ]; then
  printf '%s' '{"name":"python","version":"1.0.0","protocol":{"min":"3.0","max":"3.0"},"ir_version":{"min":"0.2","max":"0.2"},"capabilities":{"streaming":["sse","ndjson","bytes"],"enum_reprs":["string","external","internal","adjacent","untagged"],"direction_split":true,"provides":true,"flattened":true,"opaque":true,"object_openness":["reject","preserve_typed","preserve_opaque"],"extensions":{"test.example.source/v1":"exact","test.example.large/v1":"exact"}}}'
elif [ "$1" = emit ]; then
  cat > /dev/null
  printf 'log head\n' >&2
  dd if=/dev/zero bs=1048576 count=9 2>/dev/null | tr '\000' 'x' 1>&2
  printf 'log tail\n' >&2
  printf '%s' '{"protocol":3,"files":[{"path":"client.py","content":"generated"}],"coverage":{"exact":["types.EventQuery","types.LoginError"],"opaque":["operations.auth.login","operations.events.watch","types.LoginRequest","types.User"],"lossy":[],"unsupported":[]}}'
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
    fs::write(&input, FIXTURE).unwrap();
    let output_dir = directory.path().join("generated");
    let output = pons()
        .args(["emit", "--target", "python"])
        .arg(&input)
        .arg(&output_dir)
        .env("PATH", &path)
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.len() < 16 * 1024,
        "a 9 MiB log cannot become 9 MiB of terminal: {} bytes",
        stderr.len()
    );
    assert!(
        stderr.starts_with("..."),
        "{}",
        &stderr[..64.min(stderr.len())]
    );
    assert!(
        !stderr.contains("log head"),
        "{}",
        &stderr[..64.min(stderr.len())]
    );
    assert!(
        stderr.contains("log tail\n"),
        "{}",
        &stderr[..64.min(stderr.len())]
    );
    assert_eq!(
        fs::read_to_string(output_dir.join("client.py")).unwrap(),
        "generated"
    );
}
