import importlib
import subprocess
import sys

import pytest

import support


def document():
    return {
        "ir_version": "0.3",
        "info": {"title": "Open", "version": "1"},
        "types": {
            "Count": {
                "kind": "newtype",
                "name": ["count"],
                "direction": "both",
                "inner": {
                    "kind": "int",
                    "bits": 32,
                    "signed": True,
                    "wire": "number",
                },
            },
            "Opaque": {
                "kind": "struct",
                "name": ["opaque"],
                "direction": "output",
                "openness": "preserve_opaque",
                "fields": [
                    {
                        "wire": "name",
                        "ident": ["name"],
                        "ty": {"kind": "string"},
                        "presence": "required",
                    }
                ],
            },
            "Typed": {
                "kind": "struct",
                "name": ["typed"],
                "direction": "output",
                "openness": {"preserve_typed": {"value": {"ref": "Count"}}},
                "fields": [
                    {
                        "wire": "label",
                        "ident": ["label"],
                        "ty": {"kind": "string"},
                        "presence": "required",
                    }
                ],
            },
        },
        "auth_schemes": {},
        "common_errors": [],
        "operations": [],
    }


class Generated:
    def __init__(self, directory, package):
        self.response = support.emit(document(), options={"package": package})
        assert self.response.get("error") is None, self.response.get("error")
        support.write_files(self.response, directory)
        self.directory = directory
        self.package = package
        sys.path.insert(0, str(directory))
        importlib.invalidate_caches()
        self.types = importlib.import_module(package + ".types")
        self.errors = importlib.import_module(package + ".errors")

    def close(self):
        sys.path.remove(str(self.directory))
        for name in list(sys.modules):
            if name == self.package or name.startswith(self.package + "."):
                del sys.modules[name]
        importlib.invalidate_caches()


@pytest.fixture
def generated(tmp_path):
    item = Generated(tmp_path, "open_api")
    yield item
    item.close()


def compiled(directory):
    result = subprocess.run(
        [sys.executable, "-m", "compileall", "-q", str(directory)],
        capture_output=True,
        text=True,
    )
    assert result.returncode == 0, result.stdout + result.stderr


def test_open_structs_emit_extra_fields_and_codecs(generated):
    assert generated.response["coverage"] == {
        "exact": ["types.Count", "types.Opaque", "types.Typed"],
        "opaque": [],
        "lossy": [],
        "unsupported": [],
    }
    compiled(generated.directory)
    types = (generated.directory / "open_api" / "types.py").read_text()
    assert "from dataclasses import dataclass, field" in types
    assert "from typing import Any" in types
    assert "extra: dict[str, Any] = field(default_factory=dict)" in types
    assert "extra: dict[str, Count] = field(default_factory=dict)" in types
    assert "if key not in ('name',)" in types
    assert "if key not in ('label',)" in types
    assert "Count.from_wire(item, _wire.child(path, key))" in types
    assert "for key, item in self.extra.items():" in types
    assert "out[key] = item\n" in types
    assert "out[key] = item.to_wire()\n" in types


def test_closed_structs_do_not_import_field_or_any():
    response = support.emit(support.base_document())
    assert response.get("error") is None
    types = next(
        file["content"]
        for file in response["files"]
        if file["path"].endswith("types.py")
    )
    assert "from dataclasses import dataclass\n" in types
    assert "from typing import Any" not in types


def test_preserve_opaque_round_trip(generated):
    types = generated.types
    extras = {"beta": [1, {"nested": True}], "gamma": None}
    value = types.Opaque.from_wire({"name": "x", **extras})
    assert value.name == "x"
    assert value.extra == extras
    assert value.to_wire() == {"name": "x", **extras}

    assert types.Opaque(name="x").extra == {}
    assert types.Opaque.from_wire({"name": "x"}).extra == {}
    assert types.Opaque(name="x").to_wire() == {"name": "x"}

    collision = types.Opaque(name="x", extra={"name": "override", "kept": 2})
    assert collision.to_wire() == {"name": "x", "kept": 2}


def test_preserve_typed_round_trip(generated):
    types = generated.types
    value = types.Typed.from_wire({"label": "a", "first": 3})
    assert value.extra == {"first": types.Count(3)}
    assert isinstance(value.extra["first"], types.Count)
    assert value.to_wire() == {"label": "a", "first": 3}

    assert types.Typed.from_wire({"label": "a"}).extra == {}

    with pytest.raises(generated.errors.DecodeError) as caught:
        types.Typed.from_wire({"label": "a", "first": "three"})
    assert caught.value.path == "first"
