mod support;

use std::fs;
use std::path::Path;

use serde_json::Value;

use support::{
    BUILT_INS, FIXTURE, FIXTURE_DIRECTORY, FIXTURES, PRESERVED_SUBJECTS, coverage_for, coverage_in,
    coverage_levels, fixture, fixture_path, lossy_openapi_input, pons, rejecting_openness_input,
    wide_number_input,
};

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
    let errors = fs::read_to_string(output_dir.join("errors.ts")).unwrap();
    let client = fs::read_to_string(output_dir.join("client.ts")).unwrap();
    assert!(types.contains("export type User"));
    assert!(client.contains("export function createClient"));
    assert!(errors.contains("neverthrow"));
    assert!(client.contains("EventsWatchEvent"));
}

#[test]
fn emit_writes_requested_coverage() {
    let report = coverage_for("ts", FIXTURE);
    let coverage = &report["coverage"];
    assert_eq!(coverage["opaque"], serde_json::json!(PRESERVED_SUBJECTS));
    assert_eq!(
        coverage["exact"],
        serde_json::json!(["types.EventQuery", "types.LoginError"])
    );
    assert!(coverage["lossy"].as_array().unwrap().is_empty());
    assert!(coverage["unsupported"].as_array().unwrap().is_empty());
    assert_eq!(
        report["validation_coverage"],
        serde_json::json!({"exact": [], "lossy": [], "unsupported": []}),
        "the default backend writes no validators and an empty report"
    );
}

