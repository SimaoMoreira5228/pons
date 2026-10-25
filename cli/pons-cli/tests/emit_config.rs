mod support;

use std::fs;

use serde_json::Value;

use support::{FIXTURE, pons};

#[test]
fn a_config_file_supplies_the_document_and_the_output_directory() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("api.json"), FIXTURE).unwrap();
    fs::write(
        directory.path().join("pons.toml"),
        "[project]\nir = \"api.json\"\n\n[emit.ts]\nout_dir = \"generated\"\n",
    )
    .unwrap();

    let output = pons()
        .args(["emit", "--target", "ts"])
        .current_dir(directory.path())
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(directory.path().join("generated/types.ts").exists());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("does not understand the extension namespace test.example.source/v1"),
        "the only thing the run has to say is the extension the target ignores: {stderr}"
    );
}

#[test]
fn a_config_file_is_found_from_the_working_directory_and_from_the_document() {
    let directory = tempfile::tempdir().unwrap();
    let nested = directory.path().join("contract");
    fs::create_dir(&nested).unwrap();
    fs::write(nested.join("api.json"), FIXTURE).unwrap();
    fs::write(
        directory.path().join("pons.toml"),
        "[project]\nir = \"contract/api.json\"\n\n[emit.ts]\nout_dir = \"generated\"\n",
    )
    .unwrap();

    let output = pons()
        .args(["emit", "--target", "ts", "api.json"])
        .current_dir(&nested)
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        directory.path().join("generated/types.ts").exists(),
        "the paths in the file are relative to the file, so a subdirectory emits the same tree"
    );
}

#[test]
fn the_typescript_options_reach_the_built_in_target() {
    let directory = tempfile::tempdir().unwrap();
    let mut contract: Value = serde_json::from_str(FIXTURE).unwrap();
    contract["types"]["User"]["fields"][0]["wire"] = serde_json::json!("user_id");
    contract["types"]["User"]["fields"][0]["ident"] = serde_json::json!(["user", "id"]);
    fs::write(directory.path().join("api.json"), contract.to_string()).unwrap();
    fs::write(
        directory.path().join("pons.toml"),
        "[emit.ts]\nout_dir = \"generated\"\nfield_case = \"camel\"\nnamespaces = \"flat\"\n",
    )
    .unwrap();

    let output = pons()
        .args(["emit", "--target", "ts", "api.json"])
        .current_dir(directory.path())
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let types = fs::read_to_string(directory.path().join("generated/types.ts")).unwrap();
    assert!(
        types.contains("\"userId\": string"),
        "the camel spelling is the caller's key and the wire name is untouched: {types}"
    );
    assert!(
        directory
            .path()
            .join("generated/operations/auth.login.ts")
            .exists(),
        "a flat member is the whole id joined by dots"
    );
    assert!(!directory.path().join("generated/operations/auth").exists());
}

#[test]
fn a_zod_contract_emits_the_validators_a_consumer_can_use() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("api.json"), FIXTURE).unwrap();
    fs::write(
        directory.path().join("pons.toml"),
        "[emit.ts]\nout_dir = \"generated\"\n\n[emit.ts.validation]\nbackend = \"zod\"\n",
    )
    .unwrap();

    let output = pons()
        .args(["emit", "--target", "ts", "api.json"])
        .current_dir(directory.path())
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stderr}");

    let validators = fs::read_to_string(directory.path().join("generated/validators.ts")).unwrap();
    assert!(
        validators.contains("import { z } from \"zod\";"),
        "{validators}"
    );
    assert!(validators.contains("function mismatch"), "{validators}");
    assert!(
        validators.contains("export const validateUser"),
        "{validators}"
    );
    assert!(
        !directory.path().join("generated/schemas.ts").exists(),
        "the zod backend writes its validators in one module"
    );
    let index = fs::read_to_string(directory.path().join("generated/index.ts")).unwrap();
    assert!(index.contains("./validators.js"), "{index}");
    assert!(
        directory.path().join("generated/types.ts").exists(),
        "the option adds a file, it does not replace the contract"
    );
}

