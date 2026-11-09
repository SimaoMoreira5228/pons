mod support;

use std::fs;

use serde_json::Value;

use support::{FIXTURE, pons};

#[cfg(target_os = "linux")]
use support::{on_path, stub, stub_manifest};

#[cfg(target_os = "linux")]
#[test]
fn an_external_target_takes_its_options_from_the_file_and_puts_them_on_the_wire() {
    let directory = tempfile::tempdir().unwrap();
    let bin = directory.path().join("bin");
    stub(&bin, "python", &stub_manifest(""));
    stub(
        &directory.path().join("tools"),
        "python",
        &stub_manifest(r#"{"options":{"package":{"type":"string"},"timeout":{"type":"integer"}}}"#),
    );
    fs::create_dir_all(directory.path().join("contract")).unwrap();
    fs::write(directory.path().join("contract/api.json"), FIXTURE).unwrap();
    fs::write(
        directory.path().join("pons.toml"),
        "[project]\nir = \"contract/api.json\"\n\n[emit.python]\nout_dir = \"generated\"\npackage = \"acme-sdk\"\ntimeout = 30\n\n[plugins]\nsearch = [\"tools\"]\n",
    )
    .unwrap();
    let ran = directory.path().join("ran.json");

    let output = pons()
        .args(["emit", "--target", "python"])
        .current_dir(directory.path())
        .env("PATH", on_path(&bin))
        .env("PONS_REQUEST", &ran)
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stderr}");
    assert!(
        ran.exists(),
        "the emitter in the search path answered, not the one on PATH"
    );
    let sent: Value = serde_json::from_slice(&fs::read(&ran).unwrap()).unwrap();
    assert_eq!(
        sent["options"],
        serde_json::json!({ "package": "acme-sdk", "timeout": 30 }),
        "the table reaches the emitter, and the cli's own keys do not"
    );
    assert_eq!(sent["target"], "python");
    assert_eq!(sent["mode"], "strict");
    assert_eq!(sent["protocol"], 3);
    assert_eq!(sent["source_ir_version"], "0.3");
    assert!(directory.path().join("generated/acme/__init__.py").exists());
}

#[cfg(target_os = "linux")]
#[test]
fn an_option_the_emitter_does_not_declare_fails_the_command_before_it_runs() {
    let directory = tempfile::tempdir().unwrap();
    let bin = directory.path().join("bin");
    stub(
        &bin,
        "python",
        &stub_manifest(
            r#"{"options":{"package":{"type":"string","required":true},"timeout":{"type":"integer"},"retries":{"type":"array"},"strict_package":{"type":"boolean"}}}"#,
        ),
    );
    fs::write(directory.path().join("api.json"), FIXTURE).unwrap();
    let request = directory.path().join("request.json");

    for (table, expected) in [
        (
            "out_dir = \"generated\"\npackage = \"acme\"\nverboze = true\n",
            "option \"verboze\" is not one of the options python declares: package, retries, strict_package, timeout",
        ),
        (
            "out_dir = \"generated\"\n",
            "option \"package\" is required by python",
        ),
        (
            "out_dir = \"generated\"\npackage = \"acme\"\ntimeout = \"30\"\n",
            "option \"timeout\" is a string, but python declares it as an integer",
        ),
        (
            "out_dir = \"generated\"\npackage = \"acme\"\nretries = \"three\"\n",
            "option \"retries\" is a string, but python declares it as an array",
        ),
    ] {
        let _ = fs::remove_file(&request);
        fs::write(
            directory.path().join("pons.toml"),
            format!("[emit.python]\n{table}"),
        )
        .unwrap();

        let output = pons()
            .args(["emit", "--target", "python", "api.json", "generated"])
            .current_dir(directory.path())
            .env("PATH", on_path(&bin))
            .env("PONS_REQUEST", &request)
            .output()
            .unwrap();

        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success(), "{table}: {stderr}");
        assert_eq!(stderr.trim(), format!("pons: {expected}"), "{table}");
        assert!(!request.exists(), "{table}: emit ran anyway");
        assert!(!directory.path().join("generated").exists(), "{table}");
    }

    let _ = fs::remove_file(directory.path().join("pons.toml"));
    fs::write(
        directory.path().join("pons.toml"),
        "[emit.python]\nout_dir = \"generated\"\npackage = \"acme\"\nstrict_package = true\nretries = [1, 2]\n",
    )
    .unwrap();
    let output = pons()
        .args(["emit", "--target", "python", "api.json"])
        .current_dir(directory.path())
        .env("PATH", on_path(&bin))
        .env("PONS_REQUEST", &request)
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let sent: Value = serde_json::from_slice(&fs::read(&request).unwrap()).unwrap();
    assert_eq!(
        sent["options"],
        serde_json::json!({ "package": "acme", "retries": [1, 2], "strict_package": true })
    );
}

