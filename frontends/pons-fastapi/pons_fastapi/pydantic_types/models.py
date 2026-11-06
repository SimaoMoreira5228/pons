from __future__ import annotations

from typing import Any, get_args, get_origin

from pydantic import BaseModel
from pydantic.fields import FieldInfo
from pydantic_core import PydanticUndefined, to_jsonable_python

from ..diagnostics import diagnostic
from ..naming import ident, type_base_id, type_name
from .directions import BOTH, INPUT, OUTPUT, Direction, direction_for
from .unions import _NONE, _is_union

_MISSING = object()


def _is_model(value: Any) -> bool:
    return isinstance(value, type) and issubclass(value, BaseModel)


def _alias(value: Any) -> str | None:
    return value if isinstance(value, str) else None


def _field_input_wire(field: FieldInfo, name: str) -> tuple[str, bool]:
    alias = _alias(field.validation_alias) or _alias(field.alias)
    return alias or name, field.validation_alias is not None and _alias(field.validation_alias) is None


def _field_output_wire(field: FieldInfo, name: str) -> tuple[str, bool]:
    alias = _alias(field.serialization_alias) or _alias(field.alias)
    return alias or name, field.serialization_alias is not None and _alias(field.serialization_alias) is None


def _excluded(field: FieldInfo) -> bool:
    return bool(field.exclude)


def _doc(description: str | None = None, deprecated: bool | None = None) -> dict[str, Any] | None:
    value: dict[str, Any] = {}
    if description:
        value["description"] = description
    if deprecated:
        value["deprecated"] = True
    return value or None


