"""Type plans, naming, openness, and reference validation for the Python emitter."""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import TYPE_CHECKING

from . import naming
from .codes import DOCUMENT, UNSUPPORTED
from .document import Field, TypeDef, TypeRef, _decode_field, decode_type_ref

if TYPE_CHECKING:
    from .analysis import Analysis

SUPPORTED_ENUMS = frozenset(
    {"string", "external", "internal", "adjacent", "untagged"}
)


@dataclass(slots=True)
class FieldPlan:
    wire: str
    name: str
    ty: TypeRef
    optional: bool
    nullable: bool


@dataclass(slots=True)
class VariantPlan:
    wire: str
    name: str
    payload_kind: str
    ty: TypeRef | None = None
    items: list[TypeRef] = field(default_factory=list)
    fields: list[FieldPlan] = field(default_factory=list)


@dataclass(slots=True)
class TypePlan:
    id: str
    name: str
    kind: str
    definition: TypeDef
    fields: list[FieldPlan] = field(default_factory=list)
    variants: list[VariantPlan] = field(default_factory=list)
    inner: TypeRef | None = None
    repr_kind: str = ""
    tag: str = ""
    content: str = ""
    openness: str = "reject"
    extra: TypeRef | None = None
    extra_name: str = ""

    def export_names(self) -> list[str]:
        if self.kind == "enum":
            return [self.name] + [variant.name for variant in self.variants]
        return [self.name]

    def variant(self, wire: str) -> VariantPlan | None:
        for variant in self.variants:
            if variant.wire == wire:
                return variant
        return None


def is_nullable(ref: TypeRef) -> bool:
    return not ref.id and ref.kind == "nullable"


