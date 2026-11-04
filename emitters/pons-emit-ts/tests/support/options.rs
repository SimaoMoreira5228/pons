#![allow(dead_code)]

use std::{fs, path::Path, process::Command};

use pons_emit_ts::{GeneratedFile, Runtime, TsOptions, emit_with_options};
use pons_ir::Document;
use serde_json::{Value, json};

pub fn fixture() -> Value {
    serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap()
}

pub fn emitted(document: &Document, options: TsOptions) -> Vec<GeneratedFile> {
    emit_with_options(document, options)
        .expect("the TypeScript emitter must accept the document")
        .files
}

pub fn file<'a>(files: &'a [GeneratedFile], path: &str) -> &'a str {
    &files
        .iter()
        .find(|file| file.path == path)
        .unwrap_or_else(|| panic!("the emitter wrote no {path}"))
        .content
}

pub fn file_of(document: &Document, options: TsOptions, path: &str) -> String {
    let files = emitted(document, options);
    file(&files, path).to_owned()
}

pub fn run_bun(files: &[GeneratedFile], harness: &str) {
    run_bun_with(files, harness, Runtime::Vendor)
}

pub fn run_bun_with(files: &[GeneratedFile], harness: &str, runtime: Runtime) {
    let version = Command::new("bun")
        .arg("--version")
        .output()
        .expect("Bun is required for generated TypeScript runtime tests");
    assert!(version.status.success());
    let directory = tempfile::tempdir().unwrap();
    for file in files {
        let destination = directory.path().join(&file.path);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(destination, &file.content).unwrap();
    }
    if runtime == Runtime::Package {
        install_runtime_package(directory.path());
    }
    let harness_path = directory.path().join("options.ts");
    fs::write(&harness_path, harness).unwrap();
    let output = Command::new("bun")
        .arg("run")
        .arg(&harness_path)
        .current_dir(directory.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn install_runtime_package(output: &Path) {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("runtime");
    let installed = output.join("runtime");
    fs::create_dir_all(&installed).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        if name.to_string_lossy().ends_with(".ts") || name == "package.json" {
            fs::copy(entry.path(), installed.join(&name)).unwrap();
        }
    }
    let scope = output.join("node_modules").join("@pons");
    fs::create_dir_all(&scope).unwrap();
    let link = scope.join("runtime");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&installed, &link).unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_dir(&installed, &link).unwrap();
}

pub fn operation(id: Value, path: &str) -> Value {
    json!({
        "id": id,
        "method": "GET",
        "path": [{"lit": path}],
        "auth": {"alternatives": [], "optional": false},
        "semantics": {"safe": true, "idempotent": true},
        "responses": [{"status": 200, "content": "json", "body": {"kind": "string"}}]
    })
}

pub fn with_operations(operations: Vec<Value>) -> Document {
    let raw = json!({
        "ir_version": pons_ir::IR_VERSION,
        "info": {"title": "Options", "version": "1"},
        "operations": operations,
    });
    let document: Document = serde_json::from_value(raw).unwrap();
    document.validate().unwrap();
    document
}

pub fn paths(files: &[GeneratedFile]) -> Vec<String> {
    let mut paths = files
        .iter()
        .map(|file| file.path.clone())
        .collect::<Vec<_>>();
    paths.sort();
    paths
}
