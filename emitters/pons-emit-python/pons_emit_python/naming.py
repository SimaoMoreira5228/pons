"""Identifier casing and collision handling for the Python emitter."""

from __future__ import annotations

import keyword
import re
from collections.abc import Iterable

_SPLIT = re.compile(r"[^A-Za-z0-9]+")

_TYPE_RESERVED = frozenset(
    {
        "Any",
        "Decimal",
        "Enum",
        "dataclass",
        "None",
        "True",
        "False",
        "AuthAdapter",
        "AuthAttachment",
        "AuthContext",
        "Client",
        "Request",
        "Response",
        "Transport",
        "PonsError",
        "TransportError",
        "DecodeError",
        "UnexpectedStatusError",
        "ApiError",
        "ConfigurationError",
    }
)
_FIELD_RESERVED = frozenset({"self", "cls"})
_PARAMETER_RESERVED = frozenset({"self", "cls", "query", "body"})
_CLIENT_RESERVED = frozenset(
    {"auth_adapters", "base_url", "credentials", "timeout", "transport", "headers"}
)
_NAMESPACE_RESERVED = frozenset({"_client"})


def words(value: str) -> list[str]:
    return [part for part in _SPLIT.split(value) if part]


def _parts(groups: Iterable[Iterable[str]]) -> list[str]:
    parts: list[str] = []
    for group in groups:
        for word in group:
            parts.extend(part.lower() for part in words(word))
    return parts


def snake(groups: Iterable[Iterable[str]] | None, reserved: frozenset[str] = frozenset()) -> str:
    parts = _parts(groups or [])
    name = "_".join(parts) or "value"
    if name[0].isdigit():
        name = "value_" + name
    while keyword.iskeyword(name) or name in reserved:
        name += "_"
    return name


def pascal(groups: Iterable[Iterable[str]] | None, reserved: frozenset[str] = frozenset()) -> str:
    parts = _parts(groups or [])
    name = "".join(part[:1].upper() + part[1:] for part in parts) or "Value"
    if name[0].isdigit():
        name = "N" + name
    while keyword.iskeyword(name) or name in reserved:
        name += "_"
    return name


def screaming(groups: Iterable[Iterable[str]] | None) -> str:
    parts = _parts(groups or [])
    name = "_".join(part.upper() for part in parts) or "VALUE"
    if name[0].isdigit():
        name = "VALUE_" + name
    if name.startswith("_"):
        name = "VALUE" + name
    return name


def words_from_wire(wire: str) -> list[str]:
    return words(wire) or ["value"]


def operation_path(segments: Iterable[Iterable[str]]) -> str:
    escaped: list[str] = []
    for segment in segments:
        escaped.append(
            "_".join(
                word.replace("%", "%25").replace(".", "%2E").replace("_", "%5F")
                for word in segment
            )
        )
    return ".".join(escaped)


def type_reserved() -> frozenset[str]:
    return _TYPE_RESERVED


def field_reserved() -> frozenset[str]:
    return _FIELD_RESERVED


def parameter_reserved() -> frozenset[str]:
    return _PARAMETER_RESERVED


def client_reserved() -> frozenset[str]:
    return _CLIENT_RESERVED


def namespace_reserved() -> frozenset[str]:
    return _NAMESPACE_RESERVED


class NameRegistry:
    def __init__(self, reserved: Iterable[str] = ()) -> None:
        self.used = set(reserved)

    def claim(self, base: str, separator: str = "_") -> str:
        if base not in self.used:
            self.used.add(base)
            return base
        index = 2
        while f"{base}{separator}{index}" in self.used:
            index += 1
        name = f"{base}{separator}{index}"
        self.used.add(name)
        return name