class TypePlanner:
    def __init__(self, analysis: Analysis) -> None:
        self.analysis = analysis
        self.type_names: dict[str, str] = {}
        self.types: list[TypePlan] = []
        self.by_id: dict[str, TypePlan] = {}
        self.variant_names: dict[tuple[str, str], str] = {}
        self._claimed = naming.NameRegistry(naming.type_reserved())

    def build(self) -> None:
        self.assign_names()
        for type_id in sorted(self.analysis.document.types):
            definition = self.analysis.document.types[type_id]
            subject = f"types.{type_id}"
            plan = TypePlan(
                id=type_id,
                name=self.type_names[type_id],
                kind=definition.kind,
                definition=definition,
            )
            if definition.kind == "struct":
                self.build_struct(plan, subject)
            elif definition.kind == "enum":
                self.build_enum(plan, subject)
            elif definition.kind == "newtype":
                if definition.inner is None:
                    self.analysis.add(
                        DOCUMENT, subject, f"the newtype {type_id} has no inner type"
                    )
                elif self.validate_ref(
                    definition.inner, subject, f"the newtype {type_id}"
                ):
                    plan.inner = definition.inner
            else:
                self.analysis.add(
                    UNSUPPORTED,
                    subject,
                    f"the type kind {definition.kind!r} is not supported",
                )
            self.types.append(plan)
            self.by_id[type_id] = plan

    def assign_names(self) -> None:
        groups: dict[str, list[str]] = {}
        for type_id, definition in self.analysis.document.types.items():
            base = naming.pascal([definition.name], naming.type_reserved())
            groups.setdefault(base, []).append(type_id)
        for base in sorted(groups):
            ids = sorted(groups[base])
            if len(ids) == 1:
                self.type_names[ids[0]] = self._claimed.claim(base, separator="")
                continue
            for type_id in ids:
                direction = self.analysis.document.types[type_id].direction
                suffix = {"input": "Input", "output": "Output"}.get(
                    direction, "Both"
                )
                self.type_names[type_id] = self._claimed.claim(
                    base + suffix, separator=""
                )

    def openness(self, raw: object) -> str:
        if raw == "reject":
            return "reject"
        if raw == "preserve_opaque":
            return "preserve_opaque"
        if isinstance(raw, dict) and "preserve_typed" in raw:
            return "preserve_typed"
        return "invalid"

    def build_struct(self, plan: TypePlan, subject: str) -> None:
        definition = plan.definition
        if definition.flattened:
            self.analysis.add(
                UNSUPPORTED,
                subject,
                f"the struct {plan.id} uses flattened members, which this emitter does not support",
            )
        openness = self.openness(definition.openness)
        if openness == "invalid":
            self.analysis.add(
                UNSUPPORTED,
                subject,
                f"the struct {plan.id} uses the invalid object openness {definition.openness!r}",
            )
        reserved = set(naming.field_reserved())
        if openness != "reject":
            reserved.add("field")
        names = naming.NameRegistry(reserved)
        wires: set[str] = set()
        for field in definition.fields:
            field_plan = self.build_field(
                field,
                subject,
                f"the struct {plan.id}",
                names,
                wires,
                definition.direction,
            )
            if field_plan is not None:
                plan.fields.append(field_plan)
        if openness == "preserve_opaque":
            plan.openness = openness
            plan.extra_name = names.claim("extra")
        elif openness == "preserve_typed":
            plan.openness = openness
            plan.extra = self.preserve_value(
                definition.openness, subject, f"the struct {plan.id}"
            )
            if plan.extra is not None and not self.validate_ref(
                plan.extra, subject, f"the struct {plan.id} extra value"
            ):
                plan.extra = None
            plan.extra_name = names.claim("extra")

    def preserve_value(
        self, raw: object, subject: str, label: str
    ) -> TypeRef | None:
        preserve = raw.get("preserve_typed") if isinstance(raw, dict) else None
        if not isinstance(preserve, dict) or preserve.get("value") is None:
            self.analysis.add(
                DOCUMENT,
                subject,
                f"{label} has a preserve_typed openness without a value type",
            )
            return None
        try:
            return decode_type_ref(
                preserve["value"], f"{label}.openness.preserve_typed.value"
            )
        except Exception as error:
            self.analysis.add(DOCUMENT, subject, str(error))
            return None

    def build_field(
        self,
        field: Field,
        subject: str,
        label: str,
        names: naming.NameRegistry,
        wires: set[str],
        direction: str = "both",
    ) -> FieldPlan | None:
        if field.wire in wires:
            self.analysis.add(
                DOCUMENT,
                subject,
                f"{label} has more than one field with the wire name {field.wire!r}",
            )
            return None
        if field.access == "read_only" and direction != "output":
            self.analysis.add(
                UNSUPPORTED,
                subject,
                f"{label} field {field.wire} is read_only, but the type direction is {direction}",
            )
            return None
        if field.access == "write_only" and direction != "input":
            self.analysis.add(
                UNSUPPORTED,
                subject,
                f"{label} field {field.wire} is write_only, but the type direction is {direction}",
            )
            return None
        if not self.validate_ref(field.ty, subject, f"{label} field {field.wire}"):
            return None
        words = field.ident or naming.words_from_wire(field.wire)
        name = names.claim(naming.snake([words], naming.field_reserved()))
        wires.add(field.wire)
        optional = field.presence == "optional"
        return FieldPlan(field.wire, name, field.ty, optional, optional and is_nullable(field.ty))

    def build_enum(self, plan: TypePlan, subject: str) -> None:
        definition = plan.definition
        repr_kind = definition.repr_kind or ""
        plan.repr_kind = repr_kind
        plan.tag = definition.tag
        plan.content = definition.content
        if repr_kind not in SUPPORTED_ENUMS:
            self.analysis.add(
                UNSUPPORTED,
                subject,
                f"the enum {plan.id} uses the {repr_kind} representation, which this emitter does not support",
            )
            return
        for variant in definition.variants:
            word = naming.pascal(
                [variant.ident or naming.words_from_wire(variant.wire)]
            )
            if word.endswith("_") and word[:-1]:
                word = word[:-1]
            variant_name = self._claimed.claim(
                plan.name + word,
                separator="",
            )
            plan_variant = VariantPlan(
                wire=variant.wire,
                name=variant_name,
                payload_kind=variant.payload_kind,
            )
            self.variant_names[(plan.id, variant.wire)] = variant_name
            if variant.payload_kind == "newtype":
                if variant.payload_ty is None:
                    self.analysis.add(
                        DOCUMENT,
                        subject,
                        f"the enum {plan.id} variant {variant.wire} has no payload type",
                    )
                    continue
                if not self.validate_ref(
                    variant.payload_ty,
                    subject,
                    f"the enum {plan.id} variant {variant.wire}",
                ):
                    continue
                plan_variant.ty = variant.payload_ty
                if repr_kind == "internal" and not self.is_object_payload(
                    variant.payload_ty
                ):
                    self.analysis.add(
                        UNSUPPORTED,
                        subject,
                        f"the internal enum {plan.id} variant {variant.wire} has a payload that is not an object, so the tag cannot share its object",
                    )
            elif variant.payload_kind == "tuple":
                if repr_kind == "internal":
                    self.analysis.add(
                        UNSUPPORTED,
                        subject,
                        f"the internal enum {plan.id} variant {variant.wire} has a tuple payload, so the tag cannot share its object",
                    )
                    continue
                for item in variant.payload_items:
                    if not self.validate_ref(
                        item,
                        subject,
                        f"the enum {plan.id} variant {variant.wire}",
                    ):
                        continue
                    plan_variant.items.append(item)
            elif variant.payload_kind == "struct":
                names = naming.NameRegistry(naming.field_reserved())
                wires: set[str] = set()
                for item in variant.payload_fields:
                    field_plan = self.build_field(
                        item,
                        subject,
                        f"the enum {plan.id} variant {variant.wire}",
                        names,
                        wires,
                        definition.direction,
                    )
                    if field_plan is not None:
                        plan_variant.fields.append(field_plan)
            elif variant.payload_kind != "unit":
                self.analysis.add(
                    UNSUPPORTED,
                    subject,
                    f"the enum {plan.id} variant {variant.wire} uses the payload kind {variant.payload_kind!r}",
                )
            if repr_kind == "string" and variant.payload_kind != "unit":
                self.analysis.add(
                    UNSUPPORTED,
                    subject,
                    f"the string enum {plan.id} variant {variant.wire} has a {variant.payload_kind} payload",
                )
            plan.variants.append(plan_variant)

    def is_object_payload(self, ref: TypeRef) -> bool:
        if ref.id:
            definition = self.analysis.document.types.get(ref.id)
            if definition is None:
                return False
            if definition.kind == "struct":
                return True
            if definition.kind == "newtype" and definition.inner is not None:
                return self.is_object_payload(definition.inner)
            return False
        return ref.kind == "struct"

    def validate_ref(self, ref: TypeRef, subject: str, label: str) -> bool:
        if ref.id:
            definition = self.analysis.document.types.get(ref.id)
            if definition is None:
                self.analysis.add(
                    DOCUMENT,
                    subject,
                    f"{label} references the unknown type {ref.id}",
                )
                return False
            if ref.variant is None:
                return True
            if definition.kind != "enum":
                self.analysis.add(
                    UNSUPPORTED,
                    subject,
                    f"{label} narrows the non-enum type {ref.id} to the {ref.variant} variant",
                )
                return False
            if ref.variant not in {variant.wire for variant in definition.variants}:
                self.analysis.add(
                    DOCUMENT,
                    subject,
                    f"{label} narrows {ref.id} to the unknown variant {ref.variant!r}",
                )
                return False
            return True
        if ref.kind is None:
            self.analysis.add(DOCUMENT, subject, f"{label} is not a type reference")
            return False
        return self.validate_inline(ref, subject, label)

    def validate_inline(self, ref: TypeRef, subject: str, label: str) -> bool:
        raw = ref.raw
        kind = ref.kind
        if kind == "bool":
            return True
        if kind == "int":
            bits = raw.get("bits")
            if bits not in {8, 16, 32, 64, 128}:
                self.analysis.add(
                    UNSUPPORTED, subject, f"{label} uses a {bits}-bit integer"
                )
                return False
            if raw.get("wire") not in {"number", "string"}:
                self.analysis.add(
                    UNSUPPORTED,
                    subject,
                    f"{label} uses the integer wire {raw.get('wire')!r}",
                )
                return False
            return True
        if kind == "float":
            if raw.get("bits") not in {32, 64}:
                self.analysis.add(
                    UNSUPPORTED,
                    subject,
                    f"{label} uses a {raw.get('bits')}-bit float",
                )
                return False
            return True
        if kind == "decimal":
            if raw.get("wire") != "string":
                self.analysis.add(
                    UNSUPPORTED,
                    subject,
                    f"{label} uses a decimal on the {raw.get('wire')!r} wire; this emitter preserves decimals as strings",
                )
                return False
            self.analysis.uses_decimal = True
            return True
        if kind == "string":
            return True
        if kind == "bytes":
            if raw.get("encoding") not in {"base64", "base64url", "hex", "array"}:
                self.analysis.add(
                    UNSUPPORTED,
                    subject,
                    f"{label} uses the byte encoding {raw.get('encoding')!r}",
                )
                return False
            return True
        if kind == "null":
            self.analysis.add(
                UNSUPPORTED,
                subject,
                f"{label} uses the null type, which this emitter does not support",
            )
            return False
        if kind == "any":
            return True
        if kind == "opaque":
            self.analysis.uses_opaque = True
            self.analysis.opaque.add(subject)
            return True
        if kind == "nullable":
            inner = raw.get("inner")
            if inner is None:
                self.analysis.add(
                    DOCUMENT, subject, f"{label} has a nullable wrapper with no inner type"
                )
                return False
            return self.validate_ref(self._ref(inner), subject, label)
        if kind == "array":
            item = raw.get("item")
            if item is None:
                self.analysis.add(
                    DOCUMENT, subject, f"{label} has an array with no item type"
                )
                return False
            return self.validate_ref(self._ref(item), subject, label)
        if kind == "map":
            key = raw.get("key")
            value = raw.get("value")
            if key is None or value is None:
                self.analysis.add(
                    DOCUMENT,
                    subject,
                    f"{label} has a map without a key or value type",
                )
                return False
            key_ref = self._ref(key)
            if key_ref.id or key_ref.kind != "string":
                self.analysis.add(
                    UNSUPPORTED,
                    subject,
                    f"{label} uses a map key that is not a string",
                )
                return False
            return self.validate_ref(self._ref(value), subject, label)
        if kind == "tuple":
            items = raw.get("items")
            if not isinstance(items, list):
                self.analysis.add(
                    DOCUMENT, subject, f"{label} has a tuple without items"
                )
                return False
            for item in items:
                if not self.validate_ref(self._ref(item), subject, label):
                    return False
            return True
        if kind == "struct":
            if raw.get("flattened"):
                self.analysis.add(
                    UNSUPPORTED,
                    subject,
                    f"{label} uses flattened members, which this emitter does not support",
                )
                return False
            openness = self.openness(raw.get("openness", "reject"))
            if openness != "reject":
                self.analysis.add(
                    UNSUPPORTED,
                    subject,
                    f"{label} uses object openness {openness!r}; inline structs must be closed",
                )
                return False
            names = naming.NameRegistry()
            wires: set[str] = set()
            for item in raw.get("fields", []):
                field = self._field(item, label)
                if field is None:
                    return False
                if (
                    self.build_field(field, subject, label, names, wires)
                    is None
                ):
                    return False
            return True
        self.analysis.add(
            UNSUPPORTED, subject, f"{label} uses the unsupported type kind {kind!r}"
        )
        return False

    def _ref(self, value: object) -> TypeRef:
        return decode_type_ref(value, "type reference")

    def _field(self, value: object, label: str) -> Field | None:
        try:
            return _decode_field(value, f"{label}.field")
        except Exception as error:
            self.analysis.add(DOCUMENT, "document", str(error))
            return None

    def is_scalar(self, ref: TypeRef) -> bool:
        if ref.id:
            definition = self.analysis.document.types.get(ref.id)
            if definition is None:
                return False
            if definition.kind == "enum":
                return definition.repr_kind == "string"
            if definition.kind == "newtype" and definition.inner is not None:
                return self.is_scalar(definition.inner)
            return False
        return ref.kind in {"bool", "int", "float", "string", "decimal"}

    def variant_class(self, ref: TypeRef) -> str:
        assert ref.id is not None and ref.variant is not None
        return self.variant_names[(ref.id, ref.variant)]
