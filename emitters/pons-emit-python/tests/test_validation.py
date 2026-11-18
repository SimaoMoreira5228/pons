import importlib
import json
import subprocess
import sys

import pytest

import support


class Generated:
    def __init__(self, directory, document, package, options=None, *, mode="strict"):
        merged = {"package": package}
        if options is not None:
            merged.update(options)
        self.response = support.emit(document, mode=mode, options=merged)
        assert self.response.get("error") is None, self.response.get("error")
        support.write_files(self.response, directory)
        self.package = package
        self.directory = directory
        sys.path.insert(0, str(directory))
        importlib.invalidate_caches()
        self.api = importlib.import_module(package)
        self.errors = importlib.import_module(package + ".errors")

    def close(self):
        sys.path.remove(str(self.directory))
        for name in list(sys.modules):
            if name == self.package or name.startswith(self.package + "."):
                del sys.modules[name]
        importlib.invalidate_caches()


@pytest.fixture
def payloads(tmp_path):
    generated = Generated(
        tmp_path,
        validation_document(),
        "validation_api",
        {"validation": {"backend": "builtin"}},
    )
    yield generated
    generated.close()


def validation_document():
    return {
        "ir_version": "0.3",
        "info": {"title": "Validation", "version": "1"},
        "types": {
            "Meta": {
                "kind": "struct",
                "name": ["meta"],
                "direction": "output",
                "openness": "reject",
                "fields": [
                    {
                        "wire": "email",
                        "ident": ["email"],
                        "ty": {"kind": "string", "format": "email"},
                        "presence": "required",
                    }
                ],
            },
            "Choice": {
                "kind": "enum",
                "name": ["choice"],
                "direction": "output",
                "repr": {"kind": "internal", "tag": "kind"},
                "variants": [
                    {
                        "wire": "none",
                        "ident": ["none"],
                        "payload": {"kind": "unit"},
                    },
                    {
                        "wire": "email",
                        "ident": ["email"],
                        "payload": {
                            "kind": "struct",
                            "fields": [
                                {
                                    "wire": "address",
                                    "ident": ["address"],
                                    "ty": {"kind": "string", "format": "email"},
                                    "presence": "required",
                                }
                            ],
                        },
                    },
                ],
            },
            "Node": {
                "kind": "struct",
                "name": ["node"],
                "direction": "output",
                "openness": "reject",
                "fields": [
                    {
                        "wire": "name",
                        "ident": ["name"],
                        "ty": {"kind": "string", "min_length": 1},
                        "presence": "required",
                    },
                    {
                        "wire": "children",
                        "ident": ["children"],
                        "ty": {"kind": "array", "item": {"ref": "Node"}},
                        "presence": "required",
                    },
                ],
            },
            "Payload": {
                "kind": "struct",
                "name": ["payload"],
                "direction": "output",
                "openness": {"preserve_typed": {"value": {"ref": "Meta"}}},
                "fields": [
                    {
                        "wire": "email",
                        "ident": ["email"],
                        "ty": {"kind": "string", "format": "email"},
                        "presence": "required",
                    },
                    {
                        "wire": "stamp",
                        "ident": ["stamp"],
                        "ty": {"kind": "string", "format": "date-time"},
                        "presence": "required",
                    },
                    {
                        "wire": "code",
                        "ident": ["code"],
                        "ty": {
                            "kind": "string",
                            "min_length": 2,
                            "max_length": 4,
                            "pattern": "^[A-Z]+$",
                        },
                        "presence": "required",
                    },
                    {
                        "wire": "ratio",
                        "ident": ["ratio"],
                        "ty": {
                            "kind": "float",
                            "bits": 64,
                            "minimum": 0.0,
                            "maximum": 10.0,
                            "multiple_of": 0.5,
                        },
                        "presence": "required",
                    },
                    {
                        "wire": "tags",
                        "ident": ["tags"],
                        "ty": {
                            "kind": "array",
                            "item": {"kind": "string"},
                            "unique": True,
                        },
                        "presence": "required",
                    },
                ],
            },
        },
        "auth_schemes": {},
        "common_errors": [],
        "operations": [
            {
                "id": [["payloads"], ["get"]],
                "method": "GET",
                "path": [{"lit": "payloads"}],
                "responses": [
                    {"status": 200, "content": "json", "body": {"ref": "Payload"}}
                ],
                "auth": {"alternatives": [], "optional": False},
                "semantics": {"safe": True, "idempotent": True},
            },
            {
                "id": [["nodes"], ["get"]],
                "method": "GET",
                "path": [{"lit": "nodes"}],
                "responses": [
                    {"status": 200, "content": "json", "body": {"ref": "Node"}}
                ],
                "auth": {"alternatives": [], "optional": False},
                "semantics": {"safe": True, "idempotent": True},
            },
            {
                "id": [["choices"], ["get"]],
                "method": "GET",
                "path": [{"lit": "choices"}],
                "responses": [
                    {"status": 200, "content": "json", "body": {"ref": "Choice"}}
                ],
                "auth": {"alternatives": [], "optional": False},
                "semantics": {"safe": True, "idempotent": True},
            },
        ],
    }


