"""Request construction shared by normal and streaming operations."""

from __future__ import annotations

from .analysis import Analysis
from .plan_operations import OperationPlan
from .render import literal


def request_setup(
    analysis: Analysis,
    plan: OperationPlan,
    indent: str = "        ",
    accept: bool = True,
) -> list[str]:
    lines = [indent + "headers: dict[str, str] = {}"]
    if accept:
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
    else:
        body = plan.body
        if body.required:
            if body.content != "multipart":
                lines.append(
                    f"{indent}headers[\"Content-Type\"] = {literal(body.media)}"
                )
            lines.extend(encode_body(analysis, body, indent))
            if body.content == "multipart":
                lines.append(f'{indent}headers["Content-Type"] = _content_type')
        else:
            lines.append(f"{indent}payload: bytes | None = None")
            lines.append(f"{indent}if body is not None:")
            if body.content != "multipart":
                lines.append(
                    f"{indent}    headers[\"Content-Type\"] = {literal(body.media)}"
                )
            lines.extend(encode_body(analysis, body, indent + "    "))
            if body.content == "multipart":
                lines.append(f'{indent}    headers["Content-Type"] = _content_type')
    return lines


def encode_body(analysis: Analysis, body: object, indent: str) -> list[str]:
    if body.content == "form":
        return [
            f"{indent}payload = urllib.parse.urlencode({body.encoder}(body)).encode(\"utf-8\")"
        ]
    if body.content == "multipart":
        return [f"{indent}_content_type, payload = {body.encoder}(body)"]
    if body.content == "text":
        value = analysis.wire.scalar_text(body.ty, "body")
        return [f"{indent}payload = {value}.encode(\"utf-8\")"]
    if body.content == "bytes":
        return [f"{indent}payload = _wire.raw_bytes(body)"]
    encoded = analysis.wire.encode_expr(body.ty, "body")
    return [
        f"{indent}payload = json.dumps({encoded}, separators=(\",\", \":\"), ensure_ascii=False).encode(\"utf-8\")"
    ]
