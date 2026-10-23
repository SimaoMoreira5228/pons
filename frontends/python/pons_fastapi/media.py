from __future__ import annotations

from typing import Any

JSON = "application/json"
EVENT_STREAM = "text/event-stream"
URLENCODED = "application/x-www-form-urlencoded"
MULTIPART = "multipart/form-data"


def media_base(value: str) -> str:
    return value.split(";", 1)[0].strip().lower()


def is_json_media(value: str) -> bool:
    base = media_base(value)
    return base == JSON or base.endswith("+json")


def is_event_stream(value: str) -> bool:
    return media_base(value) == EVENT_STREAM


def content_kind(value: str) -> str:
    base = media_base(value)
    if is_json_media(value):
        return "json"
    if base == URLENCODED:
        return "form"
    if base.startswith("multipart/"):
        return "multipart"
    if base.startswith("text/"):
        return "text"
    return "bytes"


def error_content(value: str) -> str:
    base = media_base(value)
    if base == "application/problem+json":
        return "problem_json"
    if is_json_media(value):
        return "json"
    if base.startswith("text/"):
        return "text"
    raise ValueError(f"error media type {value!r} has no IR error content")


def opaque_type(codec: str, wire: str) -> dict[str, Any]:
    return {"kind": "opaque", "codec": codec, "wire": wire}


def media_descriptor(value: str | None) -> dict[str, Any] | None:
    if value is None:
        return None
    parts = value.split(';')
    base = parts[0].strip()
    if '/' not in base:
        return None
    media_type, subtype = (part.strip().lower() for part in base.split('/', 1))
    if not media_type or not subtype:
        return None
    parameters: dict[str, str] = {}
    for parameter in parts[1:]:
        if '=' not in parameter:
            return None
        name, raw = parameter.split('=', 1)
        name = name.strip().lower()
        raw = raw.strip()
        if not name or not raw:
            return None
        parameters[name] = raw
    descriptor: dict[str, Any] = {"type": media_type, "subtype": subtype}
    if parameters:
        descriptor["parameters"] = parameters
    return descriptor
