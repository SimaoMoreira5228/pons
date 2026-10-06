use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

const FIXTURE: &str = include_str!("../../../fixtures/login.json");
const FIXTURE_DIRECTORY: &str = "../../fixtures";

fn pons() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pons"))
}

fn fixture_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(FIXTURE_DIRECTORY)
        .join(name)
}

fn fixture(name: &str) -> String {
    fs::read_to_string(fixture_path(name)).unwrap()
}

fn wide_number_input(directory: &Path) -> PathBuf {
    let path = directory.join("api.json");
    fs::write(&path, fixture("lossy.json")).unwrap();
    path
}

fn lossy_openapi_input(directory: &Path) -> std::path::PathBuf {
    let path = directory.join("api.json");
    let mut contract: Value = serde_json::from_str(FIXTURE).unwrap();
    contract["types"]["LoginError"]["variants"][1]["payload"]["fields"][0]["ty"]["wire"] =
        serde_json::json!("number");
    contract["types"]["LoginError"]["variants"][1]["payload"]["fields"][0]["ty"]["bits"] =
        serde_json::json!(64);
    fs::write(&path, serde_json::to_vec(&contract).unwrap()).unwrap();
    path
}

fn custom_auth_input(directory: &Path) -> std::path::PathBuf {
    let path = directory.join("custom-api.json");
    let mut contract: Value = serde_json::from_str(FIXTURE).unwrap();
    contract["auth_schemes"]["tenant"] =
        serde_json::json!({ "kind": "custom", "description": "tenant handshake" });
    fs::write(&path, serde_json::to_vec(&contract).unwrap()).unwrap();
    path
}

fn rejecting_openness_input() -> String {
    let mut contract: Value = serde_json::from_str(FIXTURE).unwrap();
    for id in ["User", "LoginRequest"] {
        contract["types"][id]["openness"] = serde_json::json!("reject");
    }
    serde_json::to_string(&contract).unwrap()
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
    assert_eq!(output.status.code(), Some(2));
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
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .contains("BREAKING operations.auth.login.semantics: operation idempotency changed")
    );
}

#[test]
fn diff_returns_nonzero_for_type_name_changes() {
    let directory = tempfile::tempdir().unwrap();
    let old = directory.path().join("old.json");
    let new = directory.path().join("new.json");
    fs::write(&old, FIXTURE).unwrap();
    let mut input: Value = serde_json::from_str(FIXTURE).unwrap();
    input["types"]["User"]["name"] = serde_json::json!(["account"]);
    fs::write(&new, serde_json::to_vec(&input).unwrap()).unwrap();

    let output = pons().arg("diff").arg(&old).arg(&new).output().unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("BREAKING types.User: type name changed")
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
fn diff_reports_an_invalid_new_contract_with_its_own_exit_code() {
    let directory = tempfile::tempdir().unwrap();
    let old = directory.path().join("old.json");
    let new = directory.path().join("new.json");
    fs::write(&old, FIXTURE).unwrap();
    let mut input: Value = serde_json::from_str(FIXTURE).unwrap();
    input["types"]["User"]["diagnostics"] = serde_json::json!([{
        "severity": "error",
        "code": "type.x",
        "message": "unresolved"
    }]);
    fs::write(&new, serde_json::to_vec(&input).unwrap()).unwrap();

    let output = pons().arg("diff").arg(&old).arg(&new).output().unwrap();
    assert_eq!(output.status.code(), Some(3));
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .contains("BREAKING document: the new document is invalid: type User diagnostics")
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("error diagnostic type.x: unresolved")
    );
}

#[test]
fn diff_passes_a_repair_of_an_invalid_previous_contract() {
    let directory = tempfile::tempdir().unwrap();
    let old = directory.path().join("old.json");
    let new = directory.path().join("new.json");
    let mut input: Value = serde_json::from_str(FIXTURE).unwrap();
    input["types"]["User"]["fields"][1]["default"] = serde_json::json!(7);
    fs::write(&old, serde_json::to_vec(&input).unwrap()).unwrap();
    fs::write(&new, FIXTURE).unwrap();

    let output = pons().arg("diff").arg(&old).arg(&new).output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("INFO document: the previous document was invalid"));
    assert!(stdout.contains("NON_BREAKING types.User.fields.nickname: field default changed"));
}

