import json
import subprocess
import sys
from pathlib import Path

import support

EXECUTABLE = support.ROOT / "pons-emit-python"


def run(command, payload=None):
    return subprocess.run(
        [sys.executable, str(EXECUTABLE), command],
        input=None if payload is None else json.dumps(payload),
        capture_output=True,
        text=True,
    )


def test_describe_prints_the_manifest():
    result = run("describe")
    assert result.returncode == 0, result.stderr
    manifest = json.loads(result.stdout)
    assert manifest["name"] == "python"
    assert manifest["protocol"] == {"min": "3.0", "max": "3.0"}
    assert manifest["ir_version"] == {"min": "0.3", "max": "0.3"}
    assert manifest["option_schema"]["options"]["package"]["type"] == "string"


def test_emit_reads_a_request_from_stdin():
    document = support.load_spec("exact.json")
    payload = support.request(document, mode="lossy")
    result = run("emit", payload)
    assert result.returncode == 0, result.stderr
    response = json.loads(result.stdout)
    assert response.get("error") is None, response.get("error")
    assert {file["path"] for file in response["files"]} == {
        "pons_client/__init__.py",
        "pons_client/types.py",
        "pons_client/errors.py",
        "pons_client/client.py",
        "pons_client/_wire.py",
    }


def test_emit_returns_an_error_response_for_an_unsupported_document():
    payload = support.request(support.load_spec("login.json"), mode="lossy")
    result = run("emit", payload)
    assert result.returncode == 0, result.stderr
    response = json.loads(result.stdout)
    assert response["files"] == []
    assert response["error"]
    assert all(item["severity"] == "error" for item in response["diagnostics"])


def test_an_unknown_command_is_a_usage_error():
    result = run("explain")
    assert result.returncode == 2
    assert "describe or emit" in result.stderr
