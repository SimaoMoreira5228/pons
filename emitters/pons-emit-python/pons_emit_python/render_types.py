"""Render `types.py` for the generated package."""

from __future__ import annotations

from .analysis import Analysis
from .plan_types import FieldPlan, TypePlan, VariantPlan
from .render import literal, start


def render_types(analysis: Analysis) -> str:
    lines = [start(f"Models and wire codecs for {analysis.document.title}.")]
    lines.append("from __future__ import annotations")
    lines.append("")
    lines.append("from dataclasses import dataclass")
    if analysis.uses_decimal:
        lines.append("from decimal import Decimal")
    lines.append("")
    lines.append("from . import _wire")
    lines.append("")
    lines.append("")
    for plan in analysis.types:
        lines.extend(render_type(analysis, plan))
        lines.append("")
        lines.append("")
    return "\n".join(lines).rstrip() + "\n"


def render_type(analysis: Analysis, plan: TypePlan) -> list[str]:
    if plan.kind == "struct":
        return render_struct(analysis, plan)
    if plan.kind == "enum":
        return render_enum(analysis, plan)
    if plan.kind == "newtype":
        return render_newtype(analysis, plan)
    return []


def ordered_fields(fields: list[FieldPlan]) -> list[FieldPlan]:
    return [field for field in fields if not field.optional] + [
        field for field in fields if field.optional
    ]


def field_hint(analysis: Analysis, field: FieldPlan) -> str:
    hint = analysis.wire.type_hint(field.ty)
    if not field.optional:
        return hint
    if field.nullable:
        return f"{hint} | _wire.Missing = _wire.MISSING"
    return f"{hint} | None = None"


def render_struct(analysis: Analysis, plan: TypePlan) -> list[str]:
    lines = ["@dataclass(slots=True)", f"class {plan.name}:"]
    if not plan.fields:
        lines.append("    pass")
    for field in ordered_fields(plan.fields):
        lines.append(f"    {field.name}: {field_hint(analysis, field)}")
    lines.append("")
    lines.append("    @classmethod")
    lines.append(
        f'    def from_wire(cls, value: object, path: str = "") -> {plan.name}:'
    )
    if not plan.fields:
        lines.append("        _wire.obj(value, path)")
        lines.append("        return cls()")
    else:
        lines.append("        data = _wire.obj(value, path)")
        lines.append("        model = cls.__new__(cls)")
        lines.extend(decode_fields(analysis, plan.fields, "model", "data", "path"))
        lines.append("        return model")
    lines.append("")
    lines.append("    def to_wire(self) -> object:")
    if not plan.fields:
        lines.append("        return {}")
        return lines
    lines.append("        out: dict[str, object] = {}")
    lines.extend(encode_fields(analysis, plan.fields, "self", "out"))
    lines.append("        return out")
    return lines


def decode_fields(
    analysis: Analysis,
    fields: list[FieldPlan],
    target: str,
    data: str,
    path: str,
) -> list[str]:
    lines: list[str] = []
    for field in fields:
        key = literal(field.wire)
        field_path = f"_wire.child({path}, {key})"
        if field.optional:
            lines.append(f"        raw = _wire.opt({data}, {key})")
            decoded = analysis.wire.decode_expr(field.ty, "raw", field_path)
            missing = "_wire.MISSING" if field.nullable else "None"
            lines.append(
                f"        {target}.{field.name} = {missing} if raw is _wire.MISSING else {decoded}"
            )
            continue
        value = f"_wire.field({data}, {key}, {path})"
        decoded = analysis.wire.decode_expr(field.ty, value, field_path)
        lines.append(f"        {target}.{field.name} = {decoded}")
    return lines


def encode_fields(
    analysis: Analysis,
    fields: list[FieldPlan],
    source: str,
    target: str,
) -> list[str]:
    lines: list[str] = []
    for field in fields:
        key = literal(field.wire)
        encoded = analysis.wire.encode_expr(field.ty, f"{source}.{field.name}")
        if field.optional:
            guard = "_wire.MISSING" if field.nullable else "None"
            lines.append(f"        if {source}.{field.name} is not {guard}:")
            lines.append(f"            {target}[{key}] = {encoded}")
        else:
            lines.append(f"        {target}[{key}] = {encoded}")
    return lines


def render_newtype(analysis: Analysis, plan: TypePlan) -> list[str]:
    assert plan.inner is not None
    lines = [
        "@dataclass(slots=True)",
        f"class {plan.name}:",
        f"    value: {analysis.wire.type_hint(plan.inner)}",
        "",
        "    @classmethod",
        f'    def from_wire(cls, value: object, path: str = "") -> {plan.name}:',
    ]
    decoded = analysis.wire.decode_expr(plan.inner, "value", "path")
    lines.append(f"        return cls({decoded})")
    lines.append("")
    lines.append("    def to_wire(self) -> object:")
    encoded = analysis.wire.encode_expr(plan.inner, "self.value")
    lines.append(f"        return {encoded}")
    return lines