#[test]
fn diff_passes_a_warning_only_change() {
    let directory = tempfile::tempdir().unwrap();
    let old = directory.path().join("old.json");
    let new = directory.path().join("new.json");
    fs::write(&old, FIXTURE).unwrap();
    let mut input: Value = serde_json::from_str(FIXTURE).unwrap();
    input["operations"][0]["body"]["ext"] = serde_json::json!({"test.example.source/v1": 1});
    fs::write(&new, serde_json::to_vec(&input).unwrap()).unwrap();

    let output = pons().arg("diff").arg(&old).arg(&new).output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stderr.is_empty());
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .contains("WARNING operations.auth.login.body: request body metadata changed")
    );
}

#[test]
fn only_unchanged_or_non_breaking_contracts_exit_below_two() {
    let directory = tempfile::tempdir().unwrap();
    let old = directory.path().join("old.json");
    let new = directory.path().join("new.json");
    let mut input: Value = serde_json::from_str(FIXTURE).unwrap();
    fs::write(&old, FIXTURE).unwrap();

    fs::write(&new, FIXTURE).unwrap();
    assert_eq!(
        pons()
            .arg("diff")
            .arg(&old)
            .arg(&new)
            .output()
            .unwrap()
            .status
            .code(),
        Some(0)
    );

    input["info"]["title"] = serde_json::json!("Renamed");
    fs::write(&new, serde_json::to_vec(&input).unwrap()).unwrap();
    let code = pons()
        .arg("diff")
        .arg(&old)
        .arg(&new)
        .output()
        .unwrap()
        .status
        .code();
    assert!(
        code.is_some_and(|code| code < 2),
        "{code:?} must read as success"
    );

    let mut breaking: Value = serde_json::from_str(FIXTURE).unwrap();
    breaking["types"]["User"]["fields"][0]["ident"] = serde_json::json!(["userId"]);
    fs::write(&new, serde_json::to_vec(&breaking).unwrap()).unwrap();
    let code = pons()
        .arg("diff")
        .arg(&old)
        .arg(&new)
        .output()
        .unwrap()
        .status
        .code();
    assert!(
        code.is_some_and(|code| code >= 2),
        "{code:?} must read as failure"
    );

    for arguments in [
        vec!["check", "missing.json"],
        vec!["diff", "missing.json", "missing.json"],
        vec!["nonsense", FIXTURE],
    ] {
        let output = pons()
            .args(&arguments)
            .current_dir(directory.path())
            .output()
            .unwrap();
        assert!(
            output.status.code().is_some_and(|code| code >= 2),
            "{arguments:?} exited {:?}",
            output.status.code()
        );
    }
}

#[test]
fn diff_reports_two_invalid_contracts_with_its_own_exit_code() {
    let directory = tempfile::tempdir().unwrap();
    let old = directory.path().join("old.json");
    let new = directory.path().join("new.json");
    let mut input: Value = serde_json::from_str(FIXTURE).unwrap();
    input["types"]["User"]["fields"][1]["default"] = serde_json::json!(7);
    fs::write(&old, serde_json::to_vec(&input).unwrap()).unwrap();
    input["info"]["title"] = serde_json::json!("Renamed");
    fs::write(&new, serde_json::to_vec(&input).unwrap()).unwrap();

    let output = pons().arg("diff").arg(&old).arg(&new).output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(output.status.code(), Some(3), "{stdout}");
    assert!(
        stdout.contains("WARNING document: both documents are invalid: "),
        "{stdout}"
    );
    assert!(stdout.contains("the previous document: "), "{stdout}");
    assert!(stdout.contains("the new document: "), "{stdout}");
    assert!(!stdout.contains("BREAKING"), "{stdout}");
}

#[test]
fn diff_reports_an_unreadable_contract_with_its_own_exit_code() {
    let directory = tempfile::tempdir().unwrap();
    let old = directory.path().join("a.json");
    fs::write(&old, FIXTURE).unwrap();
    let typo = directory.path().join("typo.json");

    let output = pons().arg("diff").arg(&old).arg(&typo).output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(5), "{stderr}");
    assert!(stderr.contains("cannot read"), "{stderr}");
    assert!(stderr.contains("typo.json"), "{stderr}");
    assert_eq!(String::from_utf8_lossy(&output.stdout), "");
}

#[test]
fn help_documents_every_exit_code() {
    let output = pons().arg("--help").output().unwrap();
    let help = String::from_utf8_lossy(&output.stdout);
    for line in ["0  ", "1  ", "2  ", "3  ", "4  ", "5  "] {
        assert!(help.contains(line), "{line} missing from the help");
    }
    assert!(help.contains("breaking"), "{help}");
    assert!(help.contains("unreadable"), "{help}");
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
    let errors = fs::read_to_string(output_dir.join("errors.ts")).unwrap();
    let client = fs::read_to_string(output_dir.join("client.ts")).unwrap();
    assert!(types.contains("export type User"));
    assert!(client.contains("export function createClient"));
    assert!(errors.contains("neverthrow"));
    assert!(client.contains("EventsWatchEvent"));
}