#[test]
fn a_builtin_contract_emits_the_validators_the_runtime_walks() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("api.json"), FIXTURE).unwrap();
    fs::write(
        directory.path().join("pons.toml"),
        "[emit.ts]\nout_dir = \"generated\"\n\n[emit.ts.validation]\nbackend = \"builtin\"\n",
    )
    .unwrap();

    let output = pons()
        .args(["emit", "--target", "ts", "api.json"])
        .current_dir(directory.path())
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stderr}");

    let validators = fs::read_to_string(directory.path().join("generated/validators.ts")).unwrap();
    assert!(validators.contains("validateContract"), "{validators}");
    assert!(
        validators.contains("export const validateUser"),
        "{validators}"
    );
    let index = fs::read_to_string(directory.path().join("generated/index.ts")).unwrap();
    assert!(index.contains("./validators.js"), "{index}");
}

#[test]
fn a_bare_validation_word_is_refused_with_the_table_spelling() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("api.json"), FIXTURE).unwrap();
    fs::write(
        directory.path().join("pons.toml"),
        "[emit.ts]\nout_dir = \"generated\"\nvalidation = \"zod\"\n",
    )
    .unwrap();

    let output = pons()
        .args(["emit", "--target", "ts", "api.json"])
        .current_dir(directory.path())
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(4), "{stderr}");
    assert!(
        stderr.contains("validation is a string; it is a table")
            && stderr.contains("[emit.ts.validation]")
            && stderr.contains("backend = \"zod\""),
        "{stderr}"
    );
    assert!(!directory.path().join("generated").exists());
}

#[test]
fn a_present_validation_table_without_a_backend_is_builtin() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("api.json"), FIXTURE).unwrap();
    fs::write(
        directory.path().join("pons.toml"),
        "[emit.ts]\nout_dir = \"generated\"\n\n[emit.ts.validation]\n",
    )
    .unwrap();

    let output = pons()
        .args(["emit", "--target", "ts", "api.json"])
        .current_dir(directory.path())
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stderr}");
    let validators = fs::read_to_string(directory.path().join("generated/validators.ts")).unwrap();
    assert!(validators.contains("validateContract"), "{validators}");
}

#[test]
fn the_validation_flag_beats_the_file() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("api.json"), FIXTURE).unwrap();
    fs::write(
        directory.path().join("pons.toml"),
        "[emit.ts]\nout_dir = \"generated\"\n\n[emit.ts.validation]\nbackend = \"builtin\"\n",
    )
    .unwrap();

    let output = pons()
        .args(["emit", "--target", "ts", "api.json", "--validation", "zod"])
        .current_dir(directory.path())
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stderr}");
    let validators = fs::read_to_string(directory.path().join("generated/validators.ts")).unwrap();
    assert!(
        validators.contains("import { z } from \"zod\";"),
        "{validators}"
    );
    assert!(!validators.contains("validateContract"), "{validators}");
}

#[test]
fn the_validation_flag_is_the_typescript_targets_own() {
    for target in ["openapi", "python"] {
        let directory = tempfile::tempdir().unwrap();
        fs::write(directory.path().join("api.json"), FIXTURE).unwrap();
        let output = pons()
            .args([
                "emit",
                "--target",
                target,
                "api.json",
                "--validation",
                "zod",
            ])
            .current_dir(directory.path())
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(4), "{target}: {stderr}");
        assert!(
            stderr.contains(&format!(
                "--validation is only supported by the TypeScript target, not {target}"
            )),
            "{target}: {stderr}"
        );
    }
}