class ModelTypes:
    def _own_shape_differs(self, model: type[BaseModel]) -> bool:
        if getattr(model, "model_computed_fields", {}):
            return True
        for name, field in getattr(model, "model_fields", {}).items():
            if not field.is_required() or _excluded(field):
                return True
            input_wire, _ = _field_input_wire(field, name)
            output_wire, _ = _field_output_wire(field, name)
            if input_wire != output_wire:
                return True
        return False

    def _referenced_models(self, model: type[BaseModel]) -> set[type[BaseModel]]:
        found: set[type[BaseModel]] = set()
        for field in getattr(model, "model_fields", {}).values():
            found.update(self._annotation_models(field.annotation))
        for computed in getattr(model, "model_computed_fields", {}).values():
            found.update(self._annotation_models(getattr(computed, "return_type", Any)))
        return found

    def _annotation_models(self, annotation: Any) -> set[type[BaseModel]]:
        annotation, _ = self._annotated(annotation)
        if _is_model(annotation):
            return {annotation}
        origin = get_origin(annotation)
        if _is_union(origin):
            arguments = [argument for argument in get_args(annotation) if argument is not _NONE]
            if len(arguments) == 1:
                return self._annotation_models(arguments[0])
            return set()
        if origin in (list, set, frozenset):
            arguments = get_args(annotation)
            return self._annotation_models(arguments[0]) if arguments else set()
        if origin is tuple:
            found: set[type[BaseModel]] = set()
            for argument in get_args(annotation):
                if argument is not Ellipsis:
                    found.update(self._annotation_models(argument))
            return found
        if origin is dict:
            arguments = get_args(annotation)
            if len(arguments) == 2:
                return self._annotation_models(arguments[1])
        return set()

    def _ensure_model(self, model: type[BaseModel], direction: Direction) -> str:
        base = type_base_id(model)
        split = model in self._split
        if split:
            effective: Direction = OUTPUT if direction == OUTPUT else INPUT
            type_id = f"{base}#{effective}"
        else:
            effective = direction_for(self._usage[model])
            type_id = base
        existing = self._defs.get(type_id)
        if existing is not None:
            return type_id

        openness, openness_diagnostics = self._openness(model, effective)
        definition: dict[str, Any] = {
            "kind": "struct",
            "name": type_name(model),
            "direction": effective,
            "openness": openness,
            "origin": base,
            "fields": [],
        }
        if openness_diagnostics:
            definition["diagnostics"] = openness_diagnostics
        self._defs[type_id] = definition

        fields, diagnostics = self._fields(model, effective, base)
        definition["fields"] = fields
        if diagnostics:
            existing_diagnostics = definition.get("diagnostics", [])
            definition["diagnostics"] = existing_diagnostics + diagnostics
        return type_id

    def _fields(
        self,
        model: type[BaseModel],
        direction: Direction,
        base: str,
    ) -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
        fields: list[dict[str, Any]] = []
        diagnostics: list[dict[str, Any]] = []
        config = dict(getattr(model, "model_config", {}))
        for name, field in getattr(model, "model_fields", {}).items():
            if direction == OUTPUT and _excluded(field):
                continue
            input_wire, input_alias_problem = _field_input_wire(field, name)
            output_wire, output_alias_problem = _field_output_wire(field, name)
            wire = output_wire if direction == OUTPUT else input_wire
            field_diagnostics: list[dict[str, Any]] = []
            if input_alias_problem or output_alias_problem:
                field_diagnostics.append(
                    diagnostic(
                        "fastapi.alias",
                        f"{base}.{name} uses an alias form other than a string; its Python name is used",
                    )
                )
            required = field.is_required()
            presence = "optional" if direction == INPUT and not required else "required"
            ty, type_diagnostics = self.reference(
                field.annotation,
                direction,
                constraints=list(field.metadata),
                site=f"{base}.{name}",
                config=config,
            )
            value: dict[str, Any] = {
                "wire": wire,
                "ident": ident(name),
                "ty": ty,
                "presence": presence,
            }
            if presence == "optional":
                default = self._default(field, field_diagnostics, base, name)
                if default is not _MISSING:
                    value["default"] = default
            doc = _doc(field.description, field.deprecated if isinstance(field.deprecated, bool) else None)
            if doc is not None:
                value["doc"] = doc
            field_diagnostics = type_diagnostics + field_diagnostics
            if field_diagnostics:
                value["diagnostics"] = field_diagnostics
            fields.append(value)

        if direction in (OUTPUT, BOTH):
            for name, computed in getattr(model, "model_computed_fields", {}).items():
                wire = _alias(getattr(computed, "alias", None)) or name
                ty, type_diagnostics = self.reference(
                    getattr(computed, "return_type", Any),
                    OUTPUT,
                    site=f"{base}.{name}",
                    config=config,
                )
                value: dict[str, Any] = {
                    "wire": wire,
                    "ident": ident(name),
                    "ty": ty,
                    "presence": "required",
                }
                doc = _doc(getattr(computed, "description", None))
                if doc is not None:
                    value["doc"] = doc
                if type_diagnostics:
                    value["diagnostics"] = type_diagnostics
                fields.append(value)
        return fields, diagnostics

    def _default(
        self,
        field: FieldInfo,
        diagnostics: list[dict[str, Any]],
        base: str,
        name: str,
    ) -> Any:
        value = field.default
        if value is PydanticUndefined:
            return _MISSING
        try:
            return to_jsonable_python(value)
        except Exception:
            diagnostics.append(
                diagnostic(
                    "fastapi.default",
                    f"{base}.{name} has a default that is not representable as JSON; it is omitted",
                )
            )
            return _MISSING

    def _openness(
        self,
        model: type[BaseModel],
        direction: Direction,
    ) -> tuple[Any, list[dict[str, Any]]]:
        extra = getattr(model, "model_config", {}).get("extra", "ignore")
        extra = getattr(extra, "value", extra)
        if extra == "forbid":
            return "reject", []
        if extra == "allow":
            return "preserve_opaque", []
        if direction != OUTPUT:
            return "reject", [
                diagnostic(
                    "fastapi.extra_ignore",
                    f"{type_base_id(model)} accepts unknown object keys and drops them; Pons records openness as reject",
                )
            ]
        return "reject", []
