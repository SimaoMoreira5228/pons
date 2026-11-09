mod support;

use std::fs;

use serde_json::Value;

use support::{FIXTURE, pons};

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
    let input = FIXTURE.replace("\"ir_version\": \"0.3\"", "\"ir_version\": \"9.0\"");
    fs::write(&path, &input).unwrap();

    let output = pons().arg("fmt").arg(&path).output().unwrap();
    assert!(!output.status.success());
    assert_eq!(fs::read_to_string(path).unwrap(), input);
}
