"""Request construction shared by normal and streaming operations."""

from __future__ import annotations

from .analysis import Analysis
from .plan_operations import OperationPlan
from .render import literal


def request_setup(
    analysis: Analysis, plan: OperationPlan, indent: str = "        "
) -> list[str]:
    lines = [indent + "headers: dict[str, str] = {}"]
    lines.append(f"{indent}headers[\"Accept\"] = {literal(plan.success.media)}")
    for param in plan.params:
        if param.kind != "header":
            continue
        text = analysis.wire.scalar_text(param.ty, param.name)
        if param.optional:
            lines.append(f"{indent}if {param.name} is not None:")
            lines.append(
                f"{indent}    headers[{literal(param.wire)}] = {text}"
            )
        else:
            lines.append(f"{indent}headers[{literal(param.wire)}] = {text}")
    if plan.query is not None:
        lines.append(
            f"{indent}pairs: list[tuple[str, str]] = "
            f"{plan.query_function}(query) if query is not None else []"
        )
    else:
        lines.append(f"{indent}pairs: list[tuple[str, str]] = []")
    if plan.body is None:
        lines.append(f"{indent}payload: bytes | None = None")
    elif plan.body.required:
        lines.append(
            f"{indent}headers[\"Content-Type\"] = {literal(plan.body.media)}"
        )
        encoded = analysis.wire.encode_expr(plan.body.ty, "body")
        lines.append(
            f"{indent}payload = json.dumps({encoded}, separators=(\",\", \":\"), ensure_ascii=False).encode(\"utf-8\")"
        )
    else:
        lines.append(f"{indent}payload: bytes | None = None")
        lines.append(f"{indent}if body is not None:")
        lines.append(
            f"{indent}    headers[\"Content-Type\"] = {literal(plan.body.media)}"
        )
        encoded = analysis.wire.encode_expr(plan.body.ty, "body")
        lines.append(
            f"{indent}    payload = json.dumps({encoded}, separators=(\",\", \":\"), ensure_ascii=False).encode(\"utf-8\")"
        )
    return lines
