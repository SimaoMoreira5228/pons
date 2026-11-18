import importlib
import json
import sys

import pytest

import support


class Generated:
    def __init__(self, directory, document, package):
        self.response = support.emit(document, options={"package": package})
        assert self.response.get("error") is None, self.response.get("error")
        support.write_files(self.response, directory)
        self.package = package
        self.directory = directory
        sys.path.insert(0, str(directory))
        importlib.invalidate_caches()
        self.api = importlib.import_module(package)

    def close(self):
        sys.path.remove(str(self.directory))
        for name in list(sys.modules):
            if name == self.package or name.startswith(self.package + "."):
                del sys.modules[name]
        importlib.invalidate_caches()


def bodies_document():
    return {
        "ir_version": "0.3",
        "info": {"title": "Bodies", "version": "1"},
        "types": {
            "FormBody": {
                "kind": "struct",
                "name": ["form", "body"],
                "direction": "input",
                "openness": "reject",
                "fields": [
                    {
                        "wire": "name",
                        "ident": ["name"],
                        "ty": {"kind": "string"},
                        "presence": "required",
                    },
                    {
                        "wire": "count",
                        "ident": ["count"],
                        "ty": {"kind": "int", "bits": 32, "signed": False, "wire": "number"},
                        "presence": "optional",
                    },
                ],
            },
            "FileBody": {
                "kind": "struct",
                "name": ["file", "body"],
                "direction": "input",
                "openness": "reject",
                "fields": [
                    {
                        "wire": "name",
                        "ident": ["name"],
                        "ty": {"kind": "string"},
                        "presence": "required",
                    },
                    {
                        "wire": "file",
                        "ident": ["file"],
                        "ty": {"kind": "bytes", "encoding": "base64"},
                        "presence": "required",
                    },
                ],
            }
        },
        "auth_schemes": {},
        "common_errors": [],
        "operations": [
            {
                "id": [["bodies"], ["text"]],
                "method": "POST",
                "path": [{"lit": "bodies"}, {"lit": "text"}],
                "body": {
                    "ty": {"kind": "string"},
                    "content": "text",
                    "required": True,
                    "media": {"type": "text", "subtype": "plain"},
                },
                "responses": [
                    {"status": 200, "content": "json", "body": {"kind": "string"}}
                ],
                "auth": {"alternatives": [], "optional": False},
                "semantics": {"safe": False, "idempotent": False},
            },
            {
                "id": [["bodies"], ["bytes"]],
                "method": "POST",
                "path": [{"lit": "bodies"}, {"lit": "bytes"}],
                "body": {
                    "ty": {"kind": "bytes", "encoding": "base64"},
                    "content": "bytes",
                    "required": True,
                    "media": {"type": "application", "subtype": "octet-stream"},
                },
                "responses": [
                    {"status": 200, "content": "json", "body": {"kind": "string"}}
                ],
                "auth": {"alternatives": [], "optional": False},
                "semantics": {"safe": False, "idempotent": False},
            },
            {
                "id": [["bodies"], ["multipart"]],
                "method": "POST",
                "path": [{"lit": "bodies"}, {"lit": "multipart"}],
                "body": {
                    "ty": {"ref": "FileBody"},
                    "content": "multipart",
                    "required": True,
                },
                "responses": [
                    {"status": 200, "content": "json", "body": {"kind": "string"}}
                ],
                "auth": {"alternatives": [], "optional": False},
                "semantics": {"safe": False, "idempotent": False},
            },
            {
                "id": [["bodies"], ["form"]],
                "method": "POST",
                "path": [{"lit": "bodies"}, {"lit": "form"}],
                "body": {
                    "ty": {"ref": "FormBody"},
                    "content": "form",
                    "required": True,
                },
                "responses": [
                    {"status": 200, "content": "json", "body": {"kind": "string"}}
                ],
                "auth": {"alternatives": [], "optional": False},
                "semantics": {"safe": False, "idempotent": False},
            },
        ],
    }


@pytest.fixture
def bodies(tmp_path):
    generated = Generated(tmp_path, bodies_document(), "bodies_api")
    yield generated
    generated.close()


def call(api, method, body):
    seen = []

    def transport(request):
        seen.append(request)
        return 200, {"Content-Type": "application/json"}, b'"ok"'

    instance = api.Client("https://api.test", transport=transport)
    getattr(instance.bodies, method)(body=body)
    return seen[-1]


def test_text_body(bodies):
    request = call(bodies.api, "text", "hello")
    assert request.body == b"hello"
    assert request.headers["Content-Type"] == "text/plain"


def test_raw_bytes_body(bodies):
    request = call(bodies.api, "bytes", b"abc")
    assert request.body == b"abc"
    assert request.headers["Content-Type"] == "application/octet-stream"


def test_form_body(bodies):
    api = bodies.api
    request = call(api, "form", api.FormBody(name="ada", count=2))
    assert request.body == b"name=ada&count=2"
    assert request.headers["Content-Type"] == "application/x-www-form-urlencoded"


def test_multipart_body(bodies):
    api = bodies.api
    request = call(api, "multipart", api.FileBody(name="ada", file=b"abc"))
    assert request.headers["Content-Type"].startswith("multipart/form-data; boundary=")
    assert b'name="name"' in request.body
    assert b"ada" in request.body
    assert b'name="file"; filename="file"' in request.body
    assert b"abc" in request.body
