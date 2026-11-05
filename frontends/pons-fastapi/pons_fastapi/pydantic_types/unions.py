from __future__ import annotations

import types
from typing import Any, Union, get_args, get_origin

from ..diagnostics import diagnostic
from .directions import Direction

_NONE = type(None)


def _is_union(origin: Any) -> bool:
    return origin is Union or origin is types.UnionType


class UnionTypes:
    def _union(
        self,
        annotation: Any,
        direction: Direction,
        constraints: list[Any],
        site: str,
        config: dict[str, Any] | None,
        diagnostics: list[dict[str, Any]],
    ) -> tuple[dict[str, Any], list[dict[str, Any]]]:
        arguments = [argument for argument in get_args(annotation) if argument is not _NONE]
        if len(arguments) == 1:
            inner, inner_diagnostics = self.reference(
                arguments[0],
                direction,
                constraints=constraints,
                site=site,
                config=config,
            )
            return {"kind": "nullable", "inner": inner}, inner_diagnostics + diagnostics
        diagnostics.append(
            diagnostic(
                "fastapi.unsupported_union",
                f"{site} is a union of {len(arguments)} non-null types; Pons has no general union, so it is recorded as any",
            )
        )
        return {"kind": "any"}, diagnostics