#[cfg(target_os = "linux")]
#[test]
fn an_emitter_with_no_option_schema_takes_an_empty_options_object() {
    let directory = tempfile::tempdir().unwrap();
    let bin = directory.path().join("bin");
    stub(&bin, "python", &stub_manifest(""));
    fs::write(directory.path().join("api.json"), FIXTURE).unwrap();
    fs::write(
        directory.path().join("pons.toml"),
        "[emit.python]\nout_dir = \"generated\"\npackage = \"acme-sdk\"\n",
    )
    .unwrap();
    let request = directory.path().join("request.json");

    let output = pons()
        .args(["emit", "--target", "python", "api.json"])
        .current_dir(directory.path())
        .env("PATH", on_path(&bin))
        .env("PONS_REQUEST", &request)
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        stderr.trim(),
        "pons: python declares no options, so it takes an empty options object; \"package\" was given"
    );
    assert!(!request.exists(), "emit ran anyway");

    fs::write(
        directory.path().join("pons.toml"),
        "[emit.python]\nout_dir = \"generated\"\n",
    )
    .unwrap();
    let output = pons()
        .args(["emit", "--target", "python", "api.json"])
        .current_dir(directory.path())
        .env("PATH", on_path(&bin))
        .env("PONS_REQUEST", &request)
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let sent: Value = serde_json::from_slice(&fs::read(&request).unwrap()).unwrap();
    assert_eq!(sent["options"], serde_json::json!({}));
}

#[cfg(target_os = "linux")]
#[test]
fn dump_request_writes_the_request_and_runs_no_emitter() {
    let directory = tempfile::tempdir().unwrap();
    let bin = directory.path().join("bin");
    stub(
        &bin,
        "python",
        &stub_manifest(r#"{"options":{"package":{"type":"string"}}}"#),
    );
    fs::write(directory.path().join("api.json"), FIXTURE).unwrap();
    fs::write(
        directory.path().join("pons.toml"),
        "[emit.python]\nout_dir = \"generated\"\npackage = \"acme-sdk\"\n",
    )
    .unwrap();
    let request = directory.path().join("build/request.json");
    let ran = directory.path().join("ran.json");

    let output = pons()
        .args([
            "emit",
            "--target",
            "python",
            "api.json",
            "--dump-request",
            "build/request.json",
        ])
        .current_dir(directory.path())
        .env("PATH", on_path(&bin))
        .env("PONS_REQUEST", &ran)
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!ran.exists(), "no emitter ran");
    assert!(!directory.path().join("generated").exists(), "no emit ran");
    let dumped = fs::read_to_string(&request).unwrap();
    assert!(
        dumped.ends_with('\n'),
        "a file a human reads ends in a newline"
    );
    let sent: Value = serde_json::from_str(&dumped).unwrap();
    assert_eq!(sent["target"], "python");
    assert_eq!(
        sent["options"],
        serde_json::json!({ "package": "acme-sdk" })
    );
    assert_eq!(sent["protocol"], 3);
    assert_eq!(sent["source_ir_version"], "0.3");
    assert_eq!(sent["mode"], "strict");

    let mut expected: pons_ir::Document = serde_json::from_str(FIXTURE).unwrap();
    expected.canonicalize();
    let document: pons_ir::Document = serde_json::from_value(sent["ir"].clone()).unwrap();
    assert_eq!(document, expected, "the dumped ir is the canonical one");
}

#[test]
fn dump_request_reports_the_options_a_built_in_target_will_run_with() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("api.json"), FIXTURE).unwrap();
    fs::write(
        directory.path().join("pons.toml"),
        "[emit.ts]\nout_dir = \"generated\"\nfield_case = \"camel\"\nint64 = \"string\"\n",
    )
    .unwrap();

    let output = pons()
        .args([
            "emit",
            "--target",
            "ts",
            "api.json",
            "--dump-request",
            "request.json",
            "--lossy",
        ])
        .current_dir(directory.path())
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let sent: Value =
        serde_json::from_str(&fs::read_to_string(directory.path().join("request.json")).unwrap())
            .unwrap();
    assert_eq!(sent["mode"], "lossy");
    assert_eq!(sent["options"]["field_case"], "camel");
    assert_eq!(sent["options"]["int64"], "string");
    assert_eq!(
        sent["options"]["mode"], "lossy",
        "a flag is in the dump too"
    );
    assert!(!directory.path().join("generated").exists(), "no emit ran");
}

#[test]
fn dump_request_refuses_a_path_the_emitter_output_rules_refuse() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("api.json"), FIXTURE).unwrap();
    fs::write(
        directory.path().join("pons.toml"),
        "[emit.ts]\nout_dir = \"generated\"\n",
    )
    .unwrap();

    for path in [
        "../escape.json",
        "sub/../out.json",
        "sub/CON.json",
        "nul.json",
        "a:b.json",
        "com1.json",
        "/tmp/pons-dump.json",
        "",
    ] {
        let output = pons()
            .args(["emit", "--target", "ts", "api.json", "--dump-request", path])
            .current_dir(directory.path())
            .output()
            .unwrap();

        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success(), "{path:?}: {stderr}");
        assert!(
            stderr.contains("dump-request path is not a safe relative path"),
            "{path:?}: {stderr}"
        );
        assert!(!directory.path().join("generated").exists(), "{path:?}");
    }
    assert!(
        !directory
            .path()
            .parent()
            .unwrap()
            .join("escape.json")
            .exists()
    );
}
