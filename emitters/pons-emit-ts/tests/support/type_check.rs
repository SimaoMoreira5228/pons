use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use pons_emit_ts::{EmitMode, ErrorStrategy, GeneratedFile, emit_with_strategy};
use pons_ir::Document;
use serde_json::Value;

pub(crate) const PROBE: &str = "probe.ts";
pub(crate) const PROJECT: &str = "tsconfig.json";

const COMPILER_OPTIONS: &str = r#"{
    "target": "es2022",
    "module": "esnext",
    "moduleResolution": "bundler",
    "lib": ["es2022", "dom", "dom.iterable"],
    "types": [],
    "strict": true,
    "exactOptionalPropertyTypes": true,
    "noUncheckedIndexedAccess": true,
    "noUnusedLocals": true,
    "noUnusedParameters": true,
    "skipLibCheck": true,
    "noEmit": true
  }"#;

pub(crate) fn tsconfig(checked: &[String]) -> String {
    format!(
        r#"{{
  "compilerOptions": {COMPILER_OPTIONS},
  "files": [{}]
}}
"#,
        checked
            .iter()
            .map(|path| serde_json::to_string(path).unwrap())
            .collect::<Vec<_>>()
            .join(", ")
    )
}

pub(crate) fn checked_files(files: &[GeneratedFile]) -> Vec<String> {
    let mut checked = files
        .iter()
        .map(|file| file.path.clone())
        .collect::<Vec<_>>();
    assert!(
        !checked.iter().any(|path| path == PROBE),
        "the emitter must not write {PROBE}, which the harness owns"
    );
    checked.push(PROBE.into());
    checked
}

pub(crate) fn fixture() -> Value {
    serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap()
}

pub(crate) fn files(document: &Document) -> Vec<GeneratedFile> {
    emit_with_strategy(document, EmitMode::Strict, ErrorStrategy::Throw)
        .expect("the TypeScript emitter must accept the document")
        .files
}

pub(crate) fn files_with(
    document: &Document,
    options: pons_emit_ts::TsOptions,
) -> Vec<GeneratedFile> {
    pons_emit_ts::emit_with_options(
        document,
        pons_emit_ts::TsOptions {
            errors: ErrorStrategy::Throw,
            ..options
        },
    )
    .expect("the TypeScript emitter must accept the document")
    .files
}

pub(crate) fn types_of(files: &[GeneratedFile]) -> &str {
    &files
        .iter()
        .find(|file| file.path == "types.ts")
        .unwrap()
        .content
}

pub(crate) fn client_of(files: &[GeneratedFile]) -> &str {
    &files
        .iter()
        .find(|file| file.path == "client.ts")
        .unwrap()
        .content
}

pub(crate) fn operations_of<'a>(files: &'a [GeneratedFile], member: &str) -> &'a str {
    &files
        .iter()
        .find(|file| file.path == format!("operations/{member}.ts"))
        .unwrap()
        .content
}

#[derive(Clone, Debug)]
pub(crate) struct Compiler {
    pub(crate) program: String,
    pub(crate) arguments: Vec<String>,
}

pub(crate) fn compiler_candidates() -> Vec<Compiler> {
    let workspace = workspace_typescript().map(|path| Compiler {
        program: path.display().to_string(),
        arguments: Vec::new(),
    });
    [
        workspace,
        Some(Compiler {
            program: "tsc".into(),
            arguments: Vec::new(),
        }),
        Some(Compiler {
            program: "bunx".into(),
            arguments: ["--package", "typescript", "tsc"]
                .map(String::from)
                .to_vec(),
        }),
        Some(Compiler {
            program: "npx".into(),
            arguments: ["-p", "typescript", "tsc"].map(String::from).to_vec(),
        }),
    ]
    .into_iter()
    .flatten()
    .collect()
}

pub(crate) fn workspace_typescript() -> Option<PathBuf> {
    super::bin_directories()
        .into_iter()
        .flat_map(|bin| [bin.join("tsc"), bin.join("tsc.cmd")])
        .find(|candidate| candidate.is_file())
}

pub(crate) fn is_compiler_version(stdout: &str) -> bool {
    stdout
        .lines()
        .any(|line| line.trim_start().starts_with("Version "))
}

