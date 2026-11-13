"""Type hints and wire expressions for the generated Python package."""

from __future__ import annotations

from typing import TYPE_CHECKING

from .document import TypeRef, decode_type_ref

if TYPE_CHECKING:
    from .analysis import Analysis


class WireSyntax:
    def __init__(self, analysis: Analysis) -> None:
        self.analysis = analysis

    def type_hint(self, ref: TypeRef) -> str:
        if ref.id:
            if ref.variant is not None:
                return self.analysis.type_planner.variant_class(ref)
            return self.analysis.type_names[ref.id]
        kind = ref.kind
        raw = ref.raw
        if kind == "bool":
            return "bool"
        if kind == "int":
            return "int"
        if kind == "float":
            return "float"
        if kind == "decimal":
            return "Decimal"
        if kind == "string":
            return "str"
        if kind == "bytes":
            return "bytes"
        if kind == "any":
            return "object"
        if kind == "nullable":
            return f"{self.type_hint(self._ref(raw['inner']))} | None"
        if kind == "array":
            return f"list[{self.type_hint(self._ref(raw['item']))}]"
        if kind == "map":
            return f"dict[str, {self.type_hint(self._ref(raw['value']))}]"
        if kind == "tuple":
            items = ", ".join(self.type_hint(self._ref(item)) for item in raw["items"])
            return f"tuple[{items}]"
        if kind == "struct":
            return "dict[str, object]"
        return "object"

    def client_hint(self, ref: TypeRef) -> str:
        if ref.id:
            if ref.variant is not None:
                return f"_types.{self.analysis.type_planner.variant_class(ref)}"
            return f"_types.{self.analysis.type_names[ref.id]}"
        return self.type_hint(ref)

    def decode_expr(self, ref: TypeRef, value: str, path: str) -> str:
        if ref.id:
            if ref.variant is not None:
                return (
                    f"{self.analysis.type_planner.variant_class(ref)}.from_wire"
                    f"({value}, {path})"
                )
            plan = self.analysis.type_by_id[ref.id]
            if plan.kind == "enum":
                return f"decode_{plan.name}({value}, {path})"
            return f"{plan.name}.from_wire({value}, {path})"
        kind = ref.kind
        raw = ref.raw
        if kind == "bool":
            return f"_wire.bool_of({value}, {path})"
        if kind == "int":
            return (
                f"_wire.int_of({value}, {path}, {raw.get('bits')}, "
                f"{'True' if raw.get('signed') else 'False'}, {raw.get('wire')!r})"
            )
        if kind == "float":
            return f"_wire.float_of({value}, {path}, {raw.get('bits')})"
        if kind == "decimal":
            return f"_wire.decimal_of({value}, {path})"
        if kind == "string":
            return f"_wire.str_of({value}, {path})"
        if kind == "bytes":
            return f"_wire.bytes_of({value}, {path}, {raw.get('encoding')!r})"
        if kind == "any":
            return f"_wire.any_of({value}, {path})"
        if kind == "nullable":
            inner = self.decode_expr(self._ref(raw["inner"]), "v", "p")
            return f"_wire.nullable({value}, {path}, lambda v, p: {inner})"
        if kind == "array":
            inner = self.decode_expr(self._ref(raw["item"]), "v", "p")
            return f"_wire.list_of({value}, {path}, lambda v, p: {inner})"
        if kind == "map":
            inner = self.decode_expr(self._ref(raw["value"]), "v", "p")
            return f"_wire.map_of({value}, {path}, lambda v, p: {inner})"
        if kind == "tuple":
            decoders = ", ".join(
                f"lambda v, p: {self.decode_expr(self._ref(item), 'v', 'p')}"
                for item in raw["items"]
            )
            return f"_wire.tuple_of({value}, {path}, [{decoders}])"
        if kind == "struct":
            fields = []
            for item in raw.get("fields", []):
                field = self._field(item, "inline struct")
                if field is None:
                    continue
                inner = self.decode_expr(field.ty, "v", "p")
                fields.append(
                    f"({field.wire!r}, "
                    f"{'True' if field.presence == 'optional' else 'False'}, "
                    f"{'True' if self._optional_nullable(field) else 'False'}, "
                    f"lambda v, p: {inner})"
                )
            return f"_wire.inline_decode({value}, {path}, [{', '.join(fields)}])"
        return f"_wire.any_of({value}, {path})"

    def encode_expr(self, ref: TypeRef, value: str) -> str:
        if ref.id:
            return f"{value}.to_wire()"
        kind = ref.kind
        raw = ref.raw
        if kind == "int" and raw.get("wire") == "string":
            return f"str({value})"
        if kind in {"bool", "int", "float", "string", "any"}:
            return value
        if kind == "decimal":
            return f"_wire.decimal_to({value})"
        if kind == "bytes":
            return f"_wire.bytes_to({value}, {raw.get('encoding')!r})"
        if kind == "nullable":
            inner = self.encode_expr(self._ref(raw["inner"]), "v")
            return f"_wire.nullable_to({value}, lambda v: {inner})"
        if kind == "array":
            inner = self.encode_expr(self._ref(raw["item"]), "v")
            return f"_wire.list_to({value}, lambda v: {inner})"
        if kind == "map":
            inner = self.encode_expr(self._ref(raw["value"]), "v")
            return f"_wire.map_to({value}, lambda v: {inner})"
        if kind == "tuple":
            encoders = ", ".join(
                f"lambda v: {self.encode_expr(self._ref(item), 'v')}"
                for item in raw["items"]
            )
            return f"_wire.tuple_to({value}, [{encoders}])"
        if kind == "struct":
            fields = []
            for item in raw.get("fields", []):
                field = self._field(item, "inline struct")
                if field is None:
                    continue
                inner = self.encode_expr(field.ty, "v")
                fields.append(
                    f"({field.wire!r}, "
                    f"{'True' if field.presence == 'optional' else 'False'}, "
                    f"{'True' if self._optional_nullable(field) else 'False'}, "
                    f"lambda v: {inner})"
                )
            return f"_wire.inline_encode({value}, [{', '.join(fields)}])"
        return value

    def scalar_text(self, ref: TypeRef, value: str) -> str:
        if ref.id:
            definition = self.analysis.document.types.get(ref.id)
            if definition is None:
                return f"_wire.text({value})"
            if definition.kind == "enum":
                return f"_wire.text(({value}).to_wire())"
            if definition.kind == "newtype" and definition.inner is not None:
                return self.scalar_text(definition.inner, f"({value}).value")
            return f"_wire.text({value})"
        return f"_wire.text({value})"

    def _optional_nullable(self, field: object) -> bool:
        presence = getattr(field, "presence")
        ty = getattr(field, "ty")
        return presence == "optional" and not ty.id and ty.kind == "nullable"

    def _ref(self, value: object) -> TypeRef:
        return decode_type_ref(value, "type reference")

    def _field(self, value: object, label: str):
        from .document import _decode_field

        try:
            return _decode_field(value, f"{label}.field")
        except Exception:
            return None
