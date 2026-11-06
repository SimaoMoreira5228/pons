from __future__ import annotations

import enum
import typing
from collections import defaultdict
from typing import Annotated, Any, Literal, get_args, get_origin

from pydantic import BaseModel

from ..config import PonsConfigurationError, PonsType
from ..diagnostics import diagnostic
from ..naming import type_base_id
from .constraints import _flatten_field_info
from .containers import ContainerTypes
from .directions import BOTH, INPUT, OUTPUT, Direction, direction_for
from .enums import EnumTypes, _is_enum
from .models import ModelTypes, _excluded, _is_model
from .scalars import ScalarTypes
from .unions import UnionTypes, _NONE, _is_union


class TypeRegistry(
    ContainerTypes,
    EnumTypes,
    ModelTypes,
    ScalarTypes,
    UnionTypes,
):
    def __init__(self) -> None:
        self._usage: dict[Any, set[Direction]] = defaultdict(set)
        self._models: set[type[BaseModel]] = set()
        self._enums: set[type[enum.Enum]] = set()
        self._split: set[type[BaseModel]] = set()
        self._defs: dict[str, dict[str, Any]] = {}
        self._diagnostics: list[dict[str, Any]] = []
        self._literal_usage: dict[str, set[Direction]] = defaultdict(set)
        self._literal_defs: dict[str, dict[str, Any]] = {}

    def diagnostics(self) -> list[dict[str, Any]]:
        return list(self._diagnostics)

    def collect(self, annotation: Any, direction: Direction) -> None:
        annotation, _ = self._annotated(annotation)
        if annotation is None or annotation is _NONE:
            return
        if annotation is Any:
            return
        if _is_model(annotation):
            if direction in self._usage[annotation]:
                return
            self._models.add(annotation)
            self._usage[annotation].add(direction)
            fields = getattr(annotation, "model_fields", {})
            for field in fields.values():
                if direction == OUTPUT and _excluded(field):
                    continue
                self.collect(field.annotation, direction)
            if direction == OUTPUT:
                for computed in getattr(annotation, "model_computed_fields", {}).values():
                    self.collect(getattr(computed, "return_type", Any), direction)
            return
        if _is_enum(annotation):
            self._enums.add(annotation)
            self._usage[annotation].add(direction)
            return
        origin = get_origin(annotation)
        if _is_union(origin):
            arguments = [argument for argument in get_args(annotation) if argument is not _NONE]
            if len(arguments) == 1:
                self.collect(arguments[0], direction)
            return
        if origin is Literal:
            return
        if origin in (list, set, frozenset):
            arguments = get_args(annotation)
            if arguments:
                self.collect(arguments[0], direction)
            return
        if origin is tuple:
            for argument in get_args(annotation):
                if argument is not Ellipsis:
                    self.collect(argument, direction)
            return
        if origin is dict:
            arguments = get_args(annotation)
            if len(arguments) == 2:
                self.collect(arguments[0], direction)
                self.collect(arguments[1], direction)
            return

    def analyze(self) -> None:
        split: set[type[BaseModel]] = set()
        for model in self._models:
            if self._usage[model] == {INPUT, OUTPUT} and self._own_shape_differs(model):
                split.add(model)
        changed = True
        while changed:
            changed = False
            for model in self._models:
                if model in split or self._usage[model] != {INPUT, OUTPUT}:
                    continue
                if any(
                    nested in split for nested in self._referenced_models(model)
                ):
                    split.add(model)
                    changed = True
        self._split = split

    def lower(
        self,
        annotation: Any,
        direction: Direction,
        *,
        constraints: list[Any] | None = None,
        site: str = "root",
        config: dict[str, Any] | None = None,
    ) -> tuple[dict[str, Any], list[dict[str, Any]]]:
        self.collect(annotation, direction)
        self.analyze()
        self.build()
        return self.reference(
            annotation,
            direction,
            constraints=constraints,
            site=site,
            config=config,
        )

    def build(self) -> None:
        for model in sorted(self._models, key=type_base_id):
            for direction in sorted(self._usage[model]):
                self._ensure_model(model, direction)
        for enum_type in sorted(self._enums, key=type_base_id):
            self._ensure_enum(enum_type)

    def definitions(self) -> dict[str, dict[str, Any]]:
        self._sync_directions()
        return dict(self._defs)

    def define_struct(
        self,
        type_id: str,
        name: list[str],
        fields: list[dict[str, Any]],
        *,
        direction: Direction = INPUT,
        openness: Any = "reject",
        origin: str | None = None,
    ) -> None:
        if type_id in self._defs:
            return
        self._defs[type_id] = {
            "kind": "struct",
            "name": name,
            "direction": direction,
            "openness": openness,
            "origin": origin or type_id,
            "fields": fields,
        }

    def reference(
        self,
        annotation: Any,
        direction: Direction,
        *,
        constraints: list[Any] | None = None,
        site: str = "root",
        config: dict[str, Any] | None = None,
    ) -> tuple[dict[str, Any], list[dict[str, Any]]]:
        constraints = _flatten_field_info(list(constraints or []))
        annotation, metadata = self._annotated(annotation)
        constraints = _flatten_field_info(constraints + list(metadata))
        diagnostics: list[dict[str, Any]] = []
        override = next(
            (constraint for constraint in constraints if isinstance(constraint, PonsType)),
            None,
        )
        if override is not None:
            if _is_union(get_origin(annotation)):
                raise PonsConfigurationError(
                    f"{site} wraps a union with pons_type; annotate the non-null type instead"
                )
            return self._type_override(override, site), diagnostics

        if annotation is None or annotation is _NONE:
            return {"kind": "null"}, diagnostics
        if annotation is Any or annotation is object:
            return {"kind": "any"}, diagnostics
        if annotation is Ellipsis:
            return {"kind": "any"}, diagnostics
        if isinstance(annotation, typing.ForwardRef):
            diagnostics.append(
                diagnostic(
                    "fastapi.unresolved_forward_ref",
                    f"{site} still refers to {annotation.__forward_arg__}; it is recorded as any",
                )
            )
            return {"kind": "any"}, diagnostics

        origin = get_origin(annotation)
        if origin is Literal:
            return self._literal(annotation, direction, site)
        if _is_union(origin):
            return self._union(annotation, direction, constraints, site, config, diagnostics)
        if origin in (list, set, frozenset):
            return self._array(
                annotation, origin, direction, constraints, site, config, diagnostics
            )
        if origin is tuple:
            return self._tuple(annotation, direction, site, config, diagnostics)
        if origin is dict:
            return self._map(annotation, direction, site, config, diagnostics)
        if origin is not None:
            diagnostics.append(
                diagnostic(
                    "fastapi.unsupported_type",
                    f"{site} uses {annotation!r}, which Pons does not describe; it is recorded as any",
                )
            )
            return {"kind": "any"}, diagnostics

        if _is_model(annotation):
            return {"ref": self._ensure_model(annotation, direction)}, diagnostics
        if _is_enum(annotation):
            if not self._enum_supported(annotation):
                diagnostics.append(
                    diagnostic(
                        "fastapi.unsupported_enum",
                        f"{site} uses {annotation.__name__}, whose values are not unique strings; it is recorded as any",
                    )
                )
                return {"kind": "any"}, diagnostics
            self._ensure_enum(annotation)
            return {"ref": type_base_id(annotation)}, diagnostics
        scalar = self._scalar(annotation, constraints, diagnostics, site, config)
        if scalar is not None:
            return scalar, diagnostics

        diagnostics.append(
            diagnostic(
                "fastapi.unsupported_type",
                f"{site} uses {annotation!r}, which Pons does not describe; it is recorded as any",
            )
        )
        return {"kind": "any"}, diagnostics

    def _annotated(self, annotation: Any) -> tuple[Any, tuple[Any, ...]]:
        if get_origin(annotation) is Annotated:
            arguments = get_args(annotation)
            return arguments[0], tuple(arguments[1:])
        return annotation, ()

    def _sync_directions(self) -> None:
        for model in self._models:
            if model in self._split:
                continue
            definition = self._defs.get(type_base_id(model))
            if definition is not None:
                definition["direction"] = direction_for(self._usage[model])
        for enum_type in self._enums:
            definition = self._defs.get(type_base_id(enum_type))
            if definition is not None:
                definition["direction"] = direction_for(self._usage[enum_type])
        for type_id, definition in self._literal_defs.items():
            definition["direction"] = direction_for(self._literal_usage[type_id])
