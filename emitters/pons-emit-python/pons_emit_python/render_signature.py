"""Operation signatures and request paths for the Python emitter."""

from __future__ import annotations

from urllib.parse import quote

from .analysis import Analysis
from .plan_operations import OperationPlan
from .render import literal


def signature(analysis: Analysis, plan: OperationPlan) -> str:
    parts = ["self"]
    for param in plan.params:
        if param.kind == "path":
            parts.append(f"{param.name}: {analysis.wire.client_hint(param.ty)}")
    keywords: list[str] = []
    if plan.query is not None:
        keywords.append(f"query: _types.{plan.query.name} | None = None")
    for param in plan.params:
        if param.kind != "header":
            continue
        hint = analysis.wire.client_hint(param.ty)
        if param.optional:
            keywords.append(f"{param.name}: {hint} | None = None")
        else:
            keywords.append(f"{param.name}: {hint}")
    if plan.body is not None:
        hint = analysis.wire.client_hint(plan.body.ty)
        if plan.body.required:
            keywords.append(f"body: {hint}")
        else:
            keywords.append(f"body: {hint} | None = None")
    if keywords:
        parts.append("*")
        parts.extend(keywords)
    return ", ".join(parts)


def return_hint(analysis: Analysis, plan: OperationPlan) -> str:
    body = analysis.wire.client_hint(plan.success.body)
    if plan.success.headers:
        return f"Response[{body}]"
    return body


def operation_summary(plan: OperationPlan) -> str | None:
    if plan.doc is None:
        return None
    summary = plan.doc.get("summary")
    if isinstance(summary, str) and summary:
        return " ".join(summary.split())
    return None


def alternatives_text(plan: OperationPlan) -> str:
    return "[" + ", ".join(literal(alternative) for alternative in plan.alternatives) + "]"


def path_expression(analysis: Analysis, plan: OperationPlan) -> str:
    pieces: list[str] = []
    for kind, value, ref in plan.path:
        if kind == "lit":
            pieces.append(literal("/" + quote(value, safe="")))
        else:
            assert ref is not None
            text = analysis.wire.scalar_text(ref, value)
            pieces.append("\"/\" + _wire.path_segment(" + text + ")")
    if not pieces:
        return literal("/")
    return " + ".join(pieces)
