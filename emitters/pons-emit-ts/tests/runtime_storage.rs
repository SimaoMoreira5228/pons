use std::{fs, process::Command};

use pons_emit_ts::{EmitMode, ErrorStrategy, emit_with_strategy};
use pons_ir::Document;

#[test]
fn the_storage_adapters_persist_get_clear_and_refuse_a_broken_record() {
    let document: Document =
        serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap();
    let output = emit_with_strategy(&document, EmitMode::Strict, ErrorStrategy::Throw)
        .expect("the login fixture emits");
    let directory = tempfile::tempdir().unwrap();
    for file in output.files {
        let destination = directory.path().join(&file.path);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(destination, &file.content).unwrap();
    }
    let harness = directory.path().join("storage.ts");
    fs::write(&harness, HARNESS).unwrap();
    let result = Command::new("bun")
        .arg("run")
        .arg(&harness)
        .current_dir(directory.path())
        .output()
        .expect("Bun is required for the storage runtime test");
    assert!(
        result.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}

const HARNESS: &str = r#"
import { strict as assert } from "node:assert";
import { localStorageTokenStore, sessionStorageTokenStore } from "./runtime/credentials.js";

class MemoryStorage {
  items = new Map<string, string>();
  getItem(key: string) {
    return this.items.get(key) ?? null;
  }
  setItem(key: string, value: string) {
    this.items.set(key, value);
  }
  removeItem(key: string) {
    this.items.delete(key);
  }
}

const storage = new MemoryStorage();
const store = localStorageTokenStore({ storage, key: "session" });
assert.equal(store.get(), undefined);

store.set("token-1", { expiresAt: 1_700_000_000_000, path: "/api", secure: true, host: "api.test", origin: "https://api.test" });
assert.deepEqual(store.get(), {
  value: "token-1",
  expiresAt: 1_700_000_000_000,
  path: "/api",
  secure: true,
  host: "api.test",
  origin: "https://api.test",
});
store.set("token-2");
assert.deepEqual(store.get(), { value: "token-2" });
assert.deepEqual(JSON.parse(storage.getItem("session") ?? "null"), { value: "token-2" });

store.clear();
assert.equal(store.get(), undefined);
assert.equal(storage.getItem("session"), null);

const defaults = localStorageTokenStore({ storage });
defaults.set("default-key");
assert.equal(storage.getItem("pons.token"), "{\"value\":\"default-key\"}");

const session = sessionStorageTokenStore({ storage: new MemoryStorage(), key: "other" });
session.set("elsewhere");
assert.deepEqual(session.get(), { value: "elsewhere" });

storage.setItem("session", "{");
assert.throws(() => store.get(), /not valid JSON/);
storage.setItem("session", "\"bare\"");
assert.throws(() => store.get(), /credential record, not a string/);
storage.setItem("session", "{\"value\":7}");
assert.throws(() => store.get(), /string value/);
storage.setItem("session", "{\"value\":\"a\",\"expiresAt\":\"soon\"}");
assert.throws(() => store.get(), /expiresAt must be a number/);
storage.removeItem("session");

Object.defineProperty(globalThis, "localStorage", { value: undefined, configurable: true });
Object.defineProperty(globalThis, "sessionStorage", { value: undefined, configurable: true });
assert.throws(() => localStorageTokenStore(), /localStorage is not available/);
assert.throws(() => sessionStorageTokenStore(), /sessionStorage is not available/);

console.log("storage ok");
"#;