def render_enum(analysis: Analysis, plan: TypePlan) -> list[str]:
    lines: list[str] = []
    for variant in plan.variants:
        if lines:
            lines.append("")
            lines.append("")
        lines.extend(render_variant(analysis, plan, variant))
    lines.append("")
    lines.append("")
    names = " | ".join(variant.name for variant in plan.variants)
    lines.append(f"{plan.name} = {names}")
    lines.append("")
    lines.append("")
    lines.extend(render_enum_decoder(plan))
    return lines


def render_variant(
    analysis: Analysis, plan: TypePlan, variant: VariantPlan
) -> list[str]:
    lines = ["@dataclass(slots=True)", f"class {variant.name}:"]
    if variant.payload_kind == "unit":
        lines.append("    pass")
    elif variant.payload_kind == "newtype":
        assert variant.ty is not None
        lines.append(f"    value: {analysis.wire.type_hint(variant.ty)}")
    elif variant.payload_kind == "tuple":
        items = ", ".join(
            analysis.wire.type_hint(item) for item in variant.items
        )
        lines.append(f"    value: tuple[{items}]")
    elif variant.payload_kind == "struct":
        if not variant.fields:
            lines.append("    pass")
        for field in ordered_fields(variant.fields):
            lines.append(f"    {field.name}: {field_hint(analysis, field)}")
    lines.append("")
    lines.append("    @classmethod")
    lines.append(
        f'    def from_wire(cls, value: object, path: str = "") -> {variant.name}:'
    )
    lines.extend(variant_decode(analysis, plan, variant))
    lines.append("")
    lines.append("    def to_wire(self) -> object:")
    lines.extend(variant_encode(analysis, plan, variant))
    return lines


def variant_decode(
    analysis: Analysis, plan: TypePlan, variant: VariantPlan
) -> list[str]:
    wire = literal(variant.wire)
    tag = literal(plan.tag)
    content = literal(plan.content)
    payload = variant.payload_kind != "unit"
    lines: list[str] = []
    if plan.repr_kind == "string":
        lines.append("        text = _wire.str_of(value, path)")
        lines.append(f"        if text != {wire}:")
        lines.append(
            f"            raise _wire.variant_mismatch({literal(plan.name)}, {wire}, text, path)"
        )
        return lines + ["        return cls()"]
    if plan.repr_kind == "external":
        if not payload:
            lines.append("        text = _wire.str_of(value, path)")
            lines.append(f"        if text != {wire}:")
            lines.append(
                f"            raise _wire.variant_mismatch({literal(plan.name)}, {wire}, text, path)"
            )
            return lines + ["        return cls()"]
        lines.append("        data = _wire.obj(value, path)")
        lines.append("        if len(data) != 1:")
        lines.append(
            f"            raise _wire.variant_shape({literal(plan.name)}, \"an object with one variant\", path)"
        )
        lines.append(f"        if {wire} not in data:")
        lines.append(
            f"            raise _wire.unknown_variant({literal(plan.name)}, next(iter(data)), path)"
        )
        lines.extend(
            payload_decode(
                analysis,
                variant,
                f"data[{wire}]",
                f"_wire.child(path, {wire})",
            )
        )
        return lines
    if plan.repr_kind == "internal":
        lines.append("        data = _wire.obj(value, path)")
        lines.append(f"        tag = _wire.field(data, {tag}, path)")
        lines.append(f"        if tag != {wire}:")
        lines.append(
            f"            raise _wire.variant_mismatch({literal(plan.name)}, {wire}, tag, path)"
        )
        if not payload:
            return lines + ["        return cls()"]
        lines.extend(payload_decode(analysis, variant, "data", "path"))
        return lines
    lines.append("        data = _wire.obj(value, path)")
    lines.append(f"        tag = _wire.field(data, {tag}, path)")
    lines.append(f"        if tag != {wire}:")
    lines.append(
        f"            raise _wire.variant_mismatch({literal(plan.name)}, {wire}, tag, path)"
    )
    if not payload:
        return lines + ["        return cls()"]
    lines.append(f"        raw = _wire.field(data, {content}, path)")
    lines.extend(
        payload_decode(
            analysis,
            variant,
            "raw",
            f"_wire.child(path, {content})",
        )
    )
    return lines


