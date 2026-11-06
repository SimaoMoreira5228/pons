from __future__ import annotations

import datetime as dt
import enum
import ipaddress
import pathlib
import types
import typing
from collections import defaultdict
from decimal import Decimal
from typing import Annotated, Any, Literal, Union, get_args, get_origin
from uuid import UUID

from pydantic import BaseModel
from pydantic.fields import FieldInfo
from pydantic_core import PydanticUndefined, to_jsonable_python

from .config import PonsConfigurationError, PonsType
from .diagnostics import diagnostic
from .naming import ident, split_words, type_base_id, type_name

INPUT = "input"
OUTPUT = "output"
BOTH = "both"
Direction = Literal["input", "output", "both"]

_NONE = type(None)
_MISSING = object()


def direction_for(usage: set[Direction]) -> Direction:
    if INPUT in usage and OUTPUT in usage:
        return BOTH
    if OUTPUT in usage:
        return OUTPUT
    return INPUT


def _is_model(value: Any) -> bool:
    return isinstance(value, type) and issubclass(value, BaseModel)


def _is_enum(value: Any) -> bool:
    return isinstance(value, type) and issubclass(value, enum.Enum)


def _is_union(origin: Any) -> bool:
    return origin is Union or origin is types.UnionType


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


def _int_bound(value: Any, bits: int, signed: bool) -> int | float | None:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        return None
    low = -(1 << (bits - 1)) if signed else 0
    high = (1 << (bits - 1)) - 1 if signed else (1 << bits) - 1
    return value if low <= value <= high else None


