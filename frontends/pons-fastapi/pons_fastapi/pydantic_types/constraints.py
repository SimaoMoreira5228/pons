from __future__ import annotations

from typing import Any

from pydantic.fields import FieldInfo


def _constraint(constraints: list[Any], name: str) -> Any:
    for constraint in constraints:
        value = getattr(constraint, name, None)
        if value is not None:
            return value
    return None


def _pattern(constraints: list[Any]) -> str | None:
    value = _constraint(constraints, "pattern")
    if value is None:
        value = _constraint(constraints, "regex")
    if value is None:
        return None
    return getattr(value, "pattern", value)


def _flatten_field_info(constraints: list[Any]) -> list[Any]:
    flattened: list[Any] = []
    for constraint in constraints:
        if isinstance(constraint, FieldInfo):
            flattened.extend(constraint.metadata)
        else:
            flattened.append(constraint)
    return flattened


def _int_bound(value: Any, bits: int, signed: bool) -> int | float | None:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        return None
    low = -(1 << (bits - 1)) if signed else 0
    high = (1 << (bits - 1)) - 1 if signed else (1 << bits) - 1
    return value if low <= value <= high else None
