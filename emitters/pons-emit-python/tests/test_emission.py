import json
import subprocess
import sys

import support

EXPECTED_FILES = {
    "pons_client/__init__.py",
    "pons_client/types.py",
    "pons_client/errors.py",
    "pons_client/client.py",
    "pons_client/_wire.py",
}


def emit_package(document, *, mode="strict", options=None):
    response = support.emit(document, mode=mode, options=options)
    assert response.get("error") is None, response.get("error")
    return response


def compiled(directory):
    result = subprocess.run(
        [sys.executable, "-m", "compileall", "-q", str(directory)],
        capture_output=True,
        text=True,
    )
    assert result.returncode == 0, result.stdout + result.stderr


def test_exact_fixture_emits(tmp_path):
    response = emit_package(support.load_spec("exact.json"), mode="lossy")
    assert response["coverage"] == {
        "exact": [
            "operations.auth.login",
            "operations.events.watch",
            "types.EventQuery",
            "types.LoginError",
            "types.LoginRequest",
            "types.User",
        ],
        "opaque": [],
        "lossy": [],
        "unsupported": [],
    }
    files = support.write_files(response, tmp_path)
    assert {file.relative_to(tmp_path).as_posix() for file in files} == EXPECTED_FILES
    compiled(tmp_path)
    types = (tmp_path / "pons_client" / "types.py").read_text()
    assert "class LoginErrorInvalid:" in types
    assert "class LoginErrorLocked:" in types
    assert "LoginError = LoginErrorInvalid | LoginErrorLocked" in types
    assert "def decode_LoginError(" in types
    assert "nickname: str | None | _wire.Missing = _wire.MISSING" in types
    assert "_wire.MISSING if raw is _wire.MISSING" in types
    assert "if self.nickname is not _wire.MISSING:" in types


def test_paginated_fixture_emits(tmp_path):
    document = support.load_spec("paginated.json")
    response = emit_package(document)
    assert response["coverage"] == {
        "exact": [
            "operations.users.list",
            "types.Page",
            "types.PageQuery",
            "types.User",
        ],
        "opaque": [],
        "lossy": [],
        "unsupported": [],
    }
    files = support.write_files(response, tmp_path)
    assert {file.relative_to(tmp_path).as_posix() for file in files} == EXPECTED_FILES
    compiled(tmp_path)
    client = (tmp_path / "pons_client" / "client.py").read_text()
    assert "def list_all(" in client
    assert "Iterator[_types.User]" in client
    assert "dataclasses.replace" in client


def test_comprehensive_fixture_emits(tmp_path):
    document = support.load("client.json")
    response = emit_package(document)
    assert response["coverage"]["unsupported"] == []
    assert response["coverage"]["exact"] == sorted(support.subjects(document))
    files = support.write_files(response, tmp_path)
    assert {file.relative_to(tmp_path).as_posix() for file in files} == EXPECTED_FILES
    compiled(tmp_path)
    client = (tmp_path / "pons_client" / "client.py").read_text()
    assert "_types.Profile.from_wire" in client
    assert "_types.Session.from_wire" in client
    assert "_types.ChoiceCount.from_wire" not in client
    assert "_types.FailureLocked.from_wire" in client
    types = (tmp_path / "pons_client" / "types.py").read_text()
    for name in (
        "class ChoiceNone",
        "class ChoiceCount",
        "class ChoicePair",
        "class ChoiceRecord",
        "class MarkedEmpty",
        "class MarkedMeta",
        "class MarkedPayload",
        "class BesideOff",
        "class BesideText",
        "class BesidePair",
        "class BesideRecord",
        "def decode_Beside(",
        "def decode_Marked(",
        "def decode_Choice(",
    ):
        assert name in types


def test_generated_files_carry_the_marker_and_stay_in_the_package(tmp_path):
    response = emit_package(support.load("client.json"), options={"package": "acme"})
    for file in response["files"]:
        assert file["encoding"] == "utf8"
        assert file["path"].startswith("acme/")
        relative = file["path"].split("/")
        assert not file["path"].startswith("/")
        assert ".." not in relative
        assert file["content"].startswith(support.MARKER)
    assert {file["path"] for file in response["files"]} == {
        "acme/__init__.py",
        "acme/types.py",
        "acme/errors.py",
        "acme/client.py",
        "acme/_wire.py",
    }
    support.write_files(response, tmp_path)
    compiled(tmp_path)


def test_package_option_changes_the_import_name(tmp_path):
    response = emit_package(support.load("client.json"), options={"package": "acme"})
    support.write_files(response, tmp_path)
    init = (tmp_path / "acme" / "__init__.py").read_text()
    assert "from .client import Client, Request, Response" in init
    assert "'ChoiceCount'" in init
    assert "'FailureLocked'" in init


def test_login_fixture_is_refused_for_streams():
    response = support.emit(support.load_spec("login.json"))
    assert response["files"] == []
    assert response["error"]
    messages = " ".join(item["message"] for item in response["diagnostics"])
    assert "streams" in messages
    assert "closed objects" not in messages
    assert set(support.coverage_subjects(response)) == support.subjects(
        support.load_spec("login.json")
    )


def test_failure_reports_every_subject_once():
    document = support.load("client.json")
    document["types"]["Profile"]["openness"] = "preserve_opaque"
    document["operations"][0]["stream"] = {
        "kind": "sse",
        "events": [{"event": "thing", "data": {"ref": "Session"}}],
    }
    response = support.emit(document)
    subjects = support.coverage_subjects(response)
    assert response["files"] == []
    assert set(subjects) == support.subjects(document)
    assert len(subjects) == len(set(subjects))


def test_empty_and_one_variant_enums_emit(tmp_path):
    document = support.base_document()
    document["types"]["Only"] = {
        "kind": "enum",
        "name": ["only"],
        "direction": "output",
        "repr": {"kind": "external"},
        "variants": [{"wire": "yes", "ident": ["yes"], "payload": {"kind": "unit"}}],
    }
    document["types"]["Thing"]["fields"].append(
        {
            "wire": "only",
            "ident": ["only"],
            "ty": {"ref": "Only"},
            "presence": "required",
        }
    )
    response = emit_package(document)
    support.write_files(response, tmp_path)
    compiled(tmp_path)
    types = (tmp_path / "pons_client" / "types.py").read_text()
    assert "Only = OnlyYes" in types
    assert "def decode_Only(" in types


def test_invalid_openness_is_still_refused():
    document = support.base_document()
    document["types"]["Thing"]["openness"] = "allow"
    response = support.emit(document)
    assert response["files"] == []
    assert "invalid object openness" in response["error"]


def test_untagged_is_still_refused():
    document = support.base_document()
    document["types"]["Broken"] = {
        "kind": "enum",
        "name": ["broken"],
        "direction": "output",
        "repr": {"kind": "untagged"},
        "variants": [{"wire": "a", "ident": ["a"], "payload": {"kind": "unit"}}],
    }
    response = support.emit(document)
    assert response["files"] == []
    assert "untagged" in response["error"]
