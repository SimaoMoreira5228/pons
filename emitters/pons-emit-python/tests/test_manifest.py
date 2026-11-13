import pytest

import support
from pons_emit_python.manifest import build_manifest


def test_manifest_declares_the_python_target():
    manifest = build_manifest()
    assert manifest["name"] == "python"
    assert manifest["version"] == "0.1.0"
    assert manifest["protocol"] == {"min": "3.0", "max": "3.0"}
    assert manifest["ir_version"] == {"min": "0.3", "max": "0.3"}
    assert manifest["option_schema"] == {
        "options": {"package": {"type": "string", "required": False}}
    }


def test_manifest_declares_the_supported_subset():
    capabilities = build_manifest()["capabilities"]
    assert capabilities["streaming"] == []
    assert capabilities["pagination"] == []
    assert capabilities["enum_reprs"] == [
        "string",
        "external",
        "internal",
        "adjacent",
    ]
    assert capabilities["direction_split"] is True
    assert capabilities["provides"] is True
    assert capabilities["custom_auth"] is False
    assert capabilities["flattened"] is False
    assert capabilities["wide_integers"] is True
    assert capabilities["opaque"] is False
    assert capabilities["object_openness"] == ["reject"]
    assert capabilities["extensions"] == {}
    assert capabilities["validation"] == {"backends": [], "capabilities": {}}


def test_default_package_option_is_used():
    response = support.emit(support.load("client.json"))
    assert response.get("error") is None
    assert response["files"][0]["path"].startswith("pons_client/")


def test_custom_package_option_is_used():
    response = support.emit(
        support.load("client.json"), options={"package": "acme_client"}
    )
    assert response.get("error") is None
    assert {file["path"].split("/")[0] for file in response["files"]} == {
        "acme_client"
    }


@pytest.mark.parametrize(
    "package",
    ["", "not a package", "1client", "class", "for", "main", "__init__", "../evil", "a/b"],
)
def test_bad_package_option_is_refused(package):
    response = support.emit(support.load("client.json"), options={"package": package})
    assert response["files"] == []
    assert response["error"]
    assert response["diagnostics"][0]["code"].startswith("PY-E")
    assert all(code.startswith("PY-E") for code in _codes(response))


def test_unknown_option_is_refused():
    response = support.emit(
        support.load("client.json"), options={"package": "pons_client", "zod": True}
    )
    assert response["files"] == []
    assert "zod" in response["error"]


def test_non_string_package_option_is_refused():
    response = support.emit(support.load("client.json"), options={"package": 3})
    assert response["files"] == []
    assert "string" in response["error"]


def test_wrong_protocol_is_refused():
    response = support.emit(support.load("client.json"), protocol=2)
    assert response["files"] == []
    assert "protocol" in response["error"]


def test_wrong_target_is_refused():
    response = support.emit(support.load("client.json"), target="go")
    assert response["files"] == []
    assert "targets" in response["error"]


def test_unknown_mode_is_refused():
    response = support.emit(support.load("client.json"), mode="loose")
    assert response["files"] == []
    assert "mode" in response["error"]


def test_wrong_ir_version_is_refused():
    document = support.load("client.json")
    document["ir_version"] = "0.2"
    response = support.emit(document)
    assert response["files"] == []
    assert "unsupported IR version" in response["error"]


def test_unknown_request_member_is_refused():
    payload = support.request(support.load("client.json"), options={})
    payload["extra"] = True
    response = support.emit_raw(payload)
    assert response["files"] == []
    assert "unknown member" in response["error"]


def test_failure_carries_full_coverage():
    document = support.load("client.json")
    document["operations"][0]["stream"] = {
        "kind": "ndjson",
        "item": {"ref": "Session"},
    }
    response = support.emit(document)
    assert response["files"] == []
    assert response["error"]
    assert set(support.coverage_subjects(response)) == support.subjects(document)
    assert len(support.coverage_subjects(response)) == len(support.subjects(document))
    assert response["coverage"]["unsupported"] == ["operations.auth.login"]
    assert "operations.auth.login" not in response["coverage"]["exact"]


def _codes(response):
    return [diagnostic["code"] for diagnostic in response["diagnostics"]]
