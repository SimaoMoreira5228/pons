"""Validation levels, coverage, and schema plans for the Python emitter."""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import TYPE_CHECKING, Any

from .codes import VALIDATION
from .document import TypeDef, TypeRef, _decode_field, decode_type_ref
from .plan_operations import ref_signature
from .plan_types import TypePlan, VariantPlan, is_nullable

if TYPE_CHECKING:
    from .analysis import Analysis

EXACT = "exact"
LOSSY = "lossy"
UNSUPPORTED = "unsupported"

_RANK = {EXACT: 0, LOSSY: 1, UNSUPPORTED: 2}


@dataclass(slots=True)
class Rendered:
    name: str
    schema: dict[str, Any]
    header: bool


@dataclass(slots=True)
class ValidationPlan:
    backend: str
    levels: dict[str, str]
    capabilities: dict[str, str]
    type_schemas: dict[str, dict[str, Any]] = field(default_factory=dict)
    type_functions: dict[str, str] = field(default_factory=dict)
    ref_functions: dict[str, str] = field(default_factory=dict)
    header_functions: dict[str, str] = field(default_factory=dict)
    rendered: list[Rendered] = field(default_factory=list)

    def coverage(self) -> dict[str, list[str]]:
        exact: list[str] = []
        lossy: list[str] = []
        unsupported: list[str] = []
        for subject in sorted(self.levels):
            level = self.levels[subject]
            if level == EXACT:
                exact.append(subject)
            elif level == LOSSY:
                lossy.append(subject)
            else:
                unsupported.append(subject)
        return {"exact": exact, "lossy": lossy, "unsupported": unsupported}

    def function_for(self, ref: TypeRef) -> str | None:
        return self.ref_functions.get(ref_signature(ref))

    def header_for(self, ref: TypeRef) -> str | None:
        return self.header_functions.get(ref_signature(ref))

    def problems(self) -> list[Any]:
        from .analysis import Problem

        result: list[Problem] = []
        for subject in sorted(self.levels):
            level = self.levels[subject]
            if level == EXACT:
                continue
            capability = self.capabilities.get(subject, "validation.unsupported")
            if subject.startswith("types."):
                origin = subject[len("types.") :]
            elif subject.startswith("operations."):
                origin = subject[len("operations.") :]
            else:
                origin = subject
            result.append(
                Problem(
                    VALIDATION,
                    f"the {self.backend} validation backend cannot validate {subject} exactly "
                    f"({capability}); choose another validation backend, or run the target in lossy mode",
                    origin,
                    subject,
                )
            )
        return result


