from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

import jsonschema
import pytest
from fastapi import FastAPI
from pydantic import BaseModel

from pons_fastapi import (
    PonsApp,
    PonsConfigurationError,
    PonsError,
    PonsPagination,
    build_document,
    pons_operation,
)

PACKAGE_ROOT = Path(__file__).resolve().parents[1]
TESTS_ROOT = Path(__file__).resolve().parent
REPO_ROOT = PACKAGE_ROOT.parents[1]
SCHEMA = REPO_ROOT / "spec" / "ir.schema.json"


def environment() -> dict[str, str]:
    env = os.environ.copy()
    parts = [str(PACKAGE_ROOT), str(TESTS_ROOT)]
    if env.get("PYTHONPATH"):
        parts.append(env["PYTHONPATH"])
    env["PYTHONPATH"] = os.pathsep.join(parts)
    return env


def pons_command() -> list[str]:
    if os.environ.get("PONS_CLI"):
        return [os.environ["PONS_CLI"]]
    found = shutil.which("pons")
    if found:
        return [found]
    return ["cargo", "run", "--quiet", "-p", "pons-cli", "--"]


def run_cli(destination: Path) -> dict:
    completed = subprocess.run(
        [sys.executable, "-m", "pons_fastapi", "sample_app:app", "--pretty"],
        cwd=PACKAGE_ROOT,
        env=environment(),
        capture_output=True,
        text=True,
        check=False,
    )
    assert completed.returncode == 0, completed.stderr
    destination.write_text(completed.stdout)
    return json.loads(completed.stdout)


def test_cli_output_validates_against_the_schema_and_pons_check(tmp_path: Path) -> None:
    document = run_cli(tmp_path / "ir.json")
    schema = json.loads(SCHEMA.read_text())
    jsonschema.Draft202012Validator(schema).validate(document)

    completed = subprocess.run(
        pons_command() + ["check", str(tmp_path / "ir.json")],
        cwd=REPO_ROOT,
        capture_output=True,
        text=True,
        check=False,
        timeout=600,
    )
    assert completed.returncode == 0, completed.stderr


def test_sample_operations_cover_the_declared_features() -> None:
    from sample_app import app

    document = build_document(app)
    operations = {
        ".".join(".".join(name) for name in operation["id"]): operation
        for operation in document["operations"]
    }

    login = operations["auth.login"]
    assert login["body"]["ty"]["ref"].endswith("LoginRequest")
    assert login["responses"][0]["body"]["ref"].endswith("Session")
    assert login["errors"][0]["status"] == 401
    assert login["errors"][0]["body"]["ref"].endswith("LoginError")
    assert login["provides"]["scheme"] == "session"
    assert login["provides"]["value"] == {"from": "body", "pointer": "/token"}

    read = operations["users.get"]
    assert read["path"][1]["param"] == "user_id"
    assert read["path"][1]["ty"] == {"kind": "string", "format": "uuid"}
    assert read["headers"][0]["wire"] == "x-request-id"
    assert read["auth"]["alternatives"] == [["session"]]
    assert read["auth"]["scopes"] == {"session": ["profile:read"]}
    assert read["query"]["ref"].endswith("::query")

    update = operations["users.update"]
    assert update["body"]["ty"]["ref"].endswith("UpdateUser#input")
    assert update["responses"][0]["body"]["ref"].endswith("UpdateUser#output")

    listing = operations["users.list"]
    assert listing["semantics"]["pagination"] == {
        "request": {"kind": "cursor", "field": "cursor"},
        "items": "/users",
        "next": "/next_cursor",
        "page_size": {"field": "limit", "default": 20, "maximum": 100},
    }
    assert "pagination" not in read["semantics"]

    types = document["types"]
    role = types["sample_app.Role"]
    assert role["kind"] == "enum"
    assert role["repr"] == {"kind": "string"}
    assert role["direction"] == "both"

    user = types["sample_app.User"]
    nickname = next(field for field in user["fields"] if field["wire"] == "nickname")
    assert nickname["presence"] == "required"
    assert nickname["ty"]["kind"] == "nullable"

    chat = operations["chat.stream"]
    assert chat["responses"] == [{"status": 200, "content": "stream", "media": None}]
    assert chat["stream"] == {
        "kind": "sse",
        "events": [
            {"event": "done", "data": {"ref": "sample_app.StreamDone"}},
            {"event": "token", "data": {"ref": "sample_app.TokenEvent"}},
        ],
    }
    assert types["sample_app.StreamDone"]["kind"] == "struct"
    assert types["sample_app.TokenEvent"]["kind"] == "struct"

    upload = operations["files.upload"]
    assert upload["body"]["content"] == "multipart"
    assert upload["body"]["ty"] == {
        "kind": "opaque",
        "codec": "fastapi.multipart",
        "wire": "unknown",
    }
    assert upload["body"]["media"] == {"type": "multipart", "subtype": "form-data"}

    feedback = operations["feedback.create"]
    assert feedback["body"]["content"] == "form"
    assert feedback["body"]["media"] == {"type": "application", "subtype": "x-www-form-urlencoded"}
    form = types[feedback["body"]["ty"]["ref"]]
    form_fields = {field["wire"]: field for field in form["fields"]}
    assert form_fields["name"]["presence"] == "required"
    assert form_fields["rating"]["presence"] == "optional"
    assert form_fields["rating"]["default"] == 1

    ingest = operations["ingest.raw"]
    assert ingest["body"]["content"] == "bytes"
    assert ingest["body"]["media"] == {"type": "application", "subtype": "octet-stream"}
    assert ingest["body"]["ty"] == {
        "kind": "opaque",
        "codec": "fastapi.request",
        "wire": "bytes",
    }

    report = operations["reports.csv"]
    assert report["responses"] == [
        {
            "status": 200,
            "content": "text",
            "media": {"type": "text", "subtype": "csv"},
            "body": {"kind": "string"},
        }
    ]

    invoice = types["sample_app.Invoice"]
    invoice_fields = {field["wire"]: field for field in invoice["fields"]}
    assert invoice_fields["total"]["ty"] == {"kind": "decimal", "wire": "string"}
    assert invoice_fields["reference"]["ty"] == {
        "kind": "string",
        "format": "uuid",
    }


def test_unsupported_route_is_skipped_with_a_diagnostic() -> None:
    from sample_app import app

    document = build_document(app)
    legacy = [
        entry
        for entry in document["diagnostics"]
        if entry["code"] == "fastapi.unsupported_route" and "/legacy" in entry["message"]
    ]
    assert legacy, document["diagnostics"]
    assert all(
        ".".join(".".join(name) for name in operation["id"]) != "legacy"
        for operation in document["operations"]
    )


def test_pagination_declaration_rejects_an_unknown_kind() -> None:
    with pytest.raises(PonsConfigurationError, match="'page'"):
        pons_operation(
            pagination=PonsPagination(kind="page", field="cursor", items="/items")
        )


def test_common_errors_only_appear_when_declared() -> None:
    class Body(BaseModel):
        value: int

    app = FastAPI(title="Plain", version="1")
    app.get("/value")(lambda: Body(value=1))

    assert build_document(app)["common_errors"] == []

    app.state.pons = PonsApp(
        common_errors=(PonsError(status=429, name="rate_limited", description="Slow down."),)
    )
    common = build_document(app)["common_errors"]
    assert common == [
        {
            "status": 429,
            "name": ["rate", "limited"],
            "content": "empty",
            "media": None,
            "doc": {"description": "Slow down."},
        }
    ]