class TypeRegistry:
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
        if origin in (list, set, frozenset):
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
        if origin is tuple:
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
        if origin is dict:
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
        if annotation is bool:
            return {"kind": "bool"}, diagnostics
        if annotation is int:
            return self._integer(constraints, diagnostics, site), diagnostics
        if annotation is float:
            if any(
                _constraint(constraints, name) is not None
                for name in ("ge", "le", "gt", "lt")
            ):
                diagnostics.append(
                    diagnostic(
                        "fastapi.float_constraint",
                        f"{site} carries a numeric bound that IR floats cannot express",
                    )
                )
            return {"kind": "float", "bits": 64}, diagnostics
        if annotation is str:
            return self._string(constraints), diagnostics
        if annotation is bytes:
            return self._bytes(config, diagnostics, site), diagnostics
        if annotation is dt.datetime:
            return {"kind": "string", "format": "date-time"}, diagnostics
        if annotation is dt.date:
            return {"kind": "string", "format": "date"}, diagnostics
        if annotation is dt.time:
            return {"kind": "string", "format": "time"}, diagnostics
        if annotation is dt.timedelta:
            return {"kind": "string", "format": "duration"}, diagnostics
        if annotation is UUID:
            return {"kind": "string", "format": "uuid"}, diagnostics
        if annotation is Decimal:
            return {"kind": "decimal", "wire": "string"}, diagnostics
        if isinstance(annotation, type) and issubclass(annotation, pathlib.PurePath):
            return {"kind": "string"}, diagnostics
        if annotation is ipaddress.IPv4Address:
            return {"kind": "string", "format": "ipv4"}, diagnostics
        if annotation is ipaddress.IPv6Address:
            return {"kind": "string", "format": "ipv6"}, diagnostics

        special = self._pydantic_special(annotation)
        if special is not None:
            return special, diagnostics

        diagnostics.append(
            diagnostic(
                "fastapi.unsupported_type",
                f"{site} uses {annotation!r}, which Pons does not describe; it is recorded as any",
            )
        )
        return {"kind": "any"}, diagnostics

    def _type_override(self, override: PonsType, site: str) -> dict[str, Any]:
        kind = override.kind
        if kind == "bool":
            return {"kind": "bool"}
        if kind == "int":
            bits = override.bits if override.bits is not None else 64
            if bits not in {8, 16, 32, 64, 128}:
                raise PonsConfigurationError(f"{site} pons_type int needs bits 8, 16, 32, 64, or 128")
            wire = override.wire if override.wire is not None else "number"
            if wire not in {"number", "string"}:
                raise PonsConfigurationError(f"{site} pons_type int wire must be number or string")
            return {
                "kind": "int",
                "bits": bits,
                "signed": True if override.signed is None else override.signed,
                "wire": wire,
            }
        if kind == "float":
            bits = override.bits if override.bits is not None else 64
            if bits not in {32, 64}:
                raise PonsConfigurationError(f"{site} pons_type float needs bits 32 or 64")
            return {"kind": "float", "bits": bits}
        if kind == "decimal":
            wire = override.wire if override.wire is not None else "string"
            if wire not in {"number", "string"}:
                raise PonsConfigurationError(f"{site} pons_type decimal wire must be number or string")
            return {"kind": "decimal", "wire": wire}
        if kind == "string":
            value: dict[str, Any] = {"kind": "string"}
            if override.format is not None:
                if override.format not in {
                    "uuid",
                    "date-time",
                    "date",
                    "time",
                    "duration",
                    "uri",
                    "email",
                    "ipv4",
                    "ipv6",
                }:
                    raise PonsConfigurationError(
                        f"{site} pons_type string format {override.format!r} is not an IR format"
                    )
                value["format"] = override.format
            return value
        if kind == "bytes":
            encoding = override.encoding if override.encoding is not None else "base64"
            if encoding not in {"base64", "base64url", "hex", "array"}:
                raise PonsConfigurationError(f"{site} pons_type bytes encoding {encoding!r} is not an IR encoding")
            return {"kind": "bytes", "encoding": encoding}
        if kind == "any":
            return {"kind": "any"}
        if kind == "opaque":
            wire = override.wire if override.wire is not None else "unknown"
            if wire not in {"json", "text", "bytes", "unknown"}:
                raise PonsConfigurationError(f"{site} pons_type opaque wire {wire!r} is not an IR wire")
            return {"kind": "opaque", "codec": override.codec, "wire": wire}
        raise PonsConfigurationError(f"{site} pons_type kind {kind!r} is not supported")

    def _annotated(self, annotation: Any) -> tuple[Any, tuple[Any, ...]]:
        if get_origin(annotation) is Annotated:
            arguments = get_args(annotation)
            return arguments[0], tuple(arguments[1:])
        return annotation, ()

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

    def _integer(
        self,
        constraints: list[Any],
        diagnostics: list[dict[str, Any]],
        site: str,
    ) -> dict[str, Any]:
        bits = 64
        minimum = _constraint(constraints, "ge")
        maximum = _constraint(constraints, "le")
        greater = _constraint(constraints, "gt")
        less = _constraint(constraints, "lt")
        if isinstance(greater, int) and not isinstance(greater, bool):
            minimum = greater + 1
        if isinstance(less, int) and not isinstance(less, bool):
            maximum = less - 1
        if minimum is not None:
            bound = _int_bound(minimum, bits, True)
            if bound is None:
                diagnostics.append(
                    diagnostic(
                        "fastapi.int_bound",
                        f"{site} has a minimum outside the 64-bit range; it is omitted",
                    )
                )
                minimum = None
            else:
                minimum = bound
        if maximum is not None:
            bound = _int_bound(maximum, bits, True)
            if bound is None:
                diagnostics.append(
                    diagnostic(
                        "fastapi.int_bound",
                        f"{site} has a maximum outside the 64-bit range; it is omitted",
                    )
                )
                maximum = None
            else:
                maximum = bound
        value: dict[str, Any] = {
            "kind": "int",
            "bits": bits,
            "signed": True,
            "wire": "number",
        }
        if minimum is not None:
            value["minimum"] = minimum
        if maximum is not None:
            value["maximum"] = maximum
        multiple_of = _constraint(constraints, "multiple_of")
        if multiple_of is not None:
            value["multiple_of"] = multiple_of
        return value

    def _string(self, constraints: list[Any]) -> dict[str, Any]:
        value: dict[str, Any] = {"kind": "string"}
        minimum = _constraint(constraints, "min_length")
        maximum = _constraint(constraints, "max_length")
        pattern = _pattern(constraints)
        if minimum is not None:
            value["min_length"] = minimum
        if maximum is not None:
            value["max_length"] = maximum
        if pattern is not None:
            value["pattern"] = pattern
        return value

    def _bytes(
        self,
        config: dict[str, Any] | None,
        diagnostics: list[dict[str, Any]],
        site: str,
    ) -> dict[str, Any]:
        encoding = (config or {}).get("ser_json_bytes", "utf8")
        encoding = getattr(encoding, "value", encoding)
        if encoding == "base64":
            return {"kind": "bytes", "encoding": "base64url"}
        if encoding == "hex":
            return {"kind": "bytes", "encoding": "hex"}
        diagnostics.append(
            diagnostic(
                "fastapi.bytes_utf8",
                f"{site} uses Pydantic's default UTF-8 bytes encoding; Pons records it as a string",
            )
        )
        return {"kind": "string"}

    def _pydantic_special(self, annotation: Any) -> dict[str, Any] | None:
        module = getattr(annotation, "__module__", "")
        if not module.startswith("pydantic"):
            return None
        name = getattr(annotation, "__qualname__", getattr(annotation, "__name__", ""))
        if name in {"EmailStr"}:
            return {"kind": "string", "format": "email"}
        if name.endswith("Url") and name not in {"Url"}:
            return {"kind": "string", "format": "uri"}
        if name == "IPvAnyAddress":
            return {"kind": "string"}
        if name in {"SecretStr", "SecretBytes"}:
            return {"kind": "string"}
        return None

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
