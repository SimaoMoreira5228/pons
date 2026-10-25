mod support;

use std::fs;

use serde_json::Value;

use support::{FIXTURE, pons};

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
