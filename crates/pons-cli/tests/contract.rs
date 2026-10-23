use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::{Command, Output};
use std::thread;

use serde_json::{Value, json};

fn fixture() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/exact.json")
}

fn respond(stream: &mut TcpStream, status: &str, body: &str) {
    respond_with(
        stream,
        status,
        &[("content-type", "application/json")],
        body,
    );
}

fn respond_with(stream: &mut TcpStream, status: &str, headers: &[(&str, &str)], body: &str) {
    let mut response = format!(
        "HTTP/1.1 {status}\r\ncontent-length: {}\r\nconnection: close\r\n",
        body.len()
    );
    for (name, value) in headers {
        response.push_str(&format!("{name}: {value}\r\n"));
    }
    response.push_str("\r\n");
    response.push_str(body);
    stream.write_all(response.as_bytes()).unwrap();
}

fn receive(stream: &mut TcpStream) -> String {
    let mut request = Vec::new();
    let mut buffer = [0u8; 4096];
    loop {
        let read = stream.read(&mut buffer).unwrap_or(0);
        if read == 0 {
            break;
        }
        request.extend_from_slice(&buffer[..read]);
        let text = String::from_utf8_lossy(&request);
        if let Some(index) = text.find("\r\n\r\n") {
            let headers = text[..index].to_ascii_lowercase();
            let length = headers
                .lines()
                .find_map(|line| line.strip_prefix("content-length:"))
                .and_then(|value| value.trim().parse::<usize>().ok())
                .unwrap_or(0);
            if request.len() >= index + 4 + length {
                break;
            }
        }
    }
    String::from_utf8_lossy(&request).into_owned()
}

fn headers(request: &str) -> BTreeMap<String, String> {
    request
        .lines()
        .skip(1)
        .take_while(|line| !line.is_empty())
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_owned()))
        .collect()
}

fn serve(listener: TcpListener, valid: bool, requests: usize) {
    thread::spawn(move || {
        for _ in 0..requests {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let body = receive(&mut stream);
            let accepted = valid && body.contains('@') && body.contains("\"email\"");
            if accepted {
                respond(&mut stream, "200 OK", r#"{"id":"1"}"#);
            } else if valid {
                respond(&mut stream, "400 Bad Request", r#"{"code":"invalid"}"#);
            } else {
                respond(
                    &mut stream,
                    "500 Internal Server Error",
                    r#"{"message":"broken"}"#,
                );
            }
        }
    });
}

fn base(listener: &TcpListener) -> String {
    format!("http://{}", listener.local_addr().unwrap())
}

fn run_test(document: &Value, base: &str, extra: &[&str]) -> Output {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("api.json");
    fs::write(&path, serde_json::to_vec(document).unwrap()).unwrap();
    Command::new(env!("CARGO_BIN_EXE_pons"))
        .arg("test")
        .arg(&path)
        .arg("--base-url")
        .arg(base)
        .args(extra)
        .output()
        .unwrap()
}

fn passed(output: &Output) -> String {
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn a_matching_server_passes_and_its_invalid_request_is_checked() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    serve(listener.try_clone().unwrap(), true, 2);
    let output = Command::new(env!("CARGO_BIN_EXE_pons"))
        .args([
            "test",
            fixture().to_str().unwrap(),
            "--base-url",
            &base(&listener),
        ])
        .output()
        .unwrap();
    let stdout = passed(&output);
    assert!(stdout.contains("1 passed"), "{stdout}");
}

#[test]
fn a_contract_mismatch_exits_five() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    serve(listener.try_clone().unwrap(), false, 1);
    let output = Command::new(env!("CARGO_BIN_EXE_pons"))
        .args([
            "test",
            fixture().to_str().unwrap(),
            "--base-url",
            &base(&listener),
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(5));
}

fn secured() -> Value {
    json!({
        "ir_version": "0.2",
        "info": {"title": "Auth", "version": "1"},
        "auth_schemes": {
            "session": {"kind": "bearer"},
            "tenant": {"kind": "api_key", "in": "header", "name": "X-Tenant"}
        },
        "operations": [
            {
                "id": [["bearer"]],
                "method": "GET",
                "path": [{"lit": "bearer"}],
                "auth": {"alternatives": [["session"]], "optional": false},
                "semantics": {"safe": true, "idempotent": true},
                "responses": [{"status": 204, "content": "empty"}]
            },
            {
                "id": [["tenant"]],
                "method": "GET",
                "path": [{"lit": "tenant"}],
                "auth": {"alternatives": [["tenant"]], "optional": false},
                "semantics": {"safe": true, "idempotent": true},
                "responses": [{"status": 204, "content": "empty"}]
            }
        ]
    })
}

#[test]
fn supplied_credentials_are_attached_by_scheme() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = base(&listener);
    thread::spawn(move || {
        for _ in 0..2 {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let request = receive(&mut stream);
            let headers = headers(&request);
            let authorized = if request.starts_with("GET /bearer ") {
                headers.get("authorization").map(String::as_str) == Some("Bearer secret")
            } else {
                headers.get("x-tenant").map(String::as_str) == Some("acme")
            };
            respond_with(
                &mut stream,
                if authorized {
                    "204 No Content"
                } else {
                    "401 Unauthorized"
                },
                &[],
                "",
            );
        }
    });

    let output = run_test(
        &secured(),
        &address,
        &["--bearer", "secret", "--api-key", "tenant=acme"],
    );

    let stdout = passed(&output);
    assert!(stdout.contains("2 passed"), "{stdout}");
}

#[test]
fn a_missing_credential_skips_the_operation() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let output = run_test(&secured(), &base(&listener), &[]);

    let stdout = passed(&output);
    assert!(stdout.contains("0 passed, 0 failed, 2 skipped"), "{stdout}");
    assert!(stdout.contains("--bearer"), "{stdout}");
}