pub(crate) fn tsc_command() -> Compiler {
    let candidates = compiler_candidates();
    for candidate in &candidates {
        let Ok(output) = Command::new(&candidate.program)
            .args(&candidate.arguments)
            .arg("--version")
            .output()
        else {
            continue;
        };
        if output.status.success() && is_compiler_version(&String::from_utf8_lossy(&output.stdout))
        {
            return candidate.clone();
        }
    }
    panic!(
        "no TypeScript compiler for the generated TypeScript type tests, tried {candidates:?} and \
         every tsc and tsc.cmd in {directories:?}; the deprecated npm package named tsc prints a \
         banner instead of compiling, so install the compiler itself with \
         `npm install --save-dev typescript` or `bun add --dev typescript`",
        directories = super::bin_directories(),
    );
}

pub(crate) fn type_checks(files: &[GeneratedFile], probe: &str) {
    type_checks_with(files, probe, false)
}

pub(crate) fn type_checks_with(files: &[GeneratedFile], probe: &str, packaged: bool) {
    type_checks_with_packages(files, probe, packaged, &[])
}

pub(crate) fn type_checks_beside(files: &[GeneratedFile], probe: &str, packaged: bool, zod: bool) {
    let packages = if zod { vec!["zod"] } else { Vec::new() };
    type_checks_with_packages(files, probe, packaged, &packages);
}

pub(crate) fn type_checks_with_packages(
    files: &[GeneratedFile],
    probe: &str,
    packaged: bool,
    packages: &[&str],
) {
    let compiler = tsc_command();
    let checked = checked_files(files);
    let directory = tempfile::tempdir().unwrap();
    for file in files {
        let destination = directory.path().join(&file.path);
        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        fs::write(destination, &file.content).unwrap();
    }
    fs::write(directory.path().join(PROBE), probe).unwrap();
    fs::write(directory.path().join(PROJECT), tsconfig(&checked)).unwrap();
    let mut expected = checked.clone();
    if packaged {
        expected.extend(install_runtime_package(directory.path()));
    }
    for package in packages {
        install_package(directory.path(), package);
    }
    let mut written = written_sources(directory.path());
    written.sort();
    expected.sort();
    assert_eq!(
        written, expected,
        "the type check must cover every emitted file and the runtime beside them, listed in \
         {PROJECT}"
    );
    let output = Command::new(&compiler.program)
        .args(&compiler.arguments)
        .args(["--project", PROJECT])
        .current_dir(directory.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "tsc refused the emitted client\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

pub(crate) fn install_package(output: &Path, name: &str) {
    let package = super::package(name);
    let scope = output.join("node_modules");
    fs::create_dir_all(&scope).unwrap();
    let link = scope.join(name);
    #[cfg(unix)]
    std::os::unix::fs::symlink(&package, &link).unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_dir(&package, &link).unwrap();
}

pub(crate) fn install_runtime_package(output: &Path) -> Vec<String> {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("runtime");
    let installed = output.join("runtime");
    fs::create_dir_all(&installed).unwrap();
    let mut written = Vec::new();
    for entry in fs::read_dir(&source).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        if name.to_string_lossy().ends_with(".ts") {
            fs::copy(entry.path(), installed.join(&name)).unwrap();
            written.push(format!("runtime/{}", name.to_string_lossy()));
        }
    }
    fs::copy(source.join("package.json"), installed.join("package.json")).unwrap();
    let scope = output.join("node_modules").join("@pons");
    fs::create_dir_all(&scope).unwrap();
    let link = scope.join("runtime");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&installed, &link).unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_dir(&installed, &link).unwrap();
    written
}

pub(crate) fn written_sources(root: &Path) -> Vec<String> {
    let mut written = Vec::new();
    let mut directories = vec![root.to_path_buf()];
    while let Some(directory) = directories.pop() {
        for entry in fs::read_dir(&directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                if path.file_name().is_some_and(|name| name == "node_modules") {
                    continue;
                }
                directories.push(path);
            } else if path.extension().is_some_and(|extension| extension == "ts") {
                written.push(
                    path.strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/"),
                );
            }
        }
    }
    written
}
