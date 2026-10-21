use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::Command;
use std::thread;

fn fixture() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/exact.json")
}

fn respond(stream: &mut TcpStream, status: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).unwrap();
}

fn serve(listener: TcpListener, valid: bool, requests: usize) {
    thread::spawn(move || {
        for _ in 0..requests {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
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
            let body = String::from_utf8_lossy(&request);
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
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "stdout: {stdout}\nstderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
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