#[test]
fn a_provided_credential_is_reused_later_in_the_run() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = base(&listener);
    thread::spawn(move || {
        let Ok((mut stream, _)) = listener.accept() else {
            return;
        };
        let request = receive(&mut stream);
        assert!(request.starts_with("POST /login "), "{request}");
        respond(&mut stream, "200 OK", r#"{"token":"secret"}"#);

        let Ok((mut stream, _)) = listener.accept() else {
            return;
        };
        let request = receive(&mut stream);
        let headers = headers(&request);
        let authorized = headers.get("authorization").map(String::as_str) == Some("Bearer secret");
        respond_with(
            &mut stream,
            if authorized {
                "200 OK"
            } else {
                "401 Unauthorized"
            },
            &[("content-type", "application/json")],
            if authorized {
                r#"{"ok":true}"#
            } else {
                r#"{"ok":false}"#
            },
        );
    });

    let document = json!({
        "ir_version": "0.2",
        "info": {"title": "Login", "version": "1"},
        "types": {
            "Session": {
                "kind": "struct",
                "openness": "reject",
                "name": ["session"],
                "direction": "output",
                "fields": [{
                    "wire": "token",
                    "ident": ["token"],
                    "ty": {"kind": "string"},
                    "presence": "required"
                }]
            },
            "Ok": {
                "kind": "struct",
                "openness": "reject",
                "name": ["ok"],
                "direction": "output",
                "fields": [{
                    "wire": "ok",
                    "ident": ["ok"],
                    "ty": {"kind": "bool"},
                    "presence": "required"
                }]
            }
        },
        "auth_schemes": {"session": {"kind": "bearer"}},
        "operations": [
            {
                "id": [["auth"], ["login"]],
                "method": "POST",
                "path": [{"lit": "login"}],
                "responses": [{"status": 200, "content": "json", "body": {"ref": "Session"}}],
                "auth": {"alternatives": [], "optional": false},
                "provides": {"scheme": "session", "value": {"from": "body", "pointer": "/token"}},
                "semantics": {"safe": false, "idempotent": false}
            },
            {
                "id": [["private"]],
                "method": "GET",
                "path": [{"lit": "private"}],
                "auth": {"alternatives": [["session"]], "optional": false},
                "semantics": {"safe": true, "idempotent": true},
                "responses": [{"status": 200, "content": "json", "body": {"ref": "Ok"}}]
            }
        ]
    });

    let output = run_test(&document, &address, &[]);

    let stdout = passed(&output);
    assert!(stdout.contains("2 passed"), "{stdout}");
}
