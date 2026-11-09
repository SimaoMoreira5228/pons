mod support;

use pons_emit_ts::{TsOptions, ValidationBackend, ValidationOptions, emit_with_options};
use pons_ir::Document;
use serde_json::json;
use support::options::{emitted, file, fixture, paths};

#[test]
fn validation_coverage_names_the_subjects_the_backend_checks() {
    let document: Document = serde_json::from_value(fixture()).unwrap();
    let none = emit_with_options(&document, TsOptions::default()).unwrap();
    assert_eq!(
        none.validation_coverage,
        pons_ir::ValidationCoverage::default(),
        "the default backend writes no validators and reports nothing"
    );

    let subjects = pons_ir::coverage_subjects(&document);
    for backend in [ValidationBackend::Builtin, ValidationBackend::Zod] {
        let output = emit_with_options(
            &document,
            TsOptions {
                validation: ValidationOptions::new(backend),
                ..TsOptions::default()
            },
        )
        .unwrap();
        assert_eq!(output.validation_coverage.exact, subjects, "{backend:?}");
        assert!(output.validation_coverage.lossy.is_empty(), "{backend:?}");
        assert!(
            output.validation_coverage.unsupported.is_empty(),
            "{backend:?}"
        );
        assert_eq!(
            serde_json::to_value(&output.validation_coverage).unwrap(),
            json!({
                "exact": subjects,
                "lossy": [],
                "unsupported": []
            }),
            "{backend:?}"
        );
    }
}

#[test]
fn a_document_with_only_input_types_writes_no_validators_file() {
    let document: Document = serde_json::from_value(json!({
        "ir_version": "0.3",
        "info": {"title": "Input", "version": "1"},
        "types": {
            "Filter": {
                "kind": "struct",
                "name": ["filter"],
                "direction": "input",
                "openness": "reject",
                "fields": [{
                    "wire": "term",
                    "ident": ["term"],
                    "ty": {"kind": "string"},
                    "presence": "required"
                }]
            }
        },
        "operations": [{
            "id": [["search"], ["run"]],
            "method": "POST",
            "path": [{"lit": "search"}],
            "body": {"ty": {"ref": "Filter"}, "content": "json", "required": true},
            "responses": [{"status": 204, "content": "empty"}],
            "auth": {"alternatives": [], "optional": false},
            "semantics": {"safe": false, "idempotent": false}
        }]
    }))
    .unwrap();

    for backend in [ValidationBackend::Builtin, ValidationBackend::Zod] {
        let output = emit_with_options(
            &document,
            TsOptions {
                validation: ValidationOptions::new(backend),
                ..TsOptions::default()
            },
        )
        .unwrap();
        assert!(
            !output.files.iter().any(|file| file.path == "validators.ts"),
            "{backend:?} wrote a file with no validator in it"
        );
        let index = file(&output.files, "index.ts");
        assert!(
            !index.contains("validators.js"),
            "{backend:?} re-exports a file it did not write: {index}"
        );
    }
}

#[test]
fn the_zod_option_writes_the_validators_the_response_is_checked_against() {
    let document: Document = serde_json::from_value(fixture()).unwrap();
    let zod = emitted(
        &document,
        TsOptions {
            validation: ValidationOptions::new(ValidationBackend::Zod),
            ..TsOptions::default()
        },
    );
    let validators = file(&zod, "validators.ts");
    assert!(
        validators.contains("import { z } from \"zod\";"),
        "{validators}"
    );
    assert!(
        validators.contains("const UserSchema"),
        "a named type carries a private schema: {validators}"
    );
    assert!(
        validators.contains("function mismatch(schema: z.ZodType)"),
        "the check the validators call is private beside them: {validators}"
    );
    assert!(
        validators.contains("export const validateUser: Validator = mismatch(UserSchema);"),
        "a named type is a validator the client imports: {validators}"
    );
    let operations = file(&zod, "operations/auth.ts");
    assert!(
        operations.contains(
            "import { validateLoginErrorInvalid, validateUser } from \"../validators.js\";"
        ),
        "the operation file imports what it calls: {operations}"
    );
    assert!(
        operations.contains("validate: validateUser"),
        "a declared response carries the check: {operations}"
    );

    assert!(
        file(&zod, "index.ts").contains("export * from \"./validators.js\";"),
        "the entry point reaches the validators it checked against: {}",
        file(&zod, "index.ts")
    );

    let none = emitted(&document, TsOptions::default());
    assert!(!paths(&none).contains(&"validators.ts".to_owned()));
    assert!(
        !file(&none, "operations/auth.ts").contains("validateUser"),
        "the default writes the bytes it wrote before the option did anything"
    );
    assert_eq!(
        file(&none, "index.ts"),
        "export * from \"./errors.js\";\nexport * from \"./client.js\";\n",
        "the default's entry point is one line"
    );
    assert_eq!(
        paths(&none),
        paths(&zod)
            .into_iter()
            .filter(|path| path != "validators.ts")
            .collect::<Vec<_>>(),
        "the validator file is the only file the option adds"
    );
}

#[test]
fn the_zod_backend_imports_the_specifier_the_option_names() {
    let document: Document = serde_json::from_value(fixture()).unwrap();
    let files = emitted(
        &document,
        TsOptions {
            validation: ValidationOptions {
                backend: ValidationBackend::Zod,
                import_from: Some("zod/v4".into()),
            },
            ..TsOptions::default()
        },
    );
    assert!(
        file(&files, "validators.ts").contains("import { z } from \"zod/v4\";"),
        "{}",
        file(&files, "validators.ts")
    );
}

#[test]
fn the_builtin_backend_ignores_import_from() {
    let document: Document = serde_json::from_value(fixture()).unwrap();
    let files = emitted(
        &document,
        TsOptions {
            validation: ValidationOptions {
                backend: ValidationBackend::Builtin,
                import_from: Some("zod/v4".into()),
            },
            ..TsOptions::default()
        },
    );
    let validators = file(&files, "validators.ts");
    assert!(validators.contains("validateContract"), "{validators}");
    assert!(!validators.contains("zod"), "{validators}");
}