class ValidationPlanner:
    def __init__(self, analysis: Analysis, backend: str) -> None:
        self.analysis = analysis
        self.backend = backend
        self.levels: dict[str, str] = {}
        self.capabilities: dict[str, str] = {}
        self.type_schemas: dict[str, dict[str, Any]] = {}
        self.type_functions: dict[str, str] = {}
        self.ref_functions: dict[str, str] = {}
        self.header_functions: dict[str, str] = {}
        self.rendered: list[Rendered] = []
        self._names: set[str] = set()

    def build(self) -> ValidationPlan:
        document = self.analysis.document
        self.levels = {f"types.{type_id}": EXACT for type_id in document.types}
        self.levels.update(
            {
                f"operations.{plan.id}": EXACT
                for plan in self.analysis.operations
            }
        )
        for type_id, definition in document.types.items():
            capability = self.definition_capability(definition)
            if capability is not None:
                self.seed(f"types.{type_id}", UNSUPPORTED, capability)
        for plan in self.analysis.operations:
            capability = self.operation_capability(plan)
            if capability is not None:
                self.seed(f"operations.{plan.id}", UNSUPPORTED, capability)
        self.propagate()
        self.build_types()
        for plan in self.analysis.operations:
            self.register_ref(plan.success.body)
            if plan.stream is not None:
                for event in plan.stream.events:
                    self.register_ref(event.ty)
                if plan.stream.item is not None:
                    self.register_ref(plan.stream.item)
                if plan.stream.client is not None:
                    self.register_ref(plan.stream.client)
                if plan.stream.server is not None:
                    self.register_ref(plan.stream.server)
            for header in plan.success.headers:
                self.register_header(header.ty)
            for error in plan.errors:
                if error.body is not None:
                    self.register_ref(error.body)
                for header in error.headers:
                    self.register_header(header.ty)
        return ValidationPlan(
            backend=self.backend,
            levels=self.levels,
            capabilities=self.capabilities,
            type_schemas=self.type_schemas,
            type_functions=self.type_functions,
            ref_functions=self.ref_functions,
            header_functions=self.header_functions,
            rendered=self.rendered,
        )

    def seed(self, subject: str, level: str, capability: str) -> None:
        if subject not in self.levels:
            return
        if _RANK[level] > _RANK[self.levels[subject]]:
            self.levels[subject] = level
            self.capabilities[subject] = capability

    def propagate(self) -> None:
        changed = True
        while changed:
            changed = False
            for type_id, definition in self.analysis.document.types.items():
                subject = f"types.{type_id}"
                changed |= self.inherit(
                    subject, self.analysis.definition_refs(definition)
                )
            for plan in self.analysis.operations:
                changed |= self.inherit(
                    f"operations.{plan.id}", self.analysis.operation_refs(plan)
                )

    def inherit(self, subject: str, refs: list[TypeRef]) -> bool:
        changed = False
        for ref in refs:
            if ref.id is None:
                continue
            dependency = f"types.{ref.id}"
            if dependency not in self.levels:
                continue
            level = self.levels[dependency]
            if _RANK[level] > _RANK[self.levels[subject]]:
                self.levels[subject] = level
                self.capabilities[subject] = self.capabilities.get(
                    dependency, "validation.unsupported"
                )
                changed = True
        return changed

    def definition_capability(self, definition: TypeDef) -> str | None:
        if definition.kind == "struct":
            if definition.flattened:
                return "validation.object"
            for field in definition.fields:
                capability = self.ref_capability(field.ty)
                if capability is not None:
                    return capability
            return self.openness_capability(definition.openness)
        if definition.kind == "enum":
            for variant in definition.variants:
                if variant.payload_ty is not None:
                    capability = self.ref_capability(variant.payload_ty)
                    if capability is not None:
                        return capability
                for item in variant.payload_items:
                    capability = self.ref_capability(item)
                    if capability is not None:
                        return capability
                for field in variant.payload_fields:
                    capability = self.ref_capability(field.ty)
                    if capability is not None:
                        return capability
            return None
        if definition.kind == "newtype" and definition.inner is not None:
            return self.ref_capability(definition.inner)
        return None

    def openness_capability(self, openness: object) -> str | None:
        if not isinstance(openness, dict):
            return None
        preserve = openness.get("preserve_typed")
        if not isinstance(preserve, dict) or preserve.get("value") is None:
            return None
        try:
            return self.ref_capability(
                decode_type_ref(
                    preserve["value"], "openness.preserve_typed.value"
                )
            )
        except Exception:
            return None

    def ref_capability(self, ref: TypeRef) -> str | None:
        if ref.id is not None:
            return None
        kind = ref.kind
        raw = ref.raw
        if kind == "nullable":
            inner = raw.get("inner")
            return (
                None
                if inner is None
                else self.ref_capability(self._ref(inner))
            )
        if kind == "array":
            item = raw.get("item")
            return (
                None
                if item is None
                else self.ref_capability(self._ref(item))
            )
        if kind == "map":
            value = raw.get("value")
            return (
                None
                if value is None
                else self.ref_capability(self._ref(value))
            )
        if kind == "tuple":
            for item in raw.get("items", []):
                capability = self.ref_capability(self._ref(item))
                if capability is not None:
                    return capability
            return None
        if kind == "struct":
            if raw.get("flattened"):
                return "validation.object"
            for item in raw.get("fields", []):
                field = self._field(item)
                if field is not None:
                    capability = self.ref_capability(field.ty)
                    if capability is not None:
                        return capability
            return self.openness_capability(raw.get("openness", "reject"))
        return None

    def operation_capability(self, plan: object) -> str | None:
        references = [plan.success.body]
        references.extend(header.ty for header in plan.success.headers)
        if plan.stream is not None:
            references.extend(event.ty for event in plan.stream.events)
            if plan.stream.item is not None:
                references.append(plan.stream.item)
            if plan.stream.client is not None:
                references.append(plan.stream.client)
            if plan.stream.server is not None:
                references.append(plan.stream.server)
        for error in plan.errors:
            if error.body is not None:
                references.append(error.body)
            references.extend(header.ty for header in error.headers)
        for ref in references:
            capability = self.ref_capability(ref)
            if capability is not None:
                return capability
        return None

    def build_types(self) -> None:
        for plan in self.analysis.types:
            self.type_schemas[plan.id] = self.type_schema(plan)
            self.type_functions[plan.id] = self.claim(
                "validate_" + plan.name
            )

    def type_schema(self, plan: TypePlan) -> dict[str, Any]:
        if plan.kind == "struct":
            return self.struct_schema(plan)
        if plan.kind == "enum":
            return {
                "kind": "enum",
                "repr": plan.repr_kind,
                "variants": [
                    (variant.wire, variant.name, self.payload_schema(variant))
                    for variant in plan.variants
                ],
            }
        if plan.kind == "newtype":
            inner = (
                {"kind": "any"}
                if plan.inner is None
                else self.ref_schema(plan.inner)
            )
            return {"kind": "newtype", "inner": inner}
        return {"kind": "any"}

    def struct_schema(self, plan: TypePlan) -> dict[str, Any]:
        fields = [
            (
                field.wire,
                field.name,
                self.ref_schema(field.ty),
                field.optional,
                field.nullable,
            )
            for field in plan.fields
        ]
        extra = None
        if plan.openness == "preserve_typed" and plan.extra is not None:
            extra = self.ref_schema(plan.extra)
        return {
            "kind": "struct",
            "inline": False,
            "fields": fields,
            "openness": plan.openness,
            "extra": extra,
            "extra_attr": plan.extra_name if plan.openness != "reject" else "",
        }

    def payload_schema(self, variant: VariantPlan) -> dict[str, Any]:
        if variant.payload_kind == "newtype":
            ty = variant.ty
            return {
                "kind": "newtype",
                "ty": {"kind": "any"} if ty is None else self.ref_schema(ty),
            }
        if variant.payload_kind == "tuple":
            return {
                "kind": "tuple",
                "items": [self.ref_schema(item) for item in variant.items],
            }
        if variant.payload_kind == "struct":
            return {
                "kind": "struct",
                "inline": False,
                "fields": [
                    (
                        field.wire,
                        field.name,
                        self.ref_schema(field.ty),
                        field.optional,
                        field.nullable,
                    )
                    for field in variant.fields
                ],
                "openness": "reject",
                "extra": None,
                "extra_attr": "",
            }
        return {"kind": "unit"}

    def ref_schema(self, ref: TypeRef) -> dict[str, Any]:
        if ref.id is not None:
            schema: dict[str, Any] = {"ref": ref.id}
            if ref.variant is not None:
                schema["variant"] = ref.variant
            return schema
        kind = ref.kind
        raw = ref.raw
        if kind == "nullable":
            return {
                "kind": "nullable",
                "inner": self.inner_schema(raw.get("inner")),
            }
        if kind == "array":
            return {
                "kind": "array",
                "item": self.inner_schema(raw.get("item")),
                "unique": bool(raw.get("unique")),
            }
        if kind == "map":
            return {
                "kind": "map",
                "value": self.inner_schema(raw.get("value")),
            }
        if kind == "tuple":
            return {
                "kind": "tuple",
                "items": [
                    self.inner_schema(item) for item in raw.get("items", [])
                ],
            }
        if kind == "struct":
            return self.inline_struct_schema(raw)
        if kind == "int":
            return {
                "kind": "int",
                "bits": raw.get("bits"),
                "signed": bool(raw.get("signed")),
                "wire": raw.get("wire"),
                "minimum": raw.get("minimum"),
                "maximum": raw.get("maximum"),
                "multiple_of": raw.get("multiple_of"),
            }
        if kind == "float":
            return {
                "kind": "float",
                "bits": raw.get("bits"),
                "minimum": raw.get("minimum"),
                "maximum": raw.get("maximum"),
                "exclusive_minimum": raw.get("exclusive_minimum"),
                "exclusive_maximum": raw.get("exclusive_maximum"),
                "multiple_of": raw.get("multiple_of"),
            }
        if kind == "string":
            return {
                "kind": "string",
                "format": raw.get("format"),
                "min_length": raw.get("min_length"),
                "max_length": raw.get("max_length"),
                "pattern": raw.get("pattern"),
            }
        if kind == "decimal":
            return {"kind": "decimal", "wire": raw.get("wire")}
        if kind == "bytes":
            return {"kind": "bytes", "encoding": raw.get("encoding")}
        if kind == "bool":
            return {"kind": "bool"}
        return {"kind": "any"}

    def inline_struct_schema(self, raw: dict[str, Any]) -> dict[str, Any]:
        fields = []
        for item in raw.get("fields", []):
            field = self._field(item)
            if field is None:
                continue
            fields.append(
                (
                    field.wire,
                    field.wire,
                    self.ref_schema(field.ty),
                    field.presence == "optional",
                    field.presence == "optional" and is_nullable(field.ty),
                )
            )
        return {
            "kind": "struct",
            "inline": True,
            "fields": fields,
            "openness": "reject",
            "extra": None,
            "extra_attr": "",
        }

    def inner_schema(self, value: object) -> dict[str, Any]:
        if value is None:
            return {"kind": "any"}
        try:
            return self.ref_schema(decode_type_ref(value, "type reference"))
        except Exception:
            return {"kind": "any"}

    def register_ref(self, ref: TypeRef) -> None:
        signature = ref_signature(ref)
        if signature in self.ref_functions:
            return
        if ref.id is not None and ref.variant is None:
            name = self.type_functions.get(ref.id)
            if name is not None:
                self.ref_functions[signature] = name
                return
        name = self.claim(self.ref_name(ref))
        self.rendered.append(Rendered(name, self.ref_schema(ref), False))
        self.ref_functions[signature] = name

    def register_header(self, ref: TypeRef) -> None:
        signature = ref_signature(ref)
        if signature in self.header_functions:
            return
        name = self.claim("validate_header_" + str(len(self.header_functions) + 1))
        self.rendered.append(Rendered(name, self.ref_schema(ref), True))
        self.header_functions[signature] = name

    def ref_name(self, ref: TypeRef) -> str:
        if ref.id is not None and ref.variant is not None:
            plan = self.analysis.type_by_id.get(ref.id)
            base = plan.name if plan is not None else "value"
            suffix = "".join(
                character if character.isalnum() else "_"
                for character in ref.variant
            ).strip("_")
            return "validate_" + base + ("_" + suffix if suffix else "")
        return "validate_inline"

    def claim(self, base: str) -> str:
        name = base
        index = 2
        while name in self._names:
            name = f"{base}_{index}"
            index += 1
        self._names.add(name)
        return name

    def _ref(self, value: object) -> TypeRef:
        return decode_type_ref(value, "type reference")

    def _field(self, value: object):
        try:
            return _decode_field(value, "inline struct field")
        except Exception:
            return None