def payload_payload():
    return {
        "email": "ada@example.test",
        "stamp": "2024-02-29T12:34:56Z",
        "code": "AB",
        "ratio": 1.5,
        "tags": ["a", "b"],
        "meta": {"email": "ops@example.test"},
    }


def call(
    api,
    namespace,
    body,
    headers=None,
    method="get",
    credentials=None,
    *arguments,
):
    def transport(request):
        merged = {"Content-Type": "application/json"}
        if headers is not None:
            merged.update(headers)
        return 200, merged, json.dumps(body).encode("utf-8")

    client = api.Client(
        "https://api.test", transport=transport, credentials=credentials
    )
    return getattr(getattr(client, namespace), method)(*arguments)


def compiled(directory):
    result = subprocess.run(
        [sys.executable, "-m", "compileall", "-q", str(directory)],
        capture_output=True,
        text=True,
    )
    assert result.returncode == 0, result.stdout + result.stderr


def test_exact_fixture_validates_in_strict_mode(tmp_path):
    response = support.emit(
        support.load_spec("exact.json"),
        options={"validation": {"backend": "builtin"}},
    )
    assert response.get("error") is None, response.get("error")
    assert response["validation_coverage"] == {
        "exact": response["coverage"]["exact"],
        "lossy": [],
        "unsupported": [],
    }
    assert "pons_client/validators.py" in {file["path"] for file in response["files"]}
    support.write_files(response, tmp_path)
    compiled(tmp_path)


def test_email_and_date_time_are_validated(tmp_path):
    document = support.load_spec("exact.json")
    user = document["types"]["User"]
    user["fields"].append(
        {
            "wire": "contact",
            "ident": ["contact"],
            "ty": {"kind": "string", "format": "email"},
            "presence": "required",
        }
    )
    user["fields"].append(
        {
            "wire": "expires_at",
            "ident": ["expires", "at"],
            "ty": {"kind": "string", "format": "date-time"},
            "presence": "required",
        }
    )
    generated = Generated(
        tmp_path,
        document,
        "exact_format_api",
        {"validation": {"backend": "builtin"}},
    )
    try:
        api = generated.api
        headers = {"X-Trace-ID": "trace-1"}
        valid = {
            "id": "7",
            "contact": "ada@example.test",
            "expires_at": "2024-02-29T12:34:56Z",
        }
        credentials = {"tenant": "t"}
        response = call(
            api,
            "events",
            valid,
            headers,
            "watch",
            credentials,
            "cursor",
        )
        assert response.body.contact == "ada@example.test"

        bad_email = dict(valid, contact="not-an-email")
        with pytest.raises(api.DecodeError) as caught:
            call(
                api,
                "events",
                bad_email,
                headers,
                "watch",
                credentials,
                "cursor",
            )
        assert caught.value.path == "contact"
        assert caught.value.status == 200

        bad_month = dict(valid, expires_at="2024-13-01T00:00:00Z")
        with pytest.raises(api.DecodeError) as caught:
            call(
                api,
                "events",
                bad_month,
                headers,
                "watch",
                credentials,
                "cursor",
            )
        assert caught.value.path == "expires_at"

        bad_offset = dict(valid, expires_at="2024-01-01T00:00:00+24:00")
        with pytest.raises(api.DecodeError) as caught:
            call(
                api,
                "events",
                bad_offset,
                headers,
                "watch",
                credentials,
                "cursor",
            )
        assert caught.value.path == "expires_at"
    finally:
        generated.close()


