"""Supported-subset analysis and coverage for the Python emitter."""

from __future__ import annotations

from dataclasses import dataclass
from typing import TYPE_CHECKING

from . import naming
from .document import Document, Field, TypeDef, TypeRef, _decode_field, decode_type_ref
from .plan_operations import OperationPlan, OperationPlanner
from .plan_types import TypePlan, TypePlanner
from .plan_validation import ValidationPlan, ValidationPlanner
from .wire_syntax import WireSyntax

if TYPE_CHECKING:
    from .protocol import EmitterOptions


@dataclass(slots=True)
class Problem:
    code: str
    message: str
    origin: str
    subject: str


class Analysis:
    def __init__(
        self, document: Document, options: EmitterOptions, mode: str
    ) -> None:
        self.document = document
        self.package = options.package
        self.problems: list[Problem] = []
        self.unsupported: set[str] = set()
        self.opaque: set[str] = set()
        self.uses_decimal = False
        self.uses_opaque = False
        self.type_planner = TypePlanner(self)
        self.type_planner.build()
        self.type_names = self.type_planner.type_names
        self.types: list[TypePlan] = self.type_planner.types
        self.type_by_id = self.type_planner.by_id
        self.stream_names = naming.NameRegistry(
            {name for plan in self.types for name in plan.export_names()}
        )
        self.operation_planner = OperationPlanner(self)
        self.operation_planner.build()
        self.operations: list[OperationPlan] = self.operation_planner.operations
        self.errors = self.operation_planner.errors
        self.error_order = self.operation_planner.error_order
        self.query_types = self.operation_planner.query_types
        self.query_functions = self.operation_planner.query_functions
        self.multipart_types = self.operation_planner.multipart_types
        self.multipart_functions = self.operation_planner.multipart_functions
        self.wire = WireSyntax(self)
        self.validation_plan: ValidationPlan | None = None
        if options.validation is not None:
            self.validation_plan = ValidationPlanner(
                self, options.validation
            ).build()
            if mode == "strict":
                self.problems.extend(self.validation_plan.problems())

    def subject_types(self, type_id: str) -> str:
        return f"types.{type_id}"

    def subject_operation(self, plan: OperationPlan) -> str:
        return f"operations.{plan.id}"

    def add(self, code: str, subject: str, message: str) -> None:
        if subject.startswith("types."):
            origin = subject[len("types.") :]
        elif subject.startswith("operations."):
            origin = subject[len("operations.") :]
        else:
            origin = subject
        self.problems.append(Problem(code, message, origin, subject))
        self.unsupported.add(subject)

    def coverage(self) -> dict[str, list[str]]:
        subjects = {f"types.{type_id}" for type_id in self.document.types}
        subjects.update(f"operations.{plan.id}" for plan in self.operations)
        unsupported = set(self.unsupported)
        self.propagate_unsupported(unsupported)
        opaque = set(self.opaque)
        self.propagate_opaque(opaque)
        opaque -= unsupported
        exact = sorted(subjects - unsupported - opaque)
        return {
            "exact": exact,
            "opaque": sorted(opaque),
            "lossy": [],
            "unsupported": sorted(unsupported),
        }

    def propagate_unsupported(self, unsupported: set[str]) -> None:
        unsupported_types = {
            subject[len("types.") :]
            for subject in unsupported
            if subject.startswith("types.")
        }
        self.propagate_types(
            unsupported_types,
            lambda type_id: unsupported.add(f"types.{type_id}"),
        )
        self.propagate_operations(
            unsupported_types,
            unsupported,
            lambda subject: unsupported.add(subject),
        )

    def propagate_opaque(self, opaque: set[str]) -> None:
        opaque_types = {
            subject[len("types.") :]
            for subject in opaque
            if subject.startswith("types.")
        }
        self.propagate_types(
            opaque_types, lambda type_id: opaque.add(f"types.{type_id}")
        )
        self.propagate_operations(
            opaque_types, opaque, lambda subject: opaque.add(subject)
        )

    def propagate_types(self, marked: set[str], mark) -> None:
        changed = True
        while changed:
            changed = False
            for type_id, definition in self.document.types.items():
                if type_id in marked:
                    continue
                if any(
                    ref.id in marked for ref in self.definition_refs(definition)
                ):
                    marked.add(type_id)
                    mark(type_id)
                    changed = True

    def propagate_operations(
        self, type_ids: set[str], subjects: set[str], mark
    ) -> None:
        for plan in self.operations:
            subject = f"operations.{plan.id}"
            if subject in subjects:
                continue
            if any(ref.id in type_ids for ref in self.operation_refs(plan)):
                mark(subject)

    def definition_refs(self, definition: TypeDef) -> list[TypeRef]:
        references: list[TypeRef] = []

        def collect(ref: TypeRef) -> None:
            references.append(ref)
            if ref.id:
                return
            if ref.kind in {"nullable", "array"}:
                inner = ref.raw.get("inner") or ref.raw.get("item")
                if inner is not None:
                    collect(self._ref(inner))
            elif ref.kind == "map":
                collect(self._ref(ref.raw["key"]))
                collect(self._ref(ref.raw["value"]))
            elif ref.kind == "tuple":
                for item in ref.raw.get("items", []):
                    collect(self._ref(item))
            elif ref.kind == "struct":
                for item in ref.raw.get("fields", []):
                    field = self._field(item, "inline struct")
                    if field is not None:
                        collect(field.ty)
                if isinstance(ref.raw.get("openness"), dict):
                    preserve = ref.raw["openness"].get("preserve_typed")
                    if isinstance(preserve, dict) and preserve.get("value") is not None:
                        collect(self._ref(preserve["value"]))

        if definition.kind == "struct":
            for field in definition.fields:
                collect(field.ty)
            for ref in definition.flattened:
                collect(ref)
            if isinstance(definition.openness, dict):
                preserve = definition.openness.get("preserve_typed")
                if isinstance(preserve, dict) and preserve.get("value") is not None:
                    collect(self._ref(preserve["value"]))
        elif definition.kind == "enum":
            for variant in definition.variants:
                if variant.payload_ty is not None:
                    collect(variant.payload_ty)
                for ref in variant.payload_items:
                    collect(ref)
                for field in variant.payload_fields:
                    collect(field.ty)
        elif definition.kind == "newtype" and definition.inner is not None:
            collect(definition.inner)
        return references

    def operation_refs(self, plan: OperationPlan) -> list[TypeRef]:
        references = [ref for _, _, ref in plan.path if ref is not None]
        if plan.query is not None:
            references.append(TypeRef(id=plan.query.id))
        for param in plan.params:
            if param.kind == "header":
                references.append(param.ty)
        if plan.body is not None:
            references.append(plan.body.ty)
        if plan.success.status:
            references.append(plan.success.body)
            for header in plan.success.headers:
                references.append(header.ty)
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
            for header in error.headers:
                references.append(header.ty)
        return references

    def _ref(self, value: object) -> TypeRef:
        try:
            return decode_type_ref(value, "type reference")
        except Exception:
            return TypeRef()

    def _field(self, value: object, label: str) -> Field | None:
        try:
            return _decode_field(value, f"{label}.field")
        except Exception:
            return None
