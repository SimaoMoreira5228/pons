import pytest

import support


def document_with(callback):
    return support.mutate(support.base_document(), callback)


def assert_refused(document, *, marker=None, mode="strict"):
    response = support.emit(document, mode=mode)
    assert response["files"] == []
    assert response["error"]
    assert response["diagnostics"]
    assert all(item["code"].startswith("PY-E") for item in response["diagnostics"])
    subjects = support.coverage_subjects(response)
    assert set(subjects) == support.subjects(document)
    assert len(subjects) == len(set(subjects))
    if marker is not None:
        messages = " ".join(item["message"] for item in response["diagnostics"])
        assert marker in messages, messages
    return response


def test_stream_is_refused():
    def change(value):
        value["operations"][0]["stream"] = {
            "kind": "websocket",
            "client": {"kind": "string"},
            "server": {"ref": "Thing"},
        }

    assert_refused(document_with(change), marker="WebSocket")


@pytest.mark.parametrize("content", ["multipart", "form", "text", "bytes"])
def test_non_json_request_body_is_refused(content):
    def change(value):
        value["operations"][0]["body"] = {
            "ty": {"kind": "string"},
            "content": content,
            "required": True,
        }

    assert_refused(document_with(change), marker=content)


def test_non_json_success_content_is_refused():
    def change(value):
        value["operations"][0]["responses"][0] = {
            "status": 200,
            "content": "empty",
        }

    assert_refused(document_with(change), marker="empty")


def test_invalid_enum_representation_is_refused():
    def change(value):
        value["types"]["Broken"] = {
            "kind": "enum",
            "name": ["broken"],
            "direction": "output",
            "repr": {"kind": "mystery"},
            "variants": [
                {"wire": "a", "ident": ["a"], "payload": {"kind": "unit"}}
            ],
        }

    assert_refused(document_with(change), marker="mystery")


def test_opaque_value_is_supported():
    def change(value):
        value["types"]["Thing"]["fields"][0]["ty"] = {
            "kind": "opaque",
            "codec": "custom",
            "wire": "json",
        }

    response = support.emit(document_with(change))
    assert response.get("error") is None
    assert response["coverage"]["opaque"] == [
        "operations.things.get",
        "types.Thing",
    ]


def test_invalid_openness_is_refused():
    def change(value):
        value["types"]["Thing"]["openness"] = "allow"

    assert_refused(document_with(change), marker="invalid object openness")


def test_preserve_typed_without_a_value_is_refused():
    def change(value):
        value["types"]["Thing"]["openness"] = {"preserve_typed": {}}

    assert_refused(document_with(change), marker="without a value type")


def test_flattened_member_is_refused():
    def change(value):
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
                    "presence": "required",
                }
            ],
        }
        value["types"]["Thing"]["flattened"] = [{"ref": "Other"}]

    assert_refused(document_with(change), marker="flattened")


def test_optional_nullable_field_is_supported():
    def change(value):
        value["types"]["Thing"]["fields"][0]["presence"] = "optional"
        value["types"]["Thing"]["fields"][0]["ty"] = {
            "kind": "nullable",
            "inner": {"kind": "string"},
        }

    response = support.emit(document_with(change))
    assert response.get("error") is None
    assert response["coverage"]["unsupported"] == []


def test_required_nullable_field_is_supported():
    def change(value):
        value["types"]["Thing"]["fields"][0]["ty"] = {
            "kind": "nullable",
            "inner": {"kind": "string"},
        }

    response = support.emit(document_with(change))
    assert response.get("error") is None
    assert response["coverage"]["unsupported"] == []


def test_pagination_without_a_query_field_is_refused():
    def change(value):
        value["operations"][0]["semantics"]["pagination"] = {
            "request": {"kind": "cursor", "field": "cursor"},
            "items": "",
            "next": "/next",
        }

    assert_refused(document_with(change), marker="not declared on the operation query")


def test_non_json_media_is_refused():
    def change(value):
        value["operations"][0]["responses"][0]["media"] = {
            "type": "text",
            "subtype": "plain",
        }

    assert_refused(document_with(change), marker="text/plain")


def test_media_codec_is_refused():
    def change(value):
        value["operations"][0]["responses"][0]["media"] = {
            "type": "application",
            "subtype": "json",
            "codec": "custom",
        }

    assert_refused(document_with(change), marker="application/json")


def test_custom_auth_is_supported():
    def change(value):
        value["auth_schemes"]["custom"] = {
            "kind": "custom",
            "description": "handshake",
            "adapter": "handshake",
        }
        value["operations"][0]["auth"] = {
            "alternatives": [["custom"]],
            "optional": False,
        }

    response = support.emit(document_with(change))
    assert response.get("error") is None
    assert response["coverage"]["unsupported"] == []
    assert response["coverage"]["exact"] == [
        "operations.things.get",
        "types.Thing",
    ]