def test_string_float_array_and_extra_constraints(payloads):
    api = payloads.api
    assert call(api, "payloads", payload_payload()).email == "ada@example.test"

    for field, value in [
        ("email", "bad"),
        ("stamp", "2024-13-01T00:00:00Z"),
        ("stamp", "2024-01-01T00:00:00+24:00"),
        ("code", "ABCDE"),
        ("code", "abcd"),
        ("ratio", -0.5),
        ("ratio", 10.5),
        ("ratio", 1.2),
        ("tags", ["a", "a"]),
    ]:
        body = payload_payload()
        body[field] = value
        with pytest.raises(api.DecodeError) as caught:
            call(api, "payloads", body)
        assert caught.value.status == 200
        assert caught.value.path
        if field == "tags":
            assert caught.value.path == "tags[1]"
        else:
            assert caught.value.path == field

    extra = payload_payload()
    extra["meta"] = {"email": "bad"}
    with pytest.raises(api.DecodeError) as caught:
        call(api, "payloads", extra)
    assert caught.value.path == "meta.email"


def test_boundary_values_pass(payloads):
    api = payloads.api
    for ratio in (0.0, 10.0):
        body = payload_payload()
        body["ratio"] = ratio
        assert call(api, "payloads", body).ratio == ratio

    for code in ("AB", "ABCD"):
        body = payload_payload()
        body["code"] = code
        assert call(api, "payloads", body).code == code


def test_recursive_and_enum_values_validate(payloads):
    api = payloads.api
    node = call(
        api,
        "nodes",
        {"name": "root", "children": [{"name": "child", "children": []}]},
    )
    assert node.children[0].name == "child"

    with pytest.raises(api.DecodeError) as caught:
        call(
            api,
            "nodes",
            {"name": "root", "children": [{"name": "", "children": []}]},
        )
    assert caught.value.path == "children[0].name"

    assert call(api, "choices", {"kind": "none"}).to_wire() == {"kind": "none"}
    assert call(api, "choices", {"kind": "email", "address": "a@b.test"}).to_wire() == {
        "kind": "email",
        "address": "a@b.test",
    }

    with pytest.raises(api.DecodeError) as caught:
        call(api, "choices", {"kind": "email", "address": "bad"})
    assert caught.value.path == "address"


def test_typed_response_headers_are_validated(tmp_path):
    document = support.load_spec("exact.json")
    document["operations"][1]["responses"][0]["headers"][0]["ty"] = {
        "kind": "string",
        "format": "email",
    }
    generated = Generated(
        tmp_path,
        document,
        "header_api",
        {"validation": {"backend": "builtin"}},
    )
    try:
        api = generated.api
        body = {"id": "7"}
        assert call(
            api,
            "events",
            body,
            {"X-Trace-ID": "trace@example.test"},
            "watch",
            {"tenant": "t"},
            "cursor-1",
        ).body.id == 7
        with pytest.raises(api.DecodeError) as caught:
            call(
                api,
                "events",
                body,
                {"X-Trace-ID": "not-an-email"},
                "watch",
                {"tenant": "t"},
                "cursor-1",
            )
        assert caught.value.path == "headers.X-Trace-ID"
        assert caught.value.status == 200
    finally:
        generated.close()


def test_disabled_validation_emits_no_validators(tmp_path):
    response = support.emit(validation_document())
    assert response.get("error") is None, response.get("error")
    assert "validation_coverage" not in response
    paths = {file["path"] for file in response["files"]}
    assert "pons_client/validators.py" not in paths
    client = next(
        file["content"]
        for file in response["files"]
        if file["path"].endswith("client.py")
    )
    assert "validators" not in client

    generated = Generated(tmp_path, validation_document(), "plain_api")
    try:
        api = generated.api
        body = payload_payload()
        body["email"] = "bad"
        body["stamp"] = "not-a-date"
        assert call(api, "payloads", body).email == "bad"
    finally:
        generated.close()


@pytest.mark.parametrize(
    "options, marker",
    [
        ({"validation": {"backend": "zod"}}, "builtin or none"),
        ({"validation": "builtin"}, "string"),
        ({"validation": {"backend": "builtin", "import_from": "x"}}, "import_from"),
    ],
)
def test_invalid_validation_options_are_refused(options, marker):
    response = support.emit(validation_document(), options=options)
    assert response["files"] == []
    assert response["error"]
    assert marker in response["error"]
    assert response["diagnostics"][0]["code"] == "PY-E002"
