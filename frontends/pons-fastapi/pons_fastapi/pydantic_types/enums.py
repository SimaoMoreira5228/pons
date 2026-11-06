from __future__ import annotations

import enum
from typing import Any, get_args

from ..diagnostics import diagnostic
from ..naming import ident, split_words, type_base_id, type_name
from .constraints import _int_bound
from .directions import Direction, direction_for


def _is_enum(value: Any) -> bool:
    return isinstance(value, type) and issubclass(value, enum.Enum)


class EnumTypes:
    def _ensure_enum(self, enum_type: type[enum.Enum]) -> None:
        type_id = type_base_id(enum_type)
        if type_id in self._defs:
            return
        variants = []
        for member in enum_type:
            variants.append(
                {
                    "wire": str(member.value),
                    "ident": ident(member.name),
                    "payload": {"kind": "unit"},
                }
            )
        self._defs[type_id] = {
            "kind": "enum",
            "name": type_name(enum_type),
            "direction": direction_for(self._usage[enum_type]),
            "repr": {"kind": "string"},
            "variants": variants,
            "origin": type_id,
        }

    def _enum_supported(self, enum_type: type[enum.Enum]) -> bool:
        values: set[str] = set()
        try:
            members = list(enum_type)
        except TypeError:
            return False
        if not members:
            return False
        for member in members:
            if not isinstance(member.value, str) or member.value in values:
                return False
            values.add(member.value)
        return True

    def _literal(
        self,
        annotation: Any,
        direction: Direction,
        site: str,
    ) -> tuple[dict[str, Any], list[dict[str, Any]]]:
        values = get_args(annotation)
        if values and all(isinstance(value, str) for value in values):
            if len(set(values)) != len(values):
                return {"kind": "any"}, [
                    diagnostic(
                        "fastapi.literal",
                        f"{site} repeats a string literal; it is recorded as any",
                    )
                ]
            return {"ref": self._ensure_literal(site, values, direction)}, []
        if values and all(isinstance(value, bool) for value in values):
            return {"kind": "bool"}, [
                diagnostic(
                    "fastapi.literal",
                    f"{site} is limited to specific boolean literals, which IR bool cannot express",
                )
            ]
        if len(values) == 1 and isinstance(values[0], int) and not isinstance(values[0], bool):
            bits = 64
            bound = _int_bound(values[0], bits, True)
            if bound is None:
                return {"kind": "int", "bits": bits, "signed": True, "wire": "number"}, [
                    diagnostic(
                        "fastapi.literal",
                        f"{site} is an integer literal outside the 64-bit range; its value is omitted",
                    )
                ]
            return {
                "kind": "int",
                "bits": bits,
                "signed": True,
                "wire": "number",
                "minimum": bound,
                "maximum": bound,
            }, []
        return {"kind": "any"}, [
            diagnostic(
                "fastapi.literal",
                f"{site} mixes literal values that IR cannot enumerate; it is recorded as any",
            )
        ]

    def _ensure_literal(
        self,
        site: str,
        values: tuple[Any, ...],
        direction: Direction,
    ) -> str:
        type_id = f"{site}::literal"
        self._literal_usage[type_id].add(direction)
        existing = self._literal_defs.get(type_id)
        if existing is not None:
            return type_id
        name = split_words(site.rsplit(".", 1)[-1]) or ["literal"]
        variants = [
            {
                "wire": str(value),
                "ident": ident(str(value), "empty"),
                "payload": {"kind": "unit"},
            }
            for value in values
        ]
        self._literal_defs[type_id] = {
            "kind": "enum",
            "name": name + ["literal"],
            "direction": direction,
            "repr": {"kind": "string"},
            "variants": variants,
            "origin": site,
        }
        self._defs[type_id] = self._literal_defs[type_id]
        return type_id