#[test]
fn the_validation_flag_refuses_an_unknown_backend_and_ignores_a_stale_import() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("api.json"), FIXTURE).unwrap();

    let output = pons()
        .args([
            "emit",
            "--target",
            "ts",
            "api.json",
            "--validation",
            "pydantic",
        ])
        .current_dir(directory.path())
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(4), "{stderr}");
    assert!(
        stderr.contains("--validation expects") && stderr.contains("not \"pydantic\""),
        "{stderr}"
    );

    fs::write(
        directory.path().join("pons.toml"),
        "[emit.ts]\nout_dir = \"generated\"\n\n[emit.ts.validation]\nbackend = \"zod\"\nimport_from = \"zod/v4\"\n",
    )
    .unwrap();
    let output = pons()
        .args([
            "emit",
            "--target",
            "ts",
            "api.json",
            "--validation",
            "builtin",
        ])
        .current_dir(directory.path())
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "a flag that selects a backend with no package ignores the file's stale import: {stderr}"
    );
    let validators = fs::read_to_string(directory.path().join("generated/validators.ts")).unwrap();
    assert!(validators.contains("validateContract"), "{validators}");
    assert!(!validators.contains("from \"zod\""), "{validators}");

    fs::write(
        directory.path().join("pons.toml"),
        "[emit.ts]\nout_dir = \"generated\"\n\n[emit.ts.validation]\nbackend = \"builtin\"\nimport_from = \"zod\"\n",
    )
    .unwrap();
    let output = pons()
        .args(["emit", "--target", "ts", "api.json"])
        .current_dir(directory.path())
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(4), "{stderr}");
    assert!(
        stderr.contains("the builtin backend imports no package"),
        "{stderr}"
    );
}

#[test]
fn a_packaged_runtime_is_an_import_the_consumers_build_resolves() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("api.json"), FIXTURE).unwrap();
    fs::write(
        directory.path().join("pons.toml"),
        "[emit.ts]\nout_dir = \"generated\"\nruntime = \"package\"\n",
    )
    .unwrap();

    let output = pons()
        .args(["emit", "--target", "ts", "api.json"])
        .current_dir(directory.path())
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stderr}");

    let client = fs::read_to_string(directory.path().join("generated/client.ts")).unwrap();
    assert!(client.contains("\"@pons/runtime\""), "{client}");
    assert!(
        !directory.path().join("generated/runtime.ts").exists(),
        "a packaged runtime is not vendored"
    );
    assert!(
        !directory.path().join("generated/runtime").exists(),
        "a packaged runtime is not vendored"
    );
}

#[test]
fn a_misspelled_option_names_the_file_the_table_and_the_key() {
    for (source, expected) in [
        (
            "[project]\nir = \"api.json\"\n\n[emit.ts]\nout_dir = \"generated\"\nfieldcase = \"camel\"\n",
            ": [emit.ts] has no key \"fieldcase\"; it is mode, errors, namespaces, field_case, newtypes, int64, date_mode, runtime, validation, out_dir",
        ),
        (
            "[emit.ts]\nout_dir = \"generated\"\nfield_case = \"Camel\"\n",
            ": [emit.ts] field_case is \"Camel\"; it is wire or camel",
        ),
        (
            "[emit.openapi]\nfield_case = \"camel\"\n",
            ": [emit.openapi] has no key \"field_case\"; it is mode, out_dir",
        ),
        (
            "[project]\nout_dir = \"generated\"\n",
            ": [project] has no key \"out_dir\"; it is ir, name",
        ),
        (
            "[frontend]\nkind = \"axum\"\nrunner = \"x\"\n",
            ": [frontend] has no key \"runner\"; it is kind, command",
        ),
        (
            "[emmit.ts]\nout_dir = \"generated\"\n",
            ": has no key \"emmit\"; it is emit, frontend, plugins, project",
        ),
    ] {
        let directory = tempfile::tempdir().unwrap();
        fs::write(directory.path().join("api.json"), FIXTURE).unwrap();
        fs::write(directory.path().join("pons.toml"), source).unwrap();

        let output = pons()
            .args(["emit", "--target", "ts", "api.json"])
            .current_dir(directory.path())
            .output()
            .unwrap();

        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(4), "{source}: {stderr}");
        assert!(
            stderr.contains("pons.toml") && stderr.contains(expected),
            "{source}: {stderr}"
        );
        assert!(!directory.path().join("generated").exists(), "{source}");
    }
}
