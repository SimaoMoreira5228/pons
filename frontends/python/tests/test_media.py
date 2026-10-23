from __future__ import annotations

from typing import Annotated

from fastapi import FastAPI
from fastapi.responses import Response, StreamingResponse
from pydantic import BaseModel

from pons_fastapi import build_document, pons_media, pons_stream, pons_type


class EventStreamResponse(StreamingResponse):
    media_type = "text/event-stream"


class Payload(BaseModel):
    value: int


class Overridden(BaseModel):
    amount: Annotated[str, pons_type("decimal", wire="string")]
    token: Annotated[str, pons_type("string", format="uuid")]
    blob: Annotated[bytes, pons_type("opaque", codec="acme.blob", wire="bytes")]
    count: Annotated[int, pons_type("int", bits=64, signed=False, wire="string")]


def operations(document: dict) -> dict[str, dict]:
    return {
        ".".join(".".join(name) for name in operation["id"]): operation
        for operation in document["operations"]
    }


def test_pons_type_overrides_parameters_and_response_fields() -> None:
    app = FastAPI(title="Overrides", version="1")

    @app.get("/payload")
    def payload(
        amount: Annotated[str, pons_type("decimal", wire="string")],
    ) -> Overridden:
        return Overridden(amount=amount, token="t", blob=b"x", count=1)

    document = build_document(app)
    operation = operations(document)["payload"]
    query = document["types"][operation["query"]["ref"]]
    assert query["fields"][0]["ty"] == {"kind": "decimal", "wire": "string"}

    fields = {
        field["wire"]: field["ty"]
        for field in document["types"]["test_media.Overridden"]["fields"]
    }
    assert fields["amount"] == {"kind": "decimal", "wire": "string"}
    assert fields["token"] == {"kind": "string", "format": "uuid"}
    assert fields["blob"] == {"kind": "opaque", "codec": "acme.blob", "wire": "bytes"}
    assert fields["count"] == {
        "kind": "int",
        "bits": 64,
        "signed": False,
        "wire": "string",
    }


def test_streaming_response_without_a_declaration_is_skipped() -> None:
    app = FastAPI(title="Undeclared", version="1")

    @app.get("/events", response_class=EventStreamResponse)
    def events() -> EventStreamResponse:
        return EventStreamResponse(iter(()))

    document = build_document(app)
    assert operations(document) == {}
    assert any(
        "explicit @pons_stream" in entry["message"]
        for entry in document["diagnostics"]
        if entry["code"] == "fastapi.unsupported_route"
    )


def test_sse_without_event_stream_media_type_is_skipped() -> None:
    app = FastAPI(title="Wrong media", version="1")

    @app.get("/events", response_class=StreamingResponse)
    @pons_stream(events={"tick": Payload})
    def events() -> StreamingResponse:
        return StreamingResponse(iter(()))

    document = build_document(app)
    assert operations(document) == {}
    assert any(
        "text/event-stream" in entry["message"]
        for entry in document["diagnostics"]
        if entry["code"] == "fastapi.unsupported_route"
    )


def test_declared_custom_media_is_opaque_instead_of_json() -> None:
    app = FastAPI(title="Custom media", version="1")

    @app.get("/xml", response_class=Response)
    @pons_media(response="application/xml")
    def xml() -> Payload:
        return Payload(value=1)

    document = build_document(app)
    response = operations(document)["xml"]["responses"][0]
    assert response["content"] == "bytes"
    assert response["media"] == {"type": "application", "subtype": "xml"}
    assert response["body"] == {
        "kind": "opaque",
        "codec": "fastapi.response",
        "wire": "bytes",
    }
    diagnostics = operations(document)["xml"]["diagnostics"]
    assert any(entry["code"] == "fastapi.opaque_media" for entry in diagnostics)


def test_custom_media_response_without_a_declaration_is_skipped() -> None:
    app = FastAPI(title="Undeclared media", version="1")

    @app.get("/raw")
    def raw() -> Response:
        return Response(content=b"x", media_type="application/xml")

    document = build_document(app)
    assert operations(document) == {}
    assert any(
        "no response model" in entry["message"]
        for entry in document["diagnostics"]
        if entry["code"] == "fastapi.unsupported_route"
    )


def test_multipart_body_uses_an_opaque_type_and_exposes_no_part_schemas() -> None:
    from fastapi import File, Form, UploadFile

    app = FastAPI(title="Upload", version="1")

    @app.post("/upload")
    def upload(file: UploadFile = File(...), label: str = Form(...)) -> Payload:
        return Payload(value=1)

    document = build_document(app)
    operation = operations(document)["upload"]
    assert operation["body"] == {
        "ty": {"kind": "opaque", "codec": "fastapi.multipart", "wire": "unknown"},
        "content": "multipart",
        "required": True,
        "media": {"type": "multipart", "subtype": "form-data"},
    }
    assert any(
        entry["code"] == "fastapi.opaque_multipart"
        for entry in operation["diagnostics"]
    )
    assert not any(name.endswith("::form") for name in document["types"])
