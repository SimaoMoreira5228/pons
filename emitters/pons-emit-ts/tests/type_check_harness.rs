mod support;

use std::path::Path;

use pons_emit_ts::GeneratedFile;
use pons_ir::Document;
use serde_json::Value;

use support::type_check::{
    PROBE, PROJECT, checked_files, compiler_candidates, files, fixture, is_compiler_version,
    tsconfig,
};

#[test]
fn every_candidate_invokes_the_typescript_compiler_and_nothing_else() {
    let runners = compiler_candidates()
        .iter()
        .map(|candidate| (candidate.program.clone(), candidate.arguments.clone()))
        .collect::<Vec<_>>();
    let arguments = |program: &str| {
        runners
            .iter()
            .find(|(name, _)| name == program)
            .map(|(_, arguments)| arguments.clone())
    };
    let expected = |arguments: [&str; 3]| arguments.map(String::from).to_vec();
    assert_eq!(
        arguments("bunx"),
        Some(expected(["--package", "typescript", "tsc"])),
        "bunx tsc resolves the deprecated npm package named tsc"
    );
    assert_eq!(
        arguments("npx"),
        Some(expected(["-p", "typescript", "tsc"])),
        "npx tsc resolves the deprecated npm package named tsc"
    );
    assert_eq!(
        arguments("tsc"),
        Some(Vec::new()),
        "`tsc tsc --project` mixes a source file with --project, which the compiler refuses as TS5042"
    );
    for (program, arguments) in &runners {
        if Path::new(program)
            .file_name()
            .is_some_and(|name| name == "tsc" || name == "tsc.cmd")
        {
            assert!(
                arguments.is_empty(),
                "{program} is the compiler itself, so `tsc tsc --project` would be TS5042: {arguments:?}"
            );
        }
    }
}

#[test]
fn a_program_that_only_prints_a_banner_is_not_a_compiler() {
    assert!(is_compiler_version("Version 5.9.2\n"));
    assert!(is_compiler_version("  Version 6.0.3\r\n"));
    assert!(!is_compiler_version(""));
    assert!(!is_compiler_version(
        "This is not the tsc command you are looking for\n\nTo get access to the \
         TypeScript compiler, tsc, from the command line either:\n\n- Use `npm install \
         typescript`\n"
    ));
}

#[test]
fn the_check_covers_every_file_the_emitter_produced() {
    let document: Document = serde_json::from_value(fixture()).unwrap();
    let files = files(&document);
    let checked = checked_files(&files);
    assert!(!files.is_empty());
    for file in &files {
        assert!(
            checked.contains(&file.path),
            "{} escapes the type check",
            file.path
        );
    }
    assert_eq!(checked.last().map(String::as_str), Some(PROBE));
    let config: Value = serde_json::from_str(&tsconfig(&checked)).unwrap();
    assert_eq!(
        config["files"],
        serde_json::to_value(&checked).unwrap(),
        "{PROJECT} must list the emitted files, not a pinned list"
    );

    let extra = GeneratedFile {
        path: "runtime/extra.ts".into(),
        content: "export const extra = 1;\n".into(),
    };
    let grown = checked_files(&[files, vec![extra]].concat());
    assert!(grown.contains(&"runtime/extra.ts".to_string()), "{grown:?}");
}

#[test]
fn the_check_covers_the_vendored_runtime_sources() {
    let document: Document = serde_json::from_value(fixture()).unwrap();
    let files = files(&document);
    let checked = checked_files(&files);
    for module in [
        "runtime.ts",
        "runtime/index.ts",
        "runtime/wire.ts",
        "runtime/codec.ts",
        "runtime/validate.ts",
        "runtime/stream.ts",
        "runtime/pagination.ts",
        "runtime/credentials.ts",
        "runtime/transport.ts",
        "runtime/transport/spec.ts",
        "runtime/transport/failures.ts",
        "runtime/transport/preflight.ts",
        "runtime/transport/status.ts",
        "runtime/transport/retry.ts",
        "runtime/transport/response-body.ts",
        "runtime/transport/response.ts",
        "runtime/transport/redirect.ts",
        "runtime/transport/request.ts",
        "runtime/websocket.ts",
    ] {
        assert!(
            checked.contains(&module.to_string()),
            "{module} escapes the type check: {checked:?}"
        );
    }
    let config: Value = serde_json::from_str(&tsconfig(&checked)).unwrap();
    assert_eq!(config["files"], serde_json::to_value(&checked).unwrap());
}
