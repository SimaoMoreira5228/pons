from __future__ import annotations

from typing import Any, get_args

from ..diagnostics import diagnostic
from .constraints import _constraint
from .directions import Direction


class ContainerTypes:
    def _array(
        self,
        annotation: Any,
        origin: Any,
        direction: Direction,
        constraints: list[Any],
        site: str,
        config: dict[str, Any] | None,
        diagnostics: list[dict[str, Any]],
    ) -> tuple[dict[str, Any], list[dict[str, Any]]]:
        arguments = get_args(annotation)
        item, item_diagnostics = self.reference(
            arguments[0] if arguments else Any,
            direction,
            site=f"{site}[]",
            config=config,
        )
        value: dict[str, Any] = {"kind": "array", "item": item}
        if origin in (set, frozenset):
            value["unique"] = True
        if _constraint(constraints, "min_length") is not None or _constraint(
            constraints, "max_length"
        ) is not None:
            diagnostics.append(
                diagnostic(
                    "fastapi.array_constraint",
                    f"{site} carries a length constraint that IR arrays cannot express",
                )
            )
        return value, item_diagnostics + diagnostics

    def _tuple(
        self,
        annotation: Any,
        direction: Direction,
        site: str,
        config: dict[str, Any] | None,
        diagnostics: list[dict[str, Any]],
    ) -> tuple[dict[str, Any], list[dict[str, Any]]]:
        arguments = get_args(annotation)
        if len(arguments) == 2 and arguments[1] is Ellipsis:
            item, item_diagnostics = self.reference(
                arguments[0],
                direction,
                site=f"{site}[]",
                config=config,
            )
            return {"kind": "array", "item": item}, item_diagnostics + diagnostics
        items: list[dict[str, Any]] = []
        for index, argument in enumerate(arguments):
            item, item_diagnostics = self.reference(
                argument,
                direction,
                site=f"{site}[{index}]",
                config=config,
            )
            items.append(item)
            diagnostics.extend(item_diagnostics)
        return {"kind": "tuple", "items": items}, diagnostics

    def _map(
        self,
        annotation: Any,
        direction: Direction,
        site: str,
        config: dict[str, Any] | None,
        diagnostics: list[dict[str, Any]],
    ) -> tuple[dict[str, Any], list[dict[str, Any]]]:
        arguments = get_args(annotation)
        key = arguments[0] if arguments else str
        value_type = arguments[1] if len(arguments) > 1 else Any
        if key is Any:
            key = str
        key_ref = {"kind": "string"}
        if key is not str:
            diagnostics.append(
                diagnostic(
                    "fastapi.map_key",
                    f"{site} uses {key!r} as a map key; JSON object keys are strings, so the key is recorded as string",
                )
            )
        value, value_diagnostics = self.reference(
            value_type,
            direction,
            site=f"{site}{{}}",
            config=config,
        )
        return {
            "kind": "map",
            "key": key_ref,
            "value": value,
        }, value_diagnostics + diagnostics