const PRESERVED_SUBJECTS: [&str; 4] = [
    "operations.auth.login",
    "operations.events.watch",
    "types.LoginRequest",
    "types.User",
];

fn coverage_for(target: &str, input: &str) -> Value {
    coverage_in(target, input, false)
        .unwrap_or_else(|error| panic!("{target} refused the document: {error}"))
}

fn coverage_in(target: &str, input: &str, lossy: bool) -> Result<Value, String> {
    let directory = tempfile::tempdir().unwrap();
    let output_dir = directory.path().join("generated");
    let input_path = directory.path().join("api.json");
    fs::write(&input_path, input).unwrap();
    let mut command = pons();
    command
        .args(["emit", "--target", target])
        .arg(&input_path)
        .arg(&output_dir)
        .args(["--coverage", "coverage.json"]);
    if lossy {
        command.arg("--lossy");
    }
    let output = command.output().unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    if !output.status.success() {
        return Err(stderr
            .trim()
            .strip_prefix("pons: ")
            .unwrap_or(stderr.trim())
            .to_owned());
    }
    let reported = fs::read_to_string(output_dir.join("coverage.json")).unwrap();
    Ok(serde_json::from_str(&reported).unwrap())
}

fn coverage_levels(coverage: &Value) -> BTreeMap<String, pons_ir::CoverageLevel> {
    let mut levels = BTreeMap::new();
    for (level, subjects) in [
        (pons_ir::CoverageLevel::Exact, &coverage["exact"]),
        (pons_ir::CoverageLevel::Opaque, &coverage["opaque"]),
        (pons_ir::CoverageLevel::Lossy, &coverage["lossy"]),
        (
            pons_ir::CoverageLevel::Unsupported,
            &coverage["unsupported"],
        ),
    ] {
        for subject in subjects.as_array().unwrap() {
            let subject = subject.as_str().unwrap();
            assert!(
                levels.insert(subject.to_owned(), level).is_none(),
                "{subject}"
            );
        }
    }
    levels
}

#[test]
fn emit_writes_requested_coverage() {
    let coverage = coverage_for("ts", FIXTURE);
    assert_eq!(coverage["opaque"], serde_json::json!(PRESERVED_SUBJECTS));
    assert_eq!(
        coverage["exact"],
        serde_json::json!(["types.EventQuery", "types.LoginError"])
    );
    assert!(coverage["lossy"].as_array().unwrap().is_empty());
    assert!(coverage["unsupported"].as_array().unwrap().is_empty());
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

const BUILT_INS: [&str; 2] = ["ts", "openapi"];

/// The subjects one fixture's classification puts at each level, so a row can
/// name the whole answer rather than a difference.
#[derive(Clone, Copy)]
struct Levels {
    exact: &'static [&'static str],
    opaque: &'static [&'static str],
    lossy: &'static [&'static str],
    unsupported: &'static [&'static str],
}

impl Levels {
    fn levels(&self) -> BTreeMap<String, pons_ir::CoverageLevel> {
        [
            (pons_ir::CoverageLevel::Exact, self.exact),
            (pons_ir::CoverageLevel::Opaque, self.opaque),
            (pons_ir::CoverageLevel::Lossy, self.lossy),
            (pons_ir::CoverageLevel::Unsupported, self.unsupported),
        ]
        .into_iter()
        .flat_map(|(level, subjects)| {
            subjects
                .iter()
                .map(move |subject| ((*subject).to_owned(), level))
        })
        .collect()
    }
}

const PRESERVED_LEVELS: Levels = Levels {
    exact: &["types.EventQuery", "types.LoginError"],
    opaque: &PRESERVED_SUBJECTS,
    lossy: &[],
    unsupported: &[],
};

const EVERY_SUBJECT_EXACT: Levels = Levels {
    exact: &[
        "operations.auth.login",
        "operations.events.watch",
        "types.EventQuery",
        "types.LoginError",
        "types.LoginRequest",
        "types.User",
    ],
    opaque: &[],
    lossy: &[],
    unsupported: &[],
};

