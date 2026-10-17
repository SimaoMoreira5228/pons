use std::fs;
use std::path::Path;
use std::process::Command;

use pons_emit_ts::{
    ErrorStrategy, Runtime, TsOptions, ValidationBackend, ValidationOptions, emit_with_options,
};
use pons_ir::Document;

fn run_websocket_harness() {
    let document: Document =
        serde_json::from_str(include_str!("../../../fixtures/unsupported.json")).unwrap();
    let output = emit_with_options(
        &document,
        TsOptions {
            runtime: Runtime::Vendor,
            validation: ValidationOptions::new(ValidationBackend::Builtin),
            errors: ErrorStrategy::Throw,
            ..TsOptions::default()
        },
    )
    .expect("the websocket fixture emits");

    let directory = tempfile::tempdir().unwrap();
    for file in output.files {
        let destination = directory.path().join(&file.path);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(destination, &file.content).unwrap();
    }
    install_runtime_package(directory.path());

    let harness = directory.path().join("websocket.ts");
    fs::write(&harness, HARNESS).unwrap();
    let output = Command::new("bun")
        .arg("run")
        .arg(&harness)
        .current_dir(directory.path())
        .output()
        .expect("Bun is required for the websocket runtime test");
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn install_runtime_package(directory: &Path) {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../runtime/ts");
    let installed = directory.join("runtime");
    fs::create_dir_all(&installed).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        if name.to_string_lossy().ends_with(".ts") || name == "package.json" {
            fs::copy(entry.path(), installed.join(&name)).unwrap();
        }
    }
    let scope = directory.join("node_modules").join("@pons");
    fs::create_dir_all(&scope).unwrap();
    link(&installed, &scope.join("runtime"));
}

fn link(source: &Path, destination: &Path) {
    #[cfg(unix)]
    std::os::unix::fs::symlink(source, destination).unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_dir(source, destination).unwrap();
}

const HARNESS: &str = r#"
import { strict as assert } from "node:assert";
import { createClient } from "./index.js";

class FakeSocket {
  static last: FakeSocket;
  readyState = 0;
  listeners = new Map<string, Array<(event: any) => void>>();
  sent: string[] = [];
  url: string;
  headers: Record<string, string>;

  constructor(url: string, _protocols?: string | string[], options?: { headers?: Record<string, string> }) {
    this.url = url;
    this.headers = options?.headers ?? {};
    FakeSocket.last = this;
    queueMicrotask(() => {
      this.readyState = 1;
      this.emit("open", {});
    });
  }

  addEventListener(type: string, listener: (event: any) => void) {
    const list = this.listeners.get(type) ?? [];
    list.push(listener);
    this.listeners.set(type, list);
  }

  removeEventListener(type: string, listener: (event: any) => void) {
    this.listeners.set(type, (this.listeners.get(type) ?? []).filter((item) => item !== listener));
  }

  send(data: string) {
    this.sent.push(data);
  }

  close(code = 1000, reason = "") {
    this.readyState = 3;
    this.emit("close", { code, reason });
  }

  emit(type: string, event: any) {
    for (const listener of this.listeners.get(type) ?? []) listener(event);
  }
}

const client = createClient({
  baseUrl: "http://api.test/v1",
  auth: { tenant: "secret" },
  webSocket: FakeSocket,
});
const connection = await client.events.watch({ path: { cursor: "abc" }, query: { limit: 5 } });
const socket = FakeSocket.last;
assert.equal(socket.url, "ws://api.test/v1/events/abc?limit=5");
assert.equal(socket.headers["x-tenant"], "secret");
assert.ok(connection.send({ email: "user@example.test" }).isOk());
assert.deepEqual(JSON.parse(socket.sent[0]), { email: "user@example.test" });

const messages = connection.messages[Symbol.asyncIterator]();
socket.emit("message", { data: JSON.stringify({ id: "1" }) });
const first = await messages.next();
assert.ok(first.value.isOk());
assert.equal((first.value as any).value.id, "1");

socket.emit("message", { data: JSON.stringify({ id: 1 }) });
const second = await messages.next();
assert.ok(second.value.isErr());
assert.equal((second.value as any).error.kind, "invalid_message");

connection.close();
assert.equal((await connection.closed).code, 1000);
console.log("websocket ok");
"#;

#[test]
fn the_generated_websocket_client_sends_receives_validates_and_closes() {
    run_websocket_harness();
}
