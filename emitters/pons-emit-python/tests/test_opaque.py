import json

import pytest

import support


def document():
    return {
        "ir_version": "0.3",
        "info": {"title": "Opaque", "version": "1"},
        "types": {
            "Envelope": {
                "kind": "struct",
                "name": ["envelope"],
                "direction": "output",
                "openness": "reject",
                "fields": [
                    {
                        "wire": "json",
                        "ident": ["json"],
                        "ty": {
                            "kind": "opaque",
                            "codec": "custom-json",
                            "wire": "json",
                        },
                        "presence": "required",
                    },
                    {
                        "wire": "text",
                        "ident": ["text"],
                        "ty": {
                            "kind": "opaque",
                            "codec": "custom-text",
                            "wire": "text",
                        },
                        "presence": "required",
                    },
                    {
                        "wire": "bytes",
                        "ident": ["bytes"],
                        "ty": {
                            "kind": "opaque",
                            "codec": "custom-bytes",
                            "wire": "bytes",
                        },
                        "presence": "required",
                    },
                    {
                        "wire": "unknown",
                        "ident": ["unknown"],
                        "ty": {
                            "kind": "opaque",
                            "codec": "custom-unknown",
                            "wire": "unknown",
                        },
                        "presence": "required",
                    },
                ],
            }
        },
        "auth_schemes": {},
        "common_errors": [],
        "operations": [
            {
                "id": [["envelopes"], ["get"]],
                "method": "GET",
                "path": [{"lit": "envelopes"}],
                "responses": [
                    {"status": 200, "content": "json", "body": {"ref": "Envelope"}}
                ],
                "auth": {"alternatives": [], "optional": False},
                "semantics": {"safe": True, "idempotent": True},
            },
            {
                "id": [["raw"], ["get"]],
                "method": "GET",
                "path": [{"lit": "raw"}],
                "responses": [
                    {
                        "status": 200,
                        "content": "json",
                        "body": {
                            "kind": "opaque",
                            "codec": "custom-raw",
                            "wire": "json",
                        },
                    }
                ],
                "auth": {"alternatives": [], "optional": False},
                "semantics": {"safe": True, "idempotent": True},
            },
        ],
    }


def envelope_payload():
    return {
        "json": {"nested": [1, "two", None]},
        "text": "hello",
        "bytes": "AAEC",
        "unknown": [True, 3, {"x": 1}],
    }


def json_transport(body):
    def transport(request):
        return (
            200,
            {"Content-Type": "application/json"},
            json.dumps(body).encode("utf-8"),
        )

    return transport


@pytest.fixture
def opaque(tmp_path):
    generated = support.Generated(tmp_path, document(), "opaque_api")
    yield generated
    generated.close()


def test_opaque_fields_pass_values_through(opaque):
    payload = envelope_payload()
    envelope = opaque.types.Envelope.from_wire(payload)
    assert envelope.json == payload["json"]
    assert envelope.text == "hello"
    assert envelope.bytes == "AAEC"
    assert envelope.unknown == payload["unknown"]
    assert envelope.to_wire() == payload


def test_opaque_response_bodies_pass_values_through(opaque):
    api = opaque.api
    envelope = api.Client(
        "https://api.test", transport=json_transport(envelope_payload())
    ).envelopes.get()
    assert envelope.json == envelope_payload()["json"]

    raw = {"list": [1, "x", None]}
    assert (
        api.Client("https://api.test", transport=json_transport(raw)).raw.get()
        == raw
    )


def test_opaque_coverage_is_reported(opaque):
    assert opaque.response["coverage"] == {
        "exact": [],
        "opaque": [
            "operations.envelopes.get",
            "operations.raw.get",
            "types.Envelope",
        ],
        "lossy": [],
        "unsupported": [],
    }


def test_strict_emit_accepts_opaque(opaque):
    assert opaque.response.get("error") is None
    assert "opaque_api/types.py" in {
        file["path"] for file in opaque.response["files"]
    }


def test_opaque_validation_is_exact(tmp_path):
    generated = support.Generated(
        tmp_path,
        document(),
        "opaque_validation_api",
        options={"validation": {"backend": "builtin"}},
    )
    try:
        assert generated.response["validation_coverage"] == {
            "exact": [
                "operations.envelopes.get",
                "operations.raw.get",
                "types.Envelope",
            ],
            "lossy": [],
            "unsupported": [],
        }
        envelope = generated.api.Client(
            "https://api.test", transport=json_transport(envelope_payload())
        ).envelopes.get()
        assert envelope.json == envelope_payload()["json"]
    finally:
        generated.close()


def test_any_is_imported_only_when_opaque_is_used():
    plain = support.emit(support.base_document())
    plain_types = next(
        file["content"]
        for file in plain["files"]
        if file["path"].endswith("types.py")
    )
    assert "from typing import Any" not in plain_types

    with_opaque = support.emit(document())
    opaque_types = next(
        file["content"]
        for file in with_opaque["files"]
        if file["path"].endswith("types.py")
    )
    assert "from typing import Any" in opaque_types
