use std::{fs, process::Command};

use pons_emit_ts::{EmitMode, ErrorStrategy, emit_with_strategy};

#[test]
fn generated_client_rejects_missing_fields_and_unknown_enum_variants() {
    let document: pons_ir::Document =
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
    let files = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .unwrap()
        .files;
    let client = files.iter().find(|file| file.path == "client.ts").unwrap();
    assert!(client.content.contains("function validateWire"));

    let Ok(version) = Command::new("bun").arg("--version").output() else {
        return;
    };
    assert!(version.status.success());

    let directory = tempfile::tempdir().unwrap();
    for file in files {
        fs::write(directory.path().join(file.path), file.content).unwrap();
    }

    let harness = r#"
import { createClient } from "./client.ts";

const missingRequest = createClient({
  baseUrl: "http://example.test",
  fetch: async () => {
    throw new Error("request should not be sent");
  },
});
try {
  await missingRequest.auth.login({ body: {} });
  throw new Error("missing request was accepted");
} catch (cause) {
  const text = JSON.stringify(cause);
  if (!text.includes("missing required field email")) throw new Error(text);
}

const missing = createClient({
  baseUrl: "http://example.test",
  fetch: async () => new Response("{}", { status: 200, headers: { "content-type": "application/json" } }),
});
try {
  await missing.auth.login({ body: { email: "person@example.test" } });
  throw new Error("missing response was accepted");
} catch (cause) {
  const text = JSON.stringify(cause);
  if (!text.includes("missing required field id")) throw new Error(text);
}

const unknown = createClient({
  baseUrl: "http://example.test",
  fetch: async () => new Response(JSON.stringify({ code: "unknown" }), { status: 400, headers: { "content-type": "application/json" } }),
});
try {
  await unknown.auth.login({ body: { email: "person@example.test" } });
  throw new Error("unknown enum variant was accepted");
} catch (cause) {
  const text = JSON.stringify(cause);
  if (!text.includes("invalid enum variant unknown")) throw new Error(text);
}
"#;
    let harness_path = directory.path().join("validation.ts");
    fs::write(&harness_path, harness).unwrap();
    let output = Command::new("bun")
        .arg("run")
        .arg(&harness_path)
        .current_dir(directory.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