def test_decimal_number_wire_is_refused():
    def change(value):
        value["types"]["Thing"]["fields"][0]["ty"] = {
            "kind": "decimal",
            "wire": "number",
        }

    assert_refused(document_with(change), marker="decimals as strings")


def test_bare_null_is_refused():
    def change(value):
        value["types"]["Thing"]["fields"][0]["ty"] = {"kind": "null"}

    assert_refused(document_with(change), marker="null type")


def test_non_string_map_key_is_refused():
    def change(value):
        value["types"]["Thing"]["fields"][0]["ty"] = {
            "kind": "map",
            "key": {"kind": "int", "bits": 32, "signed": True, "wire": "number"},
            "value": {"kind": "string"},
        }

    assert_refused(document_with(change), marker="map key")


def test_unknown_narrowed_variant_is_refused():
    def change(value):
        value["types"]["Choice"] = {
            "kind": "enum",
            "name": ["choice"],
            "direction": "output",
            "repr": {"kind": "string"},
            "variants": [
                {"wire": "a", "ident": ["a"], "payload": {"kind": "unit"}}
            ],
        }
        value["types"]["Thing"]["fields"][0]["ty"] = {
            "ref": "Choice",
            "variant": "ghost",
        }

    assert_refused(document_with(change), marker="unknown variant")


def test_narrowing_a_non_enum_is_refused():
    def change(value):
        value["types"]["Thing"]["fields"][0]["ty"] = {
            "ref": "Thing",
            "variant": "ghost",
        }

    assert_refused(document_with(change), marker="non-enum")


def test_variant_narrowed_ref_is_supported():
    def change(value):
        value["types"]["Choice"] = {
            "kind": "enum",
            "name": ["choice"],
            "direction": "output",
            "repr": {"kind": "string"},
            "variants": [
                {"wire": "a", "ident": ["a"], "payload": {"kind": "unit"}}
            ],
        }
        value["types"]["Thing"]["fields"][0]["ty"] = {
            "ref": "Choice",
            "variant": "a",
        }

    response = support.emit(document_with(change))
    assert response.get("error") is None
    assert response["coverage"]["unsupported"] == []


def test_internal_enum_with_a_non_object_payload_is_refused():
    def change(value):
        value["types"]["Broken"] = {
            "kind": "enum",
            "name": ["broken"],
            "direction": "output",
            "repr": {"kind": "internal", "tag": "kind"},
            "variants": [
                {
                    "wire": "a",
                    "ident": ["a"],
                    "payload": {"kind": "newtype", "ty": {"kind": "string"}},
                }
            ],
        }

    assert_refused(document_with(change), marker="not an object")


def test_multiple_success_responses_are_refused():
    def change(value):
        value["operations"][0]["responses"].append(
            {"status": 201, "content": "json", "body": {"ref": "Thing"}}
        )

    assert_refused(document_with(change), marker="success responses")


def test_class_success_status_is_refused():
    def change(value):
        value["operations"][0]["responses"][0]["status"] = "2xx"

    assert_refused(document_with(change), marker="exact 2xx")


def test_optional_nullable_body_is_refused():
    def change(value):
        value["operations"][0]["body"] = {
            "ty": {"kind": "nullable", "inner": {"kind": "string"}},
            "content": "json",
            "required": False,
        }

    assert_refused(document_with(change), marker="nullable")


def test_text_error_is_refused():
    def change(value):
        value["operations"][0]["errors"] = [
            {
                "status": 400,
                "name": ["bad", "request"],
                "content": "text",
                "body": {"kind": "string"},
            }
        ]

    assert_refused(document_with(change), marker="text")


def test_read_only_field_in_an_output_type_is_emitted():
    def change(value):
        value["types"]["Thing"]["fields"][0]["access"] = "read_only"

    response = support.emit(document_with(change))
    assert response.get("error") is None, response.get("error")
    assert response["coverage"]["exact"] == [
        "operations.things.get",
        "types.Thing",
    ]


def test_read_only_field_in_a_both_type_is_refused():
    def change(value):
        value["types"]["Thing"]["direction"] = "both"
        value["types"]["Thing"]["fields"][0]["access"] = "read_only"

    assert_refused(document_with(change), marker="read_only")


def test_refusal_keeps_every_subject_once():
    document = document_with(
        lambda value: value["operations"][0].update(
            {"stream": {"kind": "ndjson", "item": {"ref": "Thing"}}}
        )
    )
    response = assert_refused(document)
    assert response["coverage"]["exact"] == ["types.Thing"]
    assert response["coverage"]["unsupported"] == ["operations.things.get"]
