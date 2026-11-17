import json

import pytest

import support


def document():
    value = support.base_document()
    value["types"]["Shape"] = {
        "kind": "enum",
        "name": ["shape"],
        "direction": "output",
        "repr": {"kind": "untagged"},
        "variants": [
            {
                "wire": "nothing",
                "ident": ["nothing"],
                "payload": {"kind": "unit"},
            },
            {
                "wire": "count",
                "ident": ["count"],
                "payload": {
                    "kind": "newtype",
                    "ty": {
                        "kind": "int",
                        "bits": 32,
                        "signed": True,
                        "wire": "number",
                    },
                },
            },
            {
                "wire": "pair",
                "ident": ["pair"],
                "payload": {
                    "kind": "tuple",
                    "items": [{"kind": "string"}, {"kind": "bool"}],
                },
            },
            {
                "wire": "record",
                "ident": ["record"],
                "payload": {
                    "kind": "struct",
                    "fields": [
                        {
                            "wire": "note",
                            "ident": ["note"],
                            "ty": {"kind": "string", "min_length": 2},
                            "presence": "required",
                        }
                    ],
                },
            },
        ],
    }
    value["types"]["Clash"] = {
        "kind": "enum",
        "name": ["clash"],
        "direction": "output",
        "repr": {"kind": "untagged"},
        "variants": [
            {
                "wire": "first",
                "ident": ["first"],
                "payload": {"kind": "newtype", "ty": {"kind": "string"}},
            },
            {
                "wire": "second",
                "ident": ["second"],
                "payload": {"kind": "newtype", "ty": {"kind": "string"}},
            },
        ],
    }
    value["types"]["Thing"]["fields"].append(
        {
            "wire": "shape",
            "ident": ["shape"],
            "ty": {"ref": "Shape"},
            "presence": "required",
        }
    )
    return value


@pytest.fixture
def untagged(tmp_path):
    generated = support.Generated(tmp_path, document(), "untagged_api")
    yield generated
    generated.close()


@pytest.mark.parametrize(
    "wire, name",
    [
        (None, "ShapeNothing"),
        (3, "ShapeCount"),
        (["a", True], "ShapePair"),
        ({"note": "n"}, "ShapeRecord"),
    ],
)
def test_untagged_payloads_round_trip(untagged, wire, name):
    decoded = untagged.types.decode_Shape(wire)
    assert type(decoded).__name__ == name
    assert decoded.to_wire() == wire


def test_untagged_decodes_through_a_struct_field(untagged):
    thing = untagged.types.Thing.from_wire({"value": "v", "shape": 3})
    assert isinstance(thing.shape, untagged.types.ShapeCount)
    assert thing.shape.value == 3
    assert thing.to_wire() == {"value": "v", "shape": 3}


def test_untagged_unit_payload_is_null(untagged):
    decoded = untagged.types.decode_Shape(None)
    assert isinstance(decoded, untagged.types.ShapeNothing)
    assert decoded.to_wire() is None
    with pytest.raises(untagged.errors.DecodeError):
        untagged.types.ShapeNothing.from_wire("not null")


def test_untagged_requires_a_matching_variant(untagged):
    with pytest.raises(untagged.errors.DecodeError) as caught:
        untagged.types.decode_Shape(3.5)
    assert "matches no variant" in caught.value.message


def test_untagged_reports_an_ambiguous_value(untagged):
    with pytest.raises(untagged.errors.DecodeError) as caught:
        untagged.types.decode_Clash("x")
    assert "ambiguous" in caught.value.message
    assert "first" in caught.value.message
    assert "second" in caught.value.message


def test_untagged_coverage_is_exact(untagged):
    assert untagged.response["coverage"] == {
        "exact": [
            "operations.things.get",
            "types.Clash",
            "types.Shape",
            "types.Thing",
        ],
        "opaque": [],
        "lossy": [],
        "unsupported": [],
    }


def test_untagged_validation_is_exact_and_checks_payloads(tmp_path):
    generated = support.Generated(
        tmp_path,
        document(),
        "untagged_validation_api",
        options={"validation": {"backend": "builtin"}},
    )
    try:
        assert generated.response["validation_coverage"] == {
            "exact": generated.response["coverage"]["exact"],
            "lossy": [],
            "unsupported": [],
        }

        def transport(request):
            return (
                200,
                {"Content-Type": "application/json"},
                json.dumps({"value": "v", "shape": {"note": "a"}}).encode("utf-8"),
            )

        with pytest.raises(generated.api.DecodeError) as caught:
            generated.api.Client(
                "https://api.test", transport=transport
            ).things.get()
        assert caught.value.path == "shape.note"
    finally:
        generated.close()