def payload_decode(
    analysis: Analysis, variant: VariantPlan, value: str, path: str
) -> list[str]:
    if variant.payload_kind == "newtype":
        assert variant.ty is not None
        decoded = analysis.wire.decode_expr(variant.ty, value, path)
        return [f"        return cls({decoded})"]
    if variant.payload_kind == "tuple":
        decoders = ", ".join(
            f"lambda v, p: {analysis.wire.decode_expr(item, 'v', 'p')}"
            for item in variant.items
        )
        return [f"        return cls(_wire.tuple_of({value}, {path}, [{decoders}]))"]
    if variant.payload_kind == "struct":
        if not variant.fields:
            return [f"        _wire.obj({value}, {path})", "        return cls()"]
        return [
            f"        struct = _wire.obj({value}, {path})",
            "        model = cls.__new__(cls)",
            *decode_fields(analysis, variant.fields, "model", "struct", path),
            "        return model",
        ]
    return ["        return cls()"]


def variant_encode(
    analysis: Analysis, plan: TypePlan, variant: VariantPlan
) -> list[str]:
    wire = literal(variant.wire)
    tag = literal(plan.tag)
    content = literal(plan.content)
    payload = variant.payload_kind != "unit"
    if plan.repr_kind == "string":
        return [f"        return {wire}"]
    if plan.repr_kind == "external":
        if not payload:
            return [f"        return {wire}"]
        if variant.payload_kind == "struct":
            return [
                "        payload: dict[str, object] = {}",
                *encode_fields(analysis, variant.fields, "self", "payload"),
                f"        return {{{wire}: payload}}",
            ]
        return [
            f"        return {{{wire}: {payload_encode(analysis, variant)}}}"
        ]
    if plan.repr_kind == "internal":
        if not payload:
            return [f"        return {{{tag}: {wire}}}"]
        if variant.payload_kind == "struct":
            return [
                f"        out: dict[str, object] = {{{tag}: {wire}}}",
                *encode_fields(analysis, variant.fields, "self", "out"),
                "        return out",
            ]
        return [
            f"        payload = {payload_encode(analysis, variant)}",
            f"        return {{**payload, {tag}: {wire}}}",
        ]
    if not payload:
        return [f"        return {{{tag}: {wire}}}"]
    if variant.payload_kind == "struct":
        return [
            "        payload: dict[str, object] = {}",
            *encode_fields(analysis, variant.fields, "self", "payload"),
            f"        return {{{tag}: {wire}, {content}: payload}}",
        ]
    return [
        f"        return {{{tag}: {wire}, {content}: {payload_encode(analysis, variant)}}}"
    ]


def payload_encode(analysis: Analysis, variant: VariantPlan) -> str:
    if variant.payload_kind == "newtype":
        assert variant.ty is not None
        return analysis.wire.encode_expr(variant.ty, "self.value")
    if variant.payload_kind == "tuple":
        encoders = ", ".join(
            f"lambda v: {analysis.wire.encode_expr(item, 'v')}"
            for item in variant.items
        )
        return f"_wire.tuple_to(self.value, [{encoders}])"
    return "None"


def render_enum_decoder(plan: TypePlan) -> list[str]:
    lines = [
        f"def decode_{plan.name}(value: object, path: str = \"\") -> {plan.name}:"
    ]
    if plan.repr_kind == "string":
        lines.append("    text = _wire.str_of(value, path)")
        for variant in plan.variants:
            lines.append(f"    if text == {literal(variant.wire)}:")
            lines.append(f"        return {variant.name}.from_wire(value, path)")
        lines.append(
            f"    raise _wire.unknown_variant({literal(plan.name)}, text, path)"
        )
        return lines
    if plan.repr_kind == "external":
        lines.append("    if isinstance(value, str):")
        lines.append("        text = value")
        for variant in plan.variants:
            lines.append(f"        if text == {literal(variant.wire)}:")
            lines.append(f"            return {variant.name}.from_wire(value, path)")
        lines.append(
            f"        raise _wire.unknown_variant({literal(plan.name)}, text, path)"
        )
        lines.append("    data = _wire.obj(value, path)")
        lines.append("    if len(data) != 1:")
        lines.append(
            f"        raise _wire.variant_shape({literal(plan.name)}, \"an object with one variant\", path)"
        )
        lines.append("    wire = next(iter(data))")
        for variant in plan.variants:
            lines.append(f"    if wire == {literal(variant.wire)}:")
            lines.append(f"        return {variant.name}.from_wire(value, path)")
        lines.append(
            f"    raise _wire.unknown_variant({literal(plan.name)}, wire, path)"
        )
        return lines
    lines.append("    data = _wire.obj(value, path)")
    lines.append(f"    tag = _wire.field(data, {literal(plan.tag)}, path)")
    for variant in plan.variants:
        lines.append(f"    if tag == {literal(variant.wire)}:")
        lines.append(f"        return {variant.name}.from_wire(value, path)")
    lines.append(
        f"    raise _wire.unknown_variant({literal(plan.name)}, tag, path)"
    )
    return lines
