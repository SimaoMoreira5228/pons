"""Render `validators.py` for the generated package."""

from __future__ import annotations

from .analysis import Analysis
from .render import literal, start

RUNTIME = r"""
import ipaddress
import math
import re
from dataclasses import dataclass
from decimal import Decimal, InvalidOperation

from . import _wire
from . import types as _types

_UUID = re.compile(r"^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$")
_DATE = re.compile(r"^(\d{4})-(\d{2})-(\d{2})$")
_TIME = re.compile(r"^(\d{2}):(\d{2}):(\d{2})(?:\.\d+)?(?:[Zz]|([+-])(\d{2}):(\d{2}))$")
_DATE_TIME = re.compile(r"^(\d{4})-(\d{2})-(\d{2})[Tt](\d{2}):(\d{2}):(\d{2})(?:\.\d{1,3})?(?:[Zz]|([+-])(\d{2}):(\d{2}))$")
_DURATION = re.compile(r"^P(?:(?:\d+W)|(?=\d|T\d)(?:\d+Y)?(?:\d+M)?(?:\d+D)?(?:T(?=\d)(?:\d+H)?(?:\d+M)?(?:\d+(?:[.,]\d+)?S)?)?)$")
_EMAIL = re.compile(r"^[A-Za-z0-9_'+\-]+(?:\.[A-Za-z0-9_'+\-]+)*@(?:[A-Za-z0-9](?:[A-Za-z0-9\-]*[A-Za-z0-9])?\.)+[A-Za-z]{2,}$")
_URI = re.compile(r"^[A-Za-z][A-Za-z0-9+.-]*:")
_FLOAT = re.compile(r"^-?(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][+-]?\d+)?$")
_INT = re.compile(r"^-?(?:0|[1-9][0-9]*)$")


@dataclass(frozen=True, slots=True)
class Issue:
    path: str
    message: str


def _issue(path: str, message: str) -> Issue:
    return Issue(path, message)


def _validate(schema: dict, value: object, path: str) -> Issue | None:
    if schema is None:
        return None
    if "ref" in schema:
        return _validate_named(schema, value, path)
    kind = schema.get("kind")
    if kind == "any":
        return None
    if kind == "nullable":
        return None if value is None else _validate(schema["inner"], value, path)
    if kind == "bool":
        return None if isinstance(value, bool) else _issue(path, "expected a boolean")
    if kind == "int":
        return _check_int(schema, value, path)
    if kind == "float":
        return _check_float(schema, value, path)
    if kind == "decimal":
        return None if isinstance(value, Decimal) else _issue(path, "expected a decimal")
    if kind == "bytes":
        return None if isinstance(value, bytes) else _issue(path, "expected bytes")
    if kind == "string":
        return _check_string(schema, value, path)
    if kind == "array":
        return _validate_array(schema, value, path)
    if kind == "map":
        return _validate_map(schema, value, path)
    if kind == "tuple":
        return _validate_tuple(schema, value, path)
    if kind == "struct":
        return _validate_struct(schema, value, path)
    if kind == "newtype":
        return _validate(schema["inner"], value, path)
    return _issue(path, f"unknown type kind {kind}")


def _validate_named(schema: dict, value: object, path: str) -> Issue | None:
    name = schema["ref"]
    target = _TYPES.get(name)
    if target is None:
        return _issue(path, f"unknown type {name}")
    kind = target.get("kind")
    if kind == "struct":
        return _validate_struct(target, value, path)
    if kind == "enum":
        return _validate_enum(target, value, path, schema.get("variant"))
    if kind == "newtype":
        try:
            found = value.value
        except AttributeError:
            return _issue(path, "expected a newtype")
        return _validate(target.get("inner"), found, path)
    return _issue(path, f"unknown type kind {kind}")


def _validate_struct(schema: dict, value: object, path: str) -> Issue | None:
    inline = bool(schema.get("inline"))
    if inline and not isinstance(value, dict):
        return _issue(path, "expected an object")
    for wire, attribute, field_schema, optional, nullable in schema.get("fields", []):
        if inline:
            found = value.get(wire, _wire.MISSING)
        else:
            if not hasattr(value, attribute):
                return _issue(path, "expected an object")
            found = getattr(value, attribute)
        if found is _wire.MISSING:
            if optional:
                continue
            return _issue(_wire.child(path, wire), "missing field")
        if found is None and (optional or nullable):
            continue
        result = _validate(field_schema, found, _wire.child(path, wire))
        if result is not None:
            return result
    if schema.get("openness") == "preserve_typed":
        extra_schema = schema.get("extra")
        extra = getattr(value, schema.get("extra_attr", "extra"), None)
        if extra_schema is not None and isinstance(extra, dict):
            for key, item in extra.items():
                result = _validate(extra_schema, item, _wire.child(path, key))
                if result is not None:
                    return result
    return None


def _validate_enum(schema: dict, value: object, path: str, expected: str | None) -> Issue | None:
    for wire, class_name, payload in schema.get("variants", []):
        variant = getattr(_types, class_name, None)
        if variant is not None and isinstance(value, variant):
            if expected is not None and wire != expected:
                return _issue(path, f"expected the {expected} variant")
            return _validate_payload(payload, value, path)
    return _issue(path, "value is not a known enum variant")


def _validate_payload(payload: dict, value: object, path: str) -> Issue | None:
    kind = payload.get("kind")
    if kind == "unit":
        return None
    if kind == "newtype":
        try:
            found = value.value
        except AttributeError:
            return _issue(path, "expected an enum payload")
        return _validate(payload.get("ty"), found, path)
    if kind == "tuple":
        try:
            found = value.value
        except AttributeError:
            return _issue(path, "expected an enum payload")
        items = payload.get("items", [])
        if not isinstance(found, (tuple, list)) or len(found) != len(items):
            return _issue(path, f"expected a tuple of {len(items)} items")
        for position, (item, item_schema) in enumerate(zip(found, items)):
            result = _validate(item_schema, item, _wire.index(path, position))
            if result is not None:
                return result
        return None
    if kind == "struct":
        return _validate_struct(payload, value, path)
    return None


def _validate_array(schema: dict, value: object, path: str) -> Issue | None:
    if not isinstance(value, list):
        return _issue(path, "expected an array")
    item_schema = schema.get("item")
    for position, item in enumerate(value):
        result = _validate(item_schema, item, _wire.index(path, position))
        if result is not None:
            return result
    if schema.get("unique"):
        for position, item in enumerate(value):
            for other in value[:position]:
                if item == other:
                    return _issue(_wire.index(path, position), "array items must be unique")
    return None


def _validate_map(schema: dict, value: object, path: str) -> Issue | None:
    if not isinstance(value, dict):
        return _issue(path, "expected an object")
    value_schema = schema.get("value")
    for key, item in value.items():
        result = _validate(value_schema, item, _wire.child(path, key))
        if result is not None:
            return result
    return None


def _validate_tuple(schema: dict, value: object, path: str) -> Issue | None:
    if not isinstance(value, (tuple, list)):
        return _issue(path, "expected a tuple")
    items = schema.get("items", [])
    if len(value) != len(items):
        return _issue(path, f"expected a tuple of {len(items)} items")
    for position, (item, item_schema) in enumerate(zip(value, items)):
        result = _validate(item_schema, item, _wire.index(path, position))
        if result is not None:
            return result
    return None


def _check_string(schema: dict, value: object, path: str) -> Issue | None:
    if not isinstance(value, str):
        return _issue(path, "expected a string")
    format = schema.get("format")
    if format is not None and not _check_format(format, value):
        return _issue(path, _format_message(format))
    length = len(value)
    minimum = schema.get("min_length")
    if minimum is not None and length < minimum:
        return _issue(path, "string is shorter than the contract")
    maximum = schema.get("max_length")
    if maximum is not None and length > maximum:
        return _issue(path, "string is longer than the contract")
    pattern = schema.get("pattern")
    if pattern is not None:
        try:
            if re.search(pattern, value) is None:
                return _issue(path, "string does not match its pattern")
        except re.error as error:
            return _issue(path, f"string pattern cannot be evaluated: {error}")
    return None


def _check_format(format: str, value: str) -> bool:
    if format == "uuid":
        return _UUID.fullmatch(value) is not None
    if format == "date":
        match = _DATE.fullmatch(value)
        return match is not None and _calendar_day(match.group(1), match.group(2), match.group(3))
    if format == "time":
        match = _TIME.fullmatch(value)
        return (
            match is not None
            and _clock(match.group(1), match.group(2), match.group(3))
            and _offset(match.group(4), match.group(5), match.group(6))
        )
    if format == "date-time":
        match = _DATE_TIME.fullmatch(value)
        return (
            match is not None
            and _calendar_day(match.group(1), match.group(2), match.group(3))
            and _clock(match.group(4), match.group(5), match.group(6))
            and _offset(match.group(7), match.group(8), match.group(9))
        )
    if format == "duration":
        return _DURATION.fullmatch(value) is not None
    if format == "uri":
        return _uri(value)
    if format == "email":
        return _EMAIL.fullmatch(value) is not None
    if format == "ipv4":
        return _address(value, 4)
    if format == "ipv6":
        return _address(value, 6)
    return False


def _format_message(format: str) -> str:
    return f"expected a {format}"


def _calendar_day(year: str, month: str, day: str) -> bool:
    month_number = int(month)
    day_number = int(day)
    if month_number < 1 or month_number > 12 or day_number < 1:
        return False
    return day_number <= _days_in_month(int(year), month_number)


def _days_in_month(year: int, month: int) -> int:
    if month == 2:
        leap = year % 4 == 0 and (year % 100 != 0 or year % 400 == 0)
        return 29 if leap else 28
    return 30 if month in (4, 6, 9, 11) else 31


def _clock(hour: str, minute: str, second: str) -> bool:
    return int(hour) <= 23 and int(minute) <= 59 and int(second) <= 59


def _offset(sign: str | None, hour: str | None, minute: str | None) -> bool:
    if sign is None:
        return True
    return int(hour) <= 23 and int(minute) <= 59


def _uri(value: str) -> bool:
    if any(character.isspace() for character in value):
        return False
    return _URI.match(value) is not None


def _address(value: str, version: int) -> bool:
    try:
        address = ipaddress.ip_address(value)
    except ValueError:
        return False
    return address.version == version


def _check_int(schema: dict, value: object, path: str) -> Issue | None:
    if isinstance(value, bool) or not isinstance(value, int):
        return _issue(path, "expected an integer")
    bits = schema.get("bits")
    signed = bool(schema.get("signed", True))
    if isinstance(bits, int):
        low = -(1 << (bits - 1)) if signed else 0
        high = (1 << (bits - 1)) - 1 if signed else (1 << bits) - 1
        if value < low or value > high:
            return _issue(path, "integer is outside its range")
    minimum = schema.get("minimum")
    if minimum is not None:
        bound = _integer_bound(minimum)
        if bound is None:
            return _issue(path, "integer minimum cannot be read")
        if value < bound:
            return _issue(path, "integer is below its minimum")
    maximum = schema.get("maximum")
    if maximum is not None:
        bound = _integer_bound(maximum)
        if bound is None:
            return _issue(path, "integer maximum cannot be read")
        if value > bound:
            return _issue(path, "integer is above its maximum")
    multiple = schema.get("multiple_of")
    if multiple is not None:
        divisor = _integer_bound(multiple)
        if divisor is None or divisor <= 0:
            return _issue(path, "integer multiple_of cannot be read")
        if value % divisor != 0:
            return _issue(path, "integer is not a multiple")
    return None


def _integer_bound(value: object) -> int | None:
    if isinstance(value, bool):
        return None
    if isinstance(value, int):
        return value
    if isinstance(value, float) and value.is_integer():
        return int(value)
    if isinstance(value, str) and _INT.fullmatch(value) is not None:
        return int(value)
    return None


def _check_float(schema: dict, value: object, path: str) -> Issue | None:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        return _issue(path, "expected a number")
    number = float(value)
    if not math.isfinite(number):
        return _issue(path, "expected a finite number")
    if schema.get("bits") == 32 and abs(number) > 3.4028234663852886e38:
        return _issue(path, "number is outside the f32 range")
    minimum = schema.get("minimum")
    if minimum is not None:
        bound = _float_bound(minimum)
        if bound is None:
            return _issue(path, "float minimum cannot be read")
        if number < bound:
            return _issue(path, "number is below its minimum")
    maximum = schema.get("maximum")
    if maximum is not None:
        bound = _float_bound(maximum)
        if bound is None:
            return _issue(path, "float maximum cannot be read")
        if number > bound:
            return _issue(path, "number is above its maximum")
    exclusive_minimum = schema.get("exclusive_minimum")
    if exclusive_minimum is not None:
        bound = _float_bound(exclusive_minimum)
        if bound is None:
            return _issue(path, "float exclusive minimum cannot be read")
        if number <= bound:
            return _issue(path, "number is not above its exclusive minimum")
    exclusive_maximum = schema.get("exclusive_maximum")
    if exclusive_maximum is not None:
        bound = _float_bound(exclusive_maximum)
        if bound is None:
            return _issue(path, "float exclusive maximum cannot be read")
        if number >= bound:
            return _issue(path, "number is not below its exclusive maximum")
    multiple = schema.get("multiple_of")
    if multiple is not None:
        divisor = _float_bound(multiple)
        if divisor is None or divisor <= 0:
            return _issue(path, "float multiple_of cannot be read")
        quotient = number / divisor
        tolerance = 1e-9 * max(1.0, abs(quotient))
        if abs(quotient - round(quotient)) > tolerance:
            return _issue(path, "number is not a multiple")
    return None


def _float_bound(value: object) -> float | None:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        return None
    number = float(value)
    return number if math.isfinite(number) else None


def _validate_header(schema: dict, value: object, path: str) -> Issue | None:
    if schema is None:
        return None
    if not isinstance(value, str):
        return _issue(path, "expected a header value")
    if "ref" in schema:
        target = _TYPES.get(schema["ref"])
        if target is None:
            return _issue(path, f"unknown type {schema['ref']}")
        kind = target.get("kind")
        if kind == "newtype":
            return _validate_header(target.get("inner"), value, path)
        if kind == "enum":
            if target.get("repr") != "string":
                return _issue(path, "the header enum is not a string enum")
            expected = schema.get("variant")
            for wire, _class_name, _payload in target.get("variants", []):
                if value == wire:
                    if expected is not None and wire != expected:
                        return _issue(path, f"expected the {expected} variant")
                    return None
            return _issue(path, "the header is not a known enum variant")
        return _issue(path, "the header type cannot be validated")
    kind = schema.get("kind")
    if kind == "string":
        return _check_string(schema, value, path)
    if kind == "bool":
        return None if value in ("true", "false") else _issue(path, "expected a boolean")
    if kind == "int":
        return _check_int_text(schema, value, path)
    if kind == "float":
        return _check_float_text(schema, value, path)
    if kind == "decimal":
        return _check_decimal_text(value, path)
    if kind == "nullable":
        return _validate_header(schema.get("inner"), value, path)
    return _issue(path, "the header type cannot be validated")


def _check_int_text(schema: dict, value: str, path: str) -> Issue | None:
    if _INT.fullmatch(value) is None:
        return _issue(path, "expected an integer header")
    if not bool(schema.get("signed", True)) and value.startswith("-"):
        return _issue(path, "expected an unsigned integer header")
    return _check_int(schema, int(value), path)


def _check_float_text(schema: dict, value: str, path: str) -> Issue | None:
    if _FLOAT.fullmatch(value) is None:
        return _issue(path, "expected a number")
    return _check_float(schema, float(value), path)


def _check_decimal_text(value: str, path: str) -> Issue | None:
    try:
        number = Decimal(value)
    except InvalidOperation as error:
        return _issue(path, f"expected a decimal: {error}")
    if not number.is_finite():
        return _issue(path, "expected a finite decimal")
    return None
"""


def render_validators(analysis: Analysis) -> str:
    plan = analysis.validation_plan
    assert plan is not None
    lines = [start(f"Response validators for {analysis.document.title}.")]
    lines.append("from __future__ import annotations")
    lines.append("")
    lines.extend(RUNTIME.strip("\n").splitlines())
    lines.append("")
    lines.append("_TYPES: dict[str, dict] = {")
    for type_id in sorted(plan.type_schemas):
        lines.append(f"    {literal(type_id)}: {literal(plan.type_schemas[type_id])},")
    lines.append("}")
    lines.append("")
    for type_plan in analysis.types:
        name = plan.type_functions[type_plan.id]
        lines.append(f"def {name}(value: object, path: str = \"\") -> Issue | None:")
        lines.append(
            f"    return _validate({literal({'ref': type_plan.id})}, value, path)"
        )
        lines.append("")
    for rendered in plan.rendered:
        target = "_validate_header" if rendered.header else "_validate"
        lines.append(f"def {rendered.name}(value: object, path: str = \"\") -> Issue | None:")
        lines.append(f"    return {target}({literal(rendered.schema)}, value, path)")
        lines.append("")
    return "\n".join(lines).rstrip() + "\n"