const WIDE_INTEGER_LOSSY: Levels = Levels {
    exact: &["types.EventQuery", "types.LoginError"],
    opaque: &["types.LoginRequest"],
    lossy: &[
        "operations.auth.login",
        "operations.events.watch",
        "types.User",
    ],
    unsupported: &[],
};

/// What every built-in target must do with a fixture: the coverage map they all
/// report, or the refusal each one prints for a feature it cannot describe.
#[derive(Clone, Copy)]
enum Expected {
    Coverage(Levels),
    Refused(&'static [&'static str]),
}

struct Row {
    file: &'static str,
    carries: &'static str,
    strict: Expected,
    lossy: Expected,
}

const NO_WEB_SOCKET: [&str; 2] = [
    "the TypeScript emitter supports SSE and NDJSON streams only",
    "OpenAPI paths cannot describe WebSocket operations",
];

const FIXTURES: [Row; 4] = [
    Row {
        file: "login.json",
        carries: "opaque",
        strict: Expected::Coverage(PRESERVED_LEVELS),
        lossy: Expected::Coverage(PRESERVED_LEVELS),
    },
    Row {
        file: "exact.json",
        carries: "exact",
        strict: Expected::Coverage(EVERY_SUBJECT_EXACT),
        lossy: Expected::Coverage(EVERY_SUBJECT_EXACT),
    },
    Row {
        file: "lossy.json",
        carries: "lossy",
        strict: Expected::Coverage(PRESERVED_LEVELS),
        lossy: Expected::Coverage(WIDE_INTEGER_LOSSY),
    },
    Row {
        file: "unsupported.json",
        carries: "unsupported",
        strict: Expected::Refused(&NO_WEB_SOCKET),
        lossy: Expected::Refused(&NO_WEB_SOCKET),
    },
];

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
            // One expectation for both targets, so agreement is the assertion.
            for (index, target) in BUILT_INS.iter().enumerate() {
                let at = format!("{} ({}) {target} {mode}", row.file, row.carries);
                match (expected, coverage_in(target, &input, lossy)) {
                    (Expected::Coverage(levels), Ok(reported)) => {
                        assert_eq!(coverage_levels(&reported), levels.levels(), "{at}");
                    }
                    (Expected::Refused(messages), Err(message)) => {
                        assert_eq!(message, messages[index], "{at}");
                    }
                    (Expected::Coverage(_), Err(message)) => {
                        panic!("{at} refused a document it must describe: {message}");
                    }
                    (Expected::Refused(_), Ok(reported)) => {
                        panic!("{at} described a document it cannot: {reported}");
                    }
                }
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
fn malformed_arguments_exit_with_the_usage_code_and_a_one_line_diagnosis() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("api.json"), FIXTURE).unwrap();

    for (arguments, expected) in [
        (vec![], "no command"),
        (vec!["check", "a", "b"], "check expects one document"),
        (vec!["fmt", "a", "b"], "fmt expects one document"),
        (vec!["diff", "a"], "diff expects two documents"),
        (
            vec!["emit", "--target", "ts", "api.json"],
            "emit expects --target <name> <ir.json> <output-dir>",
        ),
        (
            vec!["emit", "api.json", "api.json", "api.json", "out"],
            "not \"api.json\"",
        ),
        (
            vec!["emit", "--target", "ts", "api.json", "out", "--nonsense"],
            "--nonsense",
        ),
        (vec!["nonsense", "api.json"], "unknown command \"nonsense\""),
    ] {
        let output = pons()
            .args(&arguments)
            .current_dir(directory.path())
            .output()
            .unwrap();

        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(4), "{arguments:?}: {stderr}");
        assert!(stderr.starts_with("pons: "), "{arguments:?}: {stderr}");
        assert_eq!(stderr.lines().count(), 1, "{arguments:?}: {stderr}");
        assert!(stderr.contains(expected), "{arguments:?}: {stderr}");
        assert!(!directory.path().join("out").exists(), "{arguments:?}");
    }
}

