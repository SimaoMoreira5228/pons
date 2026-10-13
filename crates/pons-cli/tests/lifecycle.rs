use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use pons_ir::Document;
use serde_json::Value;

const FIXTURE: &str = include_str!("../../../fixtures/login.json");

fn pons() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pons"))
}

fn cargo_pons() -> Command {
    Command::new(env!("CARGO_BIN_EXE_cargo-pons"))
}

#[cfg(unix)]
fn executable(directory: &Path, name: &str, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let path = directory.join(name);
    fs::write(&path, body).unwrap();
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&path, permissions).unwrap();
    path
}

#[test]
fn init_writes_a_starter_that_the_config_reads() {
    let directory = tempfile::tempdir().unwrap();
    let output = pons()
        .arg("init")
        .current_dir(directory.path())
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let path = directory.path().join("pons.toml");
    let source = fs::read_to_string(&path).unwrap();
    let name = directory.path().file_name().unwrap().to_str().unwrap();
    assert!(source.contains(&format!("name = {name:?}")), "{source}");
    assert!(source.contains("ir = \"api/ir.json\""), "{source}");
    assert!(source.contains("kind = \"axum\""), "{source}");
    assert!(
        source.contains("command = \"cargo pons export\""),
        "{source}"
    );
    assert!(source.contains("out_dir = \"web/src/api\""), "{source}");
    assert!(source.contains("mode = \"strict\""), "{source}");
    assert!(!directory.path().join("api").exists());
}

#[test]
fn init_takes_a_path_and_creates_only_its_parent() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("acme").join("contract.toml");

    let output = pons().arg("init").arg(&path).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let source = fs::read_to_string(&path).unwrap();
    assert!(source.contains("name = \"acme\""), "{source}");
    assert!(!directory.path().join("acme/api").exists());
}

#[test]
fn init_refuses_an_existing_file() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("pons.toml");
    fs::write(&path, "hand written\n").unwrap();

    let output = pons().arg("init").arg(&path).output().unwrap();

    assert_eq!(output.status.code(), Some(5));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("already exists"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), "hand written\n");
}

#[cfg(unix)]
fn export_project(directory: &Path, script: &str) -> PathBuf {
    let root = directory.join("project");
    fs::create_dir(&root).unwrap();
    executable(&root, "frontend.sh", script);
    let mut contract: Value = serde_json::from_str(FIXTURE).unwrap();
    contract["operations"].as_array_mut().unwrap().reverse();
    fs::write(
        root.join("api.json"),
        serde_json::to_vec(&contract).unwrap(),
    )
    .unwrap();
    fs::write(
        root.join("pons.toml"),
        "[project]\nname = \"acme\"\nir = \"api/ir.json\"\n\n[frontend]\nkind = \"axum\"\ncommand = \"./frontend.sh\"\n",
    )
    .unwrap();
    root
}

#[cfg(unix)]
#[test]
fn export_writes_the_canonical_ir() {
    let directory = tempfile::tempdir().unwrap();
    let root = export_project(directory.path(), "#!/bin/sh\ncat api.json\n");

    let output = pons().arg("export").current_dir(&root).output().unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let written = root.join("api/ir.json");
    let printed = String::from_utf8_lossy(&output.stdout);
    assert!(printed.trim().ends_with("api/ir.json"), "{printed}");

    let mut expected: Document = serde_json::from_str(FIXTURE).unwrap();
    expected.canonicalize();
    let expected = format!("{}\n", expected.to_canonical_json().unwrap());
    assert_eq!(fs::read_to_string(&written).unwrap(), expected);
}

#[cfg(unix)]
#[test]
fn export_leaves_the_old_ir_when_the_frontend_fails() {
    let directory = tempfile::tempdir().unwrap();
    let root = export_project(directory.path(), "#!/bin/sh\nexit 7\n");
    let written = root.join("api/ir.json");
    fs::create_dir(root.join("api")).unwrap();
    fs::write(&written, "old ir\n").unwrap();

    let output = pons().arg("export").current_dir(&root).output().unwrap();

    assert_eq!(output.status.code(), Some(5));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("failed with"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(fs::read_to_string(&written).unwrap(), "old ir\n");
}

#[cfg(unix)]
#[test]
fn export_leaves_the_old_ir_when_the_frontend_writes_invalid_ir() {
    let directory = tempfile::tempdir().unwrap();
    let root = export_project(directory.path(), "#!/bin/sh\nprintf '{}'\n");
    let written = root.join("api/ir.json");
    fs::create_dir(root.join("api")).unwrap();
    fs::write(&written, "old ir\n").unwrap();

    let output = pons().arg("export").current_dir(&root).output().unwrap();

    assert_eq!(output.status.code(), Some(5));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("produced invalid IR"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(fs::read_to_string(&written).unwrap(), "old ir\n");
}

#[test]
fn export_needs_a_project_ir_and_a_frontend() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(
        directory.path().join("pons.toml"),
        "[project]\nname = \"acme\"\n",
    )
    .unwrap();

    let output = pons()
        .arg("export")
        .current_dir(directory.path())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(4));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("has no ir"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    fs::write(
        directory.path().join("pons.toml"),
        "[project]\nname = \"acme\"\nir = \"api/ir.json\"\n",
    )
    .unwrap();
    let output = pons()
        .arg("export")
        .current_dir(directory.path())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(4));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("needs a [frontend] table"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(unix)]
fn fake_cargo(directory: &Path, body: &str) -> PathBuf {
    executable(directory, "cargo", body)
}

#[cfg(unix)]
#[test]
fn cargo_pons_runs_the_default_example_and_forwards_its_output() {
    let directory = tempfile::tempdir().unwrap();
    let args = directory.path().join("args");
    let cargo = fake_cargo(
        directory.path(),
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$PONS_TEST_ARGS\"\nprintf 'from the example\\n' >&1\nprintf 'from the build\\n' >&2\n",
    );

    let output = cargo_pons()
        .args(["export"])
        .env("CARGO", &cargo)
        .env("PONS_TEST_ARGS", &args)
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "from the example\n"
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("from the build"));
    assert_eq!(
        fs::read_to_string(&args).unwrap(),
        "run\n--quiet\n--example\npons_export\n"
    );
}

#[cfg(unix)]
#[test]
fn cargo_pons_forwards_the_example_and_manifest_through_cargo() {
    let directory = tempfile::tempdir().unwrap();
    let args = directory.path().join("args");
    let cargo = fake_cargo(
        directory.path(),
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$PONS_TEST_ARGS\"\n",
    );

    let output = cargo_pons()
        .args([
            "pons",
            "export",
            "--example",
            "custom_export",
            "--manifest-path",
            "backend/Cargo.toml",
        ])
        .env("CARGO", &cargo)
        .env("PONS_TEST_ARGS", &args)
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read_to_string(&args).unwrap(),
        "run\n--quiet\n--example\ncustom_export\n--manifest-path\nbackend/Cargo.toml\n"
    );
}

#[cfg(unix)]
#[test]
fn cargo_pons_propagates_a_non_zero_exit() {
    let directory = tempfile::tempdir().unwrap();
    let cargo = fake_cargo(directory.path(), "#!/bin/sh\nexit 7\n");

    let output = cargo_pons()
        .args(["export"])
        .env("CARGO", &cargo)
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(7));
}
