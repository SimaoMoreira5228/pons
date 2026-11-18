"""Render streaming operations and their event classes."""

from __future__ import annotations

from .analysis import Analysis
from .plan_operations import OperationPlan
from .render import literal, start
from .render_request import request_setup
from .render_signature import (
    alternatives_text,
    operation_summary,
    path_expression,
    signature,
)


def stream_hint(analysis: Analysis, plan: OperationPlan) -> str:
    stream = plan.stream
    assert stream is not None
    if stream.kind == "sse":
        names = [event.class_name for event in stream.events]
        inner = names[0] if len(names) == 1 else " | ".join(names)
        return f"Iterator[{inner}]"
    if stream.kind == "ndjson":
        assert stream.item is not None
        return f"Iterator[{analysis.wire.client_hint(stream.item)}]"
    if stream.kind == "websocket":
        assert stream.client is not None and stream.server is not None
        client = analysis.wire.client_hint(stream.client)
        server = analysis.wire.client_hint(stream.server)
        return f"TypedWebSocket[{client}, {server}]"
    return "Iterator[bytes]"


def render_event_classes(analysis: Analysis) -> str:
    lines = [start(f"Stream events for {analysis.document.title}.")]
    lines.append("from __future__ import annotations")
    lines.append("")
    lines.append("from dataclasses import dataclass")
    lines.append("from typing import ClassVar")
    lines.append("")
    lines.append("from . import types as _types")
    lines.append("")
    for plan in analysis.operations:
        if plan.stream is None or plan.stream.kind != "sse":
            continue
        for event in plan.stream.events:
            lines.append("")
            lines.append("@dataclass(slots=True)")
            lines.append(f"class {event.class_name}:")
            lines.append(
                f"    data: {analysis.wire.client_hint(event.ty)}"
            )
            lines.append(
                f"    event: ClassVar[str] = {literal(event.wire)}"
            )
    return "\n".join(lines).rstrip() + "\n"