#[test]
fn help_is_printed_on_stdout() {
    for argument in ["--help", "-h", "help"] {
        let output = pons().arg(argument).output().unwrap();
        assert!(output.status.success(), "{argument}");
        assert!(String::from_utf8_lossy(&output.stdout).starts_with("Usage: pons"));
        assert!(output.stderr.is_empty(), "{argument}");
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
  printf '%s' '{"name":"fixture","version":"1.0.0","protocol":{"min":"2.0","max":"2.0"},"ir_version":{"min":"0.2","max":"0.2"},"capabilities":{"streaming":["sse"],"enum_reprs":["string","external","internal","adjacent","untagged"],"direction_split":true,"provides":true,"flattened":true,"object_openness":["reject","preserve_typed","preserve_opaque"]}}'
elif [ "$1" = emit ]; then
  request=$(cat)
  printf '%s' "$request" > "$PONS_REQUEST"
  printf '%s' '{"protocol":2,"files":[{"path":"nested/client.ts","content":"generated"}],"coverage":{"exact":["types.EventQuery","types.LoginError"],"opaque":["operations.auth.login","operations.events.watch","types.LoginRequest","types.User"],"lossy":[],"unsupported":[]}}'
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
  printf '%s' '{"name":"fixture","version":"1.0.0","protocol":{"min":"2.0","max":"2.0"},"ir_version":{"min":"0.2","max":"0.2"},"capabilities":{"streaming":["sse"],"enum_reprs":["string","external","internal","adjacent","untagged"],"direction_split":true,"provides":true,"flattened":true,"opaque":true,"object_openness":["reject","preserve_typed","preserve_opaque"]}}'
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
    *'"bits":64,"signed":false,"wire":"number"'*'"target":"fixture"'*) printf '%s' '{"protocol":2,"files":[{"path":"nested/client.ts","content":"generated"}],"coverage":{"exact":["types.EventQuery","types.LoginError"],"opaque":[],"lossy":["operations.auth.login","operations.events.watch","types.LoginRequest","types.User"],"unsupported":[]}}' ;;
    *'"target":"fixture"'*) printf '%s' '{"protocol":2,"files":[{"path":"nested/client.ts","content":"generated"}],"coverage":{"exact":["types.EventQuery","types.LoginError"],"opaque":["operations.auth.login","operations.events.watch","types.LoginRequest","types.User"],"lossy":[],"unsupported":[]}}' ;;
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
  printf '%s' '{"name":"python","version":"1.0.0","protocol":{"min":"2.0","max":"2.0"},"ir_version":{"min":"0.2","max":"0.2"},"capabilities":{"streaming":["sse"],"enum_reprs":["string","external","internal","adjacent","untagged"],"direction_split":true,"provides":true,"flattened":true,"opaque":true,"object_openness":["reject","preserve_typed","preserve_opaque"]}}'
elif [ "$1" = emit ]; then
  cat > /dev/null
  printf '%s' '{"protocol":2,"files":[{"path":"client.py","content":"generated"}],"coverage":{"exact":["types.EventQuery","types.LoginError"],"opaque":["operations.auth.login","operations.events.watch","types.LoginRequest"],"lossy":["types.User"],"unsupported":[]}}'
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
  printf '%s' '{"name":"python","version":"1.0.0","protocol":{"min":"2.0","max":"2.0"},"ir_version":{"min":"0.2","max":"0.2"},"capabilities":{"streaming":["sse"],"enum_reprs":["string","external","internal","adjacent","untagged"],"direction_split":true,"provides":true,"flattened":true,"opaque":true,"object_openness":["reject","preserve_typed","preserve_opaque"]}}'
elif [ "$1" = emit ]; then
  request=$(cat)
  printf '%s' "$request" > "$PONS_REQUEST"
  printf '%s' '{"protocol":2,"files":[{"path":"client.py","content":"generated"}],"coverage":{"exact":["types.EventQuery","types.LoginError"],"opaque":["operations.auth.login","types.LoginRequest","types.User"],"lossy":[],"unsupported":["operations.events.watch"]}}'
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
  printf '%s' '{"name":"python","version":"1.0.0","protocol":{"min":"2.0","max":"2.0"},"ir_version":{"min":"0.2","max":"0.2"},"capabilities":{"streaming":["sse","ndjson","bytes"],"enum_reprs":["string","external","internal","adjacent","untagged"],"direction_split":true,"provides":true,"flattened":true,"opaque":true,"object_openness":["reject","preserve_typed","preserve_opaque"]}}'
elif [ "$1" = emit ]; then
  cat > /dev/null
  printf 'log head\n' >&2
  dd if=/dev/zero bs=1048576 count=9 2>/dev/null | tr '\000' 'x' 1>&2
  printf 'log tail\n' >&2
  printf '%s' '{"protocol":2,"files":[{"path":"client.py","content":"generated"}],"coverage":{"exact":["types.EventQuery","types.LoginError"],"opaque":["operations.auth.login","operations.events.watch","types.LoginRequest","types.User"],"lossy":[],"unsupported":[]}}'
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
