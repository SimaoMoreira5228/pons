"""Wire helpers for the generated client."""

from __future__ import annotations

import base64
import binascii
import json
import math
import re
from collections.abc import Callable, Iterable, Iterator, Mapping
from decimal import Decimal, InvalidOperation
from typing import Any
from urllib.parse import quote

from .errors import DecodeError


class _MissingType:
    __slots__ = ()

    def __repr__(self) -> str:
        return "MISSING"


Missing = _MissingType
MISSING = _MissingType()

_INT_TEXT = re.compile(r"-?(?:0|[1-9][0-9]*)")


def child(path: str, key: str) -> str:
    return f"{path}.{key}" if path else key


def index(path: str, position: int) -> str:
    return f"{path}[{position}]"


def _kind(value: object) -> str:
    if value is None:
        return "null"
    if isinstance(value, bool):
        return "a boolean"
    if isinstance(value, (int, float)):
        return "a number"
    if isinstance(value, str):
        return "a string"
    if isinstance(value, list):
        return "an array"
    if isinstance(value, dict):
        return "an object"
    return type(value).__name__


def obj(value: object, path: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise DecodeError(f"expected an object, found {_kind(value)}", path=path)
    return value


def field(data: Mapping[str, Any], key: str, path: str) -> Any:
    if key not in data:
        raise DecodeError("missing field", path=child(path, key))
    return data[key]


def opt(data: Mapping[str, Any], key: str) -> Any:
    return data.get(key, MISSING)


def bool_of(value: object, path: str) -> bool:
    if not isinstance(value, bool):
        raise DecodeError(f"expected a boolean, found {_kind(value)}", path=path)
    return value


def int_of(
    value: object, path: str, bits: int, signed: bool, wire: str
) -> int:
    if wire == "string":
        if not isinstance(value, str) or _INT_TEXT.fullmatch(value) is None:
            raise DecodeError(
                f"expected an integer string, found {_kind(value)}", path=path
            )
        number = int(value, 10)
    else:
        if isinstance(value, bool) or not isinstance(value, int):
            raise DecodeError(
                f"expected an integer, found {_kind(value)}", path=path
            )
        number = value
    low = -(1 << (bits - 1)) if signed else 0
    high = (1 << (bits - 1)) - 1 if signed else (1 << bits) - 1
    if number < low or number > high:
        raise DecodeError(
            f"the integer {number} does not fit in {bits} bits", path=path
        )
    return number


def float_of(value: object, path: str, bits: int) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise DecodeError(f"expected a number, found {_kind(value)}", path=path)
    number = float(value)
    if not math.isfinite(number):
        raise DecodeError("expected a finite number", path=path)
    return number


def decimal_of(value: object, path: str) -> Decimal:
    if not isinstance(value, str):
        raise DecodeError(
            f"expected a decimal string, found {_kind(value)}", path=path
        )
    try:
        number = Decimal(value)
    except InvalidOperation as error:
        raise DecodeError(str(error), path=path) from error
    if not number.is_finite():
        raise DecodeError("expected a finite decimal", path=path)
    return number


def str_of(value: object, path: str) -> str:
    if not isinstance(value, str):
        raise DecodeError(f"expected a string, found {_kind(value)}", path=path)
    return value


def bytes_of(value: object, path: str, encoding: str) -> bytes:
    if encoding == "array":
        if not isinstance(value, list):
            raise DecodeError(f"expected an array, found {_kind(value)}", path=path)
        result = bytearray()
        for position, item in enumerate(value):
            if (
                isinstance(item, bool)
                or not isinstance(item, int)
                or item < 0
                or item > 255
            ):
                raise DecodeError(
                    "expected a byte between 0 and 255",
                    path=index(path, position),
                )
            result.append(item)
        return bytes(result)
    if not isinstance(value, str):
        raise DecodeError(f"expected a byte string, found {_kind(value)}", path=path)
    try:
        if encoding == "base64":
            return base64.b64decode(value, validate=True)
        if encoding == "base64url":
            padded = value + "=" * (-len(value) % 4)
            translated = padded.replace("-", "+").replace("_", "/")
            return base64.b64decode(translated, validate=True)
        if encoding == "hex":
            return binascii.unhexlify(value)
    except (binascii.Error, ValueError) as error:
        raise DecodeError(str(error), path=path) from error
    raise DecodeError(f"unknown byte encoding {encoding!r}", path=path)


def null_of(value: object, path: str) -> None:
    if value is not None:
        raise DecodeError(f"expected null, found {_kind(value)}", path=path)
    return None


def any_of(value: object, path: str) -> object:
    return value


def variant_mismatch(
    enum: str, expected: str, found: object, path: str
) -> DecodeError:
    return DecodeError(
        f"expected the {expected} variant of {enum}, found {found!r}", path=path
    )


def unknown_variant(enum: str, found: object, path: str) -> DecodeError:
    return DecodeError(f"unknown {enum} variant {found!r}", path=path)


def variant_shape(enum: str, expected: str, path: str) -> DecodeError:
    return DecodeError(f"{enum} expects {expected}", path=path)


def no_untagged_variant(enum: str, path: str) -> DecodeError:
    return DecodeError(f"the value matches no variant of {enum}", path=path)


def ambiguous_untagged_variant(
    enum: str, variants: list[str], path: str
) -> DecodeError:
    names = ", ".join(variants)
    return DecodeError(
        f"the value is ambiguous for {enum}; it matches the {names} variants",
        path=path,
    )


def nullable(
    value: object, path: str, decode: Callable[[object, str], Any]
) -> Any:
    if value is None:
        return None
    return decode(value, path)


def list_of(
    value: object, path: str, decode: Callable[[object, str], Any]
) -> list[Any]:
    if not isinstance(value, list):
        raise DecodeError(f"expected an array, found {_kind(value)}", path=path)
    return [
        decode(item, index(path, position))
        for position, item in enumerate(value)
    ]


def map_of(
    value: object, path: str, decode: Callable[[object, str], Any]
) -> dict[str, Any]:
    data = obj(value, path)
    return {
        key: decode(item, child(path, key)) for key, item in data.items()
    }


def tuple_of(
    value: object,
    path: str,
    decoders: list[Callable[[object, str], Any]],
) -> tuple[Any, ...]:
    if not isinstance(value, list):
        raise DecodeError(f"expected an array, found {_kind(value)}", path=path)
    if len(value) != len(decoders):
        raise DecodeError(
            f"expected an array of {len(decoders)} items", path=path
        )
    return tuple(
        decoder(value[position], index(path, position))
        for position, decoder in enumerate(decoders)
    )


def inline_decode(
    value: object,
    path: str,
    fields: list[tuple[str, bool, bool, Callable[[object, str], Any]]],
) -> dict[str, Any]:
    data = obj(value, path)
    result: dict[str, Any] = {}
    for wire, optional, nullable, decode in fields:
        field_path = child(path, wire)
        raw = data.get(wire, MISSING)
        if raw is MISSING:
            if optional:
                result[wire] = MISSING if nullable else None
                continue
            raise DecodeError("missing field", path=field_path)
        result[wire] = decode(raw, field_path)
    return result


def decimal_to(value: Decimal) -> str:
    return str(value)


def bytes_to(value: bytes, encoding: str) -> object:
    if encoding == "array":
        return list(value)
    if encoding == "base64":
        return base64.b64encode(value).decode("ascii")
    if encoding == "base64url":
        return base64.urlsafe_b64encode(value).decode("ascii").rstrip("=")
    if encoding == "hex":
        return value.hex()
    raise ValueError(f"unknown byte encoding {encoding!r}")


def nullable_to(value: object, encode: Callable[[Any], Any]) -> object:
    return None if value is None else encode(value)


def list_to(value: list[Any], encode: Callable[[Any], Any]) -> list[Any]:
    return [encode(item) for item in value]


def map_to(
    value: dict[str, Any], encode: Callable[[Any], Any]
) -> dict[str, Any]:
    return {key: encode(item) for key, item in value.items()}


def tuple_to(
    value: tuple[Any, ...], encoders: list[Callable[[Any], Any]]
) -> list[Any]:
    return [encoder(item) for encoder, item in zip(encoders, value)]


def inline_encode(
    value: dict[str, Any],
    fields: list[tuple[str, bool, bool, Callable[[Any], Any]]],
) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for wire, optional, nullable, encode in fields:
        item = value.get(wire)
        if optional and item is (MISSING if nullable else None):
            continue
        result[wire] = encode(item)
    return result


def text(value: object) -> str:
    if isinstance(value, bool):
        return "true" if value else "false"
    if isinstance(value, Decimal):
        return str(value)
    return str(value)


def path_segment(value: object) -> str:
    return quote(str(value), safe="")


def normalize_headers(headers: Mapping[str, Any] | None) -> dict[str, str]:
    if headers is None:
        return {}
    result: dict[str, str] = {}
    for name, value in headers.items():
        if isinstance(value, (list, tuple)):
            result[str(name)] = "\n".join(str(item) for item in value)
        else:
            result[str(name)] = str(value)
    return result


def header(headers: Mapping[str, str], name: str) -> str | None:
    lowered = name.lower()
    for key, value in headers.items():
        if key.lower() == lowered:
            return value
    return None


def require_header(headers: Mapping[str, str], name: str, path: str) -> str:
    value = header(headers, name)
    if value is None:
        raise DecodeError("missing response header", path=path)
    return value


def _media(value: str) -> tuple[str, dict[str, str]]:
    base, *parameters = value.split(";")
    parsed: dict[str, str] = {}
    for parameter in parameters:
        name, separator, item = parameter.partition("=")
        if not separator:
            continue
        parsed[name.strip().lower()] = item.strip().lower()
    return base.strip().lower(), parsed


def content_type_matches(headers: Mapping[str, str], expected: str) -> bool:
    actual = header(headers, "content-type")
    if actual is None:
        return False
    actual_base, actual_parameters = _media(actual)
    expected_base, expected_parameters = _media(expected)
    if actual_base != expected_base:
        return False
    return all(
        actual_parameters.get(name) == value
        for name, value in expected_parameters.items()
    )


def pointer(value: object, pointer: str) -> object:
    if pointer == "":
        return value
    if not pointer.startswith("/"):
        raise DecodeError(
            "a JSON pointer must be empty or start with '/'", path=pointer
        )
    current = value
    for raw in pointer[1:].split("/"):
        token = raw.replace("~1", "/").replace("~0", "~")
        if isinstance(current, dict):
            if token not in current:
                raise DecodeError("the JSON pointer does not resolve", path=pointer)
            current = current[token]
            continue
        if isinstance(current, list):
            if not token.isdigit():
                raise DecodeError("the JSON pointer index is not a number", path=pointer)
            position = int(token)
            if position >= len(current):
                raise DecodeError("the JSON pointer index is out of range", path=pointer)
            current = current[position]
            continue
        raise DecodeError("the JSON pointer does not resolve", path=pointer)
    return current


def set_cookie(headers: Mapping[str, str], name: str) -> str | None:
    value = header(headers, "set-cookie")
    if value is None:
        return None
    separator_text = "\n" if "\n" in value else ", "
    for part in value.split(separator_text):
        pair = part.split(";", 1)[0].strip()
        key, separator, item = pair.partition("=")
        if separator and key.strip() == name:
            return item.strip()
    return None


def locator_value(
    locator: tuple[str, str], headers: Mapping[str, str], body: bytes
) -> object:
    kind, name = locator
    if kind == "body":
        if not body:
            raise DecodeError("the response body is empty", path=name)
        try:
            document = json.loads(body)
        except (ValueError, UnicodeDecodeError) as error:
            raise DecodeError(str(error), path=name) from error
        return pointer(document, name)
    if kind == "header":
        value = header(headers, name)
        if value is None:
            raise DecodeError("missing header", path=name)
        return value
    if kind == "set_cookie":
        value = set_cookie(headers, name)
        if value is None:
            raise DecodeError("missing cookie", path=name)
        return value
    raise DecodeError(f"unknown credential locator {kind!r}", path=name)


def provide_value(
    locator: tuple[str, str], headers: Mapping[str, str], body: bytes
) -> str:
    value = locator_value(locator, headers, body)
    if not isinstance(value, str):
        raise DecodeError("the provided credential is not a string", path=locator[1])
    return value


def _stream_text(chunks: Iterable[bytes]) -> Iterator[str]:
    for chunk in chunks:
        yield chunk.decode("utf-8")


def sse_events(chunks: Iterable[bytes]) -> Iterator[tuple[str, object]]:
    event = ""
    data: list[str] = []
    buffer = ""
    for text in _stream_text(chunks):
        buffer += text
        while "\n" in buffer:
            line, buffer = buffer.split("\n", 1)
            line = line.rstrip("\r")
            if not line:
                if data:
                    yield event or "message", _stream_json("\n".join(data), "event")
                    event = ""
                    data = []
                continue
            if line.startswith(":"):
                continue
            field, _, value = line.partition(":")
            if value.startswith(" "):
                value = value[1:]
            if field == "event":
                event = value
            elif field == "data":
                data.append(value)
    if buffer.strip():
        field, _, value = buffer.partition(":")
        if value.startswith(" "):
            value = value[1:]
        if field == "data":
            data.append(value)
    if data:
        yield event or "message", _stream_json("\n".join(data), "event")


def ndjson(chunks: Iterable[bytes]) -> Iterator[object]:
    buffer = ""
    for text in _stream_text(chunks):
        buffer += text
        while "\n" in buffer:
            line, buffer = buffer.split("\n", 1)
            line = line.strip()
            if line:
                yield _stream_json(line, "item")
    if buffer.strip():
        yield _stream_json(buffer.strip(), "item")


def _stream_json(text: str, path: str) -> object:
    try:
        return json.loads(text)
    except (ValueError, UnicodeDecodeError) as error:
        raise DecodeError(str(error), path=path) from error