def render_stream_operation(
    analysis: Analysis, plan: OperationPlan, provides: dict[str, str]
) -> list[str]:
    stream = plan.stream
    assert stream is not None
    hint = stream_hint(analysis, plan)
    lines = [
        f"    def {plan.method_name}({signature(analysis, plan)}) -> {hint}:"
    ]
    summary = operation_summary(plan)
    if summary:
        lines.append(f"        {literal(summary)}")
    lines.append("        client = self._client")
    lines.extend(request_setup(analysis, plan, accept=stream.kind != "websocket"))
    if stream.kind == "websocket":
        path = path_expression(analysis, plan)
        lines.append(f"        url = client._ws_url({path}, pairs)")
        lines.append(
            f"        auth_headers, auth_pairs = client._authorize("
        )
        lines.append(
            f"            {literal(plan.method)}, url, {alternatives_text(plan)}, {plan.required_auth}, headers"
        )
        lines.append("        )")
        lines.append("        headers.update(auth_headers)")
        lines.append("        if auth_pairs:")
        lines.append("            pairs.extend(auth_pairs)")
        lines.append(f"            url = client._ws_url({path}, pairs)")
        lines.append(
            "        request = Request(method="
            f"{literal(plan.method)}, url=url, headers=headers, body=payload, timeout=client.timeout)"
        )
        from .render_client import codec_expr, validator_expr

        assert stream.client is not None and stream.server is not None
        to_wire = analysis.wire.encode_expr(stream.client, "value")
        from_wire = codec_expr(analysis, stream.server)
        lines.append("        connection = client._open_websocket(request)")
        validator = validator_expr(analysis, stream.server)
        client_binary = binary_message(analysis, stream.client)
        server_binary = binary_message(analysis, stream.server)
        lines.append(
            f"        return TypedWebSocket(connection, lambda value: {to_wire}, {from_wire}, {validator}, {client_binary}, {server_binary})"
        )
        return lines
    path = path_expression(analysis, plan)
    lines.append(f"        url = client._url({path}, pairs)")
    lines.append(
        f"        auth_headers, auth_pairs = client._authorize("
    )
    lines.append(
        f"            {literal(plan.method)}, url, {alternatives_text(plan)}, {plan.required_auth}, headers"
    )
    lines.append("        )")
    lines.append("        headers.update(auth_headers)")
    lines.append("        if auth_pairs:")
    lines.append("            pairs.extend(auth_pairs)")
    lines.append(f"            url = client._url({path}, pairs)")
    lines.append(
        "        request = Request(method="
        f"{literal(plan.method)}, url=url, headers=headers, body=payload, timeout=client.timeout)"
    )
    lines.append("        with client._open_stream(request) as response:")
    lines.append(f"            if response.status != {plan.success.status}:")
    lines.append("                status = response.status")
    lines.append("                response_headers = response.headers")
    lines.append("                response_body = b\"\".join(response.body)")
    from .render_client import render_errors

    lines.extend(indent(render_errors(analysis, plan), 2))
    lines.append(
        f"            if not _wire.content_type_matches(response.headers, {literal(plan.success.media)}):"
    )
    lines.append("                raise DecodeError(")
    lines.append(
        f"                    f\"expected content type {plan.success.media!r}, found {{response.headers.get('content-type', 'missing')!r}}\","
    )
    lines.append(
        "                    path=\"headers.content-type\", status=response.status,"
    )
    lines.append("                )")
    for header in plan.success.headers:
        check = header_validator(analysis, header.ty)
        lines.append(
            f"            client._check_header(response.headers, {literal(header.wire)}, "
            f"response.status, {literal('headers.' + header.wire)}, "
            f"{'False' if header.optional else 'True'}, {check})"
        )
    if stream.kind == "bytes":
        lines.append("            for _chunk in response.body:")
        lines.append("                yield _chunk")
        return lines
    from .render_client import codec_expr, validator_expr

    if stream.kind == "sse":
        lines.append("            for _event, _data in _wire.sse_events(response.body):")
        for index, event in enumerate(stream.events):
            keyword = "if" if index == 0 else "elif"
            lines.append(f"                {keyword} _event == {literal(event.wire)}:")
            codec = codec_expr(analysis, event.ty)
            validator = validator_expr(analysis, event.ty)
            if validator is None:
                lines.append(
                    f"                    yield _streams.{event.class_name}("
                    f"data={codec}(_data, \"\"))"
                )
            else:
                lines.append(f"                    _value = {codec}(_data, \"\")")
                lines.append(f"                    _issue = {validator}(_value)")
                lines.append("                    if _issue is not None:")
                lines.append(
                    "                        raise DecodeError(_issue.message, path=_issue.path, status=response.status)"
                )
                lines.append(
                    f"                    yield _streams.{event.class_name}(data=_value)"
                )
        lines.append("                else:")
        lines.append(
            "                    raise DecodeError("
        )
        lines.append(
            "                        f\"unknown SSE event {_event!r}\", path=\"event\", status=response.status"
        )
        lines.append("                    )")
        return lines
    assert stream.item is not None
    lines.append("            for _item in _wire.ndjson(response.body):")
    item_codec = codec_expr(analysis, stream.item)
    item_validator = validator_expr(analysis, stream.item)
    if item_validator is None:
        lines.append(f"                yield {item_codec}(_item, \"\")")
    else:
        lines.append(f"                _value = {item_codec}(_item, \"\")")
        lines.append(f"                _issue = {item_validator}(_value)")
        lines.append("                if _issue is not None:")
        lines.append(
            "                    raise DecodeError(_issue.message, path=_issue.path, status=response.status)"
        )
        lines.append("                yield _value")
    return lines


def indent(lines: list[str], levels: int) -> list[str]:
    prefix = "    " * levels
    return [prefix + line if line else line for line in lines]


def header_validator(analysis: Analysis, ref: object) -> str | None:
    if analysis.validation_plan is None:
        return None
    name = analysis.validation_plan.header_for(ref)
    return None if name is None else f"_validators.{name}"


def binary_message(analysis: Analysis, ref: object) -> bool:
    if getattr(ref, "id", None):
        definition = analysis.document.types.get(ref.id)
        if definition is not None and definition.kind == "newtype" and definition.inner is not None:
            return binary_message(analysis, definition.inner)
        return False
    kind = getattr(ref, "kind", None)
    raw = getattr(ref, "raw", {})
    if kind == "bytes":
        return True
    if kind == "opaque":
        return raw.get("wire") == "bytes"
    return False
