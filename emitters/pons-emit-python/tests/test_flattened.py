import importlib
import json
import sys

import pytest

import support


class Generated:
    def __init__(self, directory, document, package, *, options=None):
        merged = {"package": package}
        if options is not None:
            merged.update(options)
        self.response = support.emit(document, options=merged)
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


def document():
    value = support.base_document()
    value["types"]["Other"] = {
        "kind": "struct",
        "name": ["other"],
        "direction": "output",
        "openness": "reject",
        "fields": [
            {
                "wire": "other",
                "ident": ["other"],
                "ty": {"kind": "string"},
                "presence": "optional",
            },
            {
                "wire": "count",
                "ident": ["count"],
                "ty": {"kind": "int", "bits": 32, "signed": False, "wire": "number"},
                "presence": "required",
            },
        ],
    }
    value["types"]["Thing"]["flattened"] = [{"ref": "Other"}]
    return value


@pytest.fixture
def flat(tmp_path):
    generated = Generated(tmp_path, document(), "flat_api")
    yield generated
    generated.close()


def test_flattened_fields_are_merged_into_the_parent(flat):
    api = flat.api
    thing = api.Thing.from_wire({"value": "x", "other": "y", "count": 2})
    assert thing.value == "x"
    assert thing.other == "y"
    assert thing.count == 2
    assert thing.to_wire() == {"value": "x", "other": "y", "count": 2}


def test_nested_flattened_fields(tmp_path):
    value = document()
    value["types"]["Inner"] = {
        "kind": "struct",
        "name": ["inner"],
        "direction": "output",
        "openness": "reject",
        "fields": [
            {
                "wire": "inner",
                "ident": ["inner"],
                "ty": {"kind": "string"},
                "presence": "required",
            }
        ],
    }
    value["types"]["Middle"] = {
        "kind": "struct",
        "name": ["middle"],
        "direction": "output",
        "openness": "reject",
        "fields": [
            {
                "wire": "middle",
                "ident": ["middle"],
                "ty": {"kind": "string"},
                "presence": "required",
            }
        ],
        "flattened": [{"ref": "Inner"}],
    }
    value["types"]["Thing"]["flattened"] = [{"ref": "Middle"}, {"ref": "Other"}]
    generated = Generated(tmp_path, value, "nested_api")
    try:
        api = generated.api
        thing = api.Thing.from_wire(
            {"value": "x", "middle": "m", "inner": "i", "other": "y", "count": 2}
        )
        assert thing.inner == "i"
        assert thing.middle == "m"
        assert thing.other == "y"
    finally:
        generated.close()


def test_a_wire_collision_is_refused(tmp_path):
    value = document()
    value["types"]["Other"]["fields"][0]["wire"] = "value"
    response = support.emit(value)
    assert response["files"] == []
    assert "more than one field" in response["error"]


def test_a_flattened_map_is_refused(tmp_path):
    value = support.base_document()
    value["types"]["Thing"]["flattened"] = [
        {"kind": "map", "key": {"kind": "string"}, "value": {"kind": "string"}}
    ]
    response = support.emit(value)
    assert response["files"] == []
    assert "not a named struct" in response["error"]


def test_flattened_fields_are_validated(tmp_path):
    value = document()
    value["types"]["Other"]["fields"][0]["ty"] = {
        "kind": "string",
        "format": "email",
    }
    generated = Generated(
        tmp_path,
        value,
        "flat_validation_api",
        options={"validation": {"backend": "builtin"}},
    )
    try:
        api = generated.api
        assert generated.response["validation_coverage"]["unsupported"] == []

        def transport(request):
            return (
                200,
                {"Content-Type": "application/json"},
                json.dumps({"value": "x", "other": "nope", "count": 2}).encode(),
            )

        with pytest.raises(api.DecodeError) as caught:
            api.Client("https://api.test", transport=transport).things.get()
        assert caught.value.path == "other"
    finally:
        generated.close()
