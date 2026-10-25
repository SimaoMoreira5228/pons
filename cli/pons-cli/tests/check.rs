mod support;

use support::pons;

#[test]
fn check_accepts_a_valid_contract() {
    let output = pons()
        .args(["check", "../../spec/fixtures/login.json"])
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
