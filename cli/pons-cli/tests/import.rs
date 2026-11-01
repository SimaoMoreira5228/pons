mod support;

use std::fs;
use std::path::Path;

use serde_json::Value;

use support::pons;

fn fixture() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../frontends/pons-openapi/testdata/orders.json")
}

#[test]
fn imports_an_openapi_document_to_canonical_json() {
    let output = pons()
        .arg("import")
        .arg("openapi")
        .arg(fixture())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let canonical = String::from_utf8(output.stdout).unwrap();
    let document: Value = serde_json::from_str(&canonical).unwrap();
    assert_eq!(document["ir_version"], "0.2");
    assert_eq!(document["info"]["title"], "Orders");
    let ids = document["operations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|operation| operation["id"].clone())
        .collect::<Vec<_>>();
    assert!(ids.contains(&serde_json::json!([["orders"], ["create"]])));

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("contract.json");
    let output = pons()
        .arg("import")
        .arg("openapi")
        .arg(fixture())
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), canonical);

    let invalid = directory.path().join("old.json");
    fs::write(&invalid, br#"{"openapi":"3.0.3"}"#).unwrap();
    let output = pons()
        .arg("import")
        .arg("openapi")
        .arg(&invalid)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("3.0.3"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn import_prints_usage_for_help() {
    let output = pons()
        .args(["import", "openapi", "--help"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("usage: pons import openapi"),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
}