#[test]
fn emit_writes_the_coverage_envelope_with_the_validators_report() {
    let directory = tempfile::tempdir().unwrap();
    let output_dir = directory.path().join("generated");
    let input_path = directory.path().join("api.json");
    fs::write(&input_path, FIXTURE).unwrap();
    let output = pons()
        .args(["emit", "--target", "ts"])
        .arg(&input_path)
        .arg(&output_dir)
        .args(["--validation", "builtin"])
        .args(["--coverage", "coverage.json"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let report: Value =
        serde_json::from_str(&fs::read_to_string(output_dir.join("coverage.json")).unwrap())
            .unwrap();
    assert_eq!(
        report.as_object().unwrap().keys().collect::<Vec<_>>(),
        vec!["coverage", "validation_coverage"],
        "{report}"
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

#[test]
fn targets_never_claim_more_capability_than_canonical_coverage() {
    for input in [FIXTURE.to_owned(), rejecting_openness_input()] {
        let document: pons_ir::Document = serde_json::from_str(&input).unwrap();
        let canonical = pons_ir::canonical_coverage(&document, false).unwrap();
        let reported = ["openapi", "ts"]
            .map(|target| (target, coverage_levels(&coverage_for(target, &input))));

        for (target, levels) in &reported {
            assert_eq!(
                levels.keys().collect::<Vec<_>>(),
                canonical.keys().collect::<Vec<_>>(),
                "{target} coverage subjects"
            );
        }
        for (subject, level) in &canonical {
            for (target, levels) in &reported {
                assert!(
                    levels[subject] >= *level,
                    "{target} reports {subject} as {} where canonical coverage is {}",
                    levels[subject].as_str(),
                    level.as_str()
                );
            }
        }
    }
}

#[test]
fn every_target_classifies_every_fixture_the_same_way() {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join(FIXTURE_DIRECTORY);
    let mut found = fs::read_dir(&directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".json"))
        .collect::<Vec<_>>();
    found.sort();
    let mut rows = FIXTURES
        .iter()
        .map(|row| row.file.to_owned())
        .collect::<Vec<_>>();
    rows.sort();
    assert_eq!(
        found, rows,
        "every fixture in {FIXTURE_DIRECTORY} needs a row, and every row needs a fixture"
    );

    for row in &FIXTURES {
        let input = fixture(row.file);
        let checked = pons()
            .arg("check")
            .arg(fixture_path(row.file))
            .output()
            .unwrap();
        assert!(
            checked.status.success(),
            "{} must be a valid document: {}",
            row.file,
            String::from_utf8_lossy(&checked.stderr)
        );

        for (lossy, expected) in [(false, row.strict), (true, row.lossy)] {
            let mode = if lossy { "--lossy" } else { "--strict" };
            for (index, target) in BUILT_INS.iter().enumerate() {
                let at = format!("{} ({}) {target} {mode}", row.file, row.carries);
                let reported = coverage_in(target, &input, lossy).unwrap_or_else(|message| {
                    panic!("{at} refused a document it must describe: {message}")
                });
                assert_eq!(coverage_levels(&reported), expected[index].levels(), "{at}");
            }
        }
    }
}

#[test]
fn emit_rejects_coverage_paths_that_alias_output_files() {
    let directory = tempfile::tempdir().unwrap();
    let output_dir = directory.path().join("generated");
    let input_path = directory.path().join("api.json");
    fs::write(&input_path, FIXTURE).unwrap();
    let output = pons()
        .args(["emit", "--target", "openapi"])
        .arg(&input_path)
        .arg(&output_dir)
        .args(["--coverage", "openapi.json/coverage.json"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("emitter output path conflict"));
    assert!(!output_dir.exists());
}

#[test]
fn emit_rejects_coverage_paths_the_plugin_protocol_would_also_refuse() {
    let directory = tempfile::tempdir().unwrap();
    let input_path = directory.path().join("api.json");
    fs::write(&input_path, FIXTURE).unwrap();

    for coverage in [
        "NUL",
        "nul.json",
        "a:b.json",
        "sub/CON.txt",
        "nested/aux",
        "com1.json",
        "LPT9.json",
        "coverage/../escape.json",
        "",
    ] {
        let output_dir = directory
            .path()
            .join(format!("generated{}", coverage.replace(['/', ':'], "_")));
        let output = pons()
            .args(["emit", "--target", "openapi"])
            .arg(&input_path)
            .arg(&output_dir)
            .args(["--coverage", coverage])
            .output()
            .unwrap();

        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(5), "{coverage:?}: {stderr}");
        assert!(
            stderr.contains("safe relative path"),
            "{coverage:?}: {stderr}"
        );
        assert!(!output_dir.exists(), "{coverage:?} wrote to {output_dir:?}");
    }
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
    let errors = fs::read_to_string(output_dir.join("errors.ts")).unwrap();
    let auth = fs::read_to_string(output_dir.join("operations/auth.ts")).unwrap();
    assert!(auth.contains("requestOrThrow(request<"));
    assert!(!errors.contains("from \"neverthrow\""));
}

#[test]
fn emit_rejects_the_error_strategy_for_non_typescript_targets() {
    let directory = tempfile::tempdir().unwrap();
    let input_path = directory.path().join("api.json");
    fs::write(&input_path, FIXTURE).unwrap();

    for target in ["openapi", "fixture"] {
        for strategy in ["neverthrow", "throw"] {
            let output_dir = directory
                .path()
                .join(format!("generated-{target}-{strategy}"));
            let output = pons()
                .args(["emit", "--target", target])
                .arg(&input_path)
                .arg(&output_dir)
                .args(["--errors", strategy])
                .output()
                .unwrap();

            let stderr = String::from_utf8_lossy(&output.stderr);
            assert_eq!(
                output.status.code(),
                Some(4),
                "{target} --errors {strategy}"
            );
            assert_eq!(
                stderr.trim(),
                format!("pons: --errors is only supported by the TypeScript target, not {target}"),
                "{target} --errors {strategy}"
            );
            assert!(!output_dir.exists(), "{target} --errors {strategy}");
        }
    }
}

#[test]
fn emit_rejects_a_flag_supplied_as_another_flag_s_value() {
    let directory = tempfile::tempdir().unwrap();
    let input_path = directory.path().join("api.json");
    fs::write(&input_path, FIXTURE).unwrap();

    for (flag, value) in [
        ("--coverage", "--lossy"),
        ("--coverage", "--strict"),
        ("--errors", "--lossy"),
        ("--errors", "--coverage"),
    ] {
        let output_dir = directory
            .path()
            .join(format!("generated{flag}{value}").replace('-', ""));
        let output = pons()
            .args(["emit", "--target", "ts"])
            .arg(&input_path)
            .arg(&output_dir)
            .args([flag, value])
            .output()
            .unwrap();

        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(4), "{flag} {value}: {stderr}");
        assert!(stderr.contains(flag), "{flag} {value}: {stderr}");
        assert!(stderr.contains(value), "{flag} {value}: {stderr}");
        assert!(
            !output_dir.exists(),
            "{flag} {value} wrote to {}",
            output_dir.display()
        );
    }
}

#[test]
fn emit_accepts_a_repeated_coverage_flag_and_rejects_a_conflict() {
    let directory = tempfile::tempdir().unwrap();
    let input = wide_number_input(directory.path());

    for (flag, warns) in [("--strict", false), ("--lossy", true)] {
        let output_dir = directory.path().join(format!("generated-{flag}"));
        let output = pons()
            .args(["emit", "--target", "ts"])
            .arg(&input)
            .arg(&output_dir)
            .args([flag, flag])
            .output()
            .unwrap();

        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "{flag} {flag}: {stderr}");
        assert_eq!(
            stderr.contains("wide numeric integers"),
            warns,
            "{flag} {flag} must behave like {flag}: {stderr}"
        );
        assert!(output_dir.join("types.ts").exists(), "{flag} {flag}");
    }

    for flags in [["--strict", "--lossy"], ["--lossy", "--strict"]] {
        let output_dir = directory.path().join(format!("generated{flags:?}"));
        let output = pons()
            .args(["emit", "--target", "ts"])
            .arg(&input)
            .arg(&output_dir)
            .args(flags)
            .output()
            .unwrap();

        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(4), "{flags:?}: {stderr}");
        assert!(stderr.contains("--strict"), "{flags:?}: {stderr}");
        assert!(stderr.contains("--lossy"), "{flags:?}: {stderr}");
        assert!(!output_dir.exists(), "{flags:?} wrote to {output_dir:?}");
    }
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

#[test]
fn emit_lossy_openapi_reports_non_exact_coverage() {
    let directory = tempfile::tempdir().unwrap();
    let input = lossy_openapi_input(directory.path());
    let output_dir = directory.path().join("generated");
    let output = pons()
        .args(["emit", "--target", "openapi"])
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
    assert!(String::from_utf8_lossy(&output.stderr).contains("lossy coverage"));
    assert!(output_dir.join("openapi.json").exists());
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
