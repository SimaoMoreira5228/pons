"""Render pagination iterators for `client.py`."""

from __future__ import annotations

from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from .analysis import Analysis
    from .plan_operations import OperationPlan


def render_iterator(
    analysis: Analysis, plan: OperationPlan, parameters: str
) -> list[str]:
    pagination = plan.pagination
    assert pagination is not None
    item_hint = analysis.wire.client_hint(pagination.item)
    lines = [
        f"    def {pagination.all_name}({parameters}) -> Iterator[{item_hint}]:",
        "        _base = query",
    ]
    if pagination.kind == "cursor":
        lines.append(
            f"        _cursor = _base.{pagination.request.name} if _base is not None else None"
        )
    else:
        lines.append("        _offset = 0")
        lines.append(
            f"        if _base is not None and _base.{pagination.request.name} is not None:"
        )
        lines.append(f"            _offset = _base.{pagination.request.name}")
        for _ in pagination.request.wrappers:
            lines.append("            _offset = _offset.value")
    default = None
    if pagination.page_size is not None:
        default = pagination.page_size.default
    if default is not None:
        lines.append("        _first = True")
    lines.append("        while True:")
    lines.append(
        f"            _request = _base if _base is not None else _types.{plan.query.name}()"
    )
    if default is not None:
        assert pagination.page_size is not None
        field = pagination.page_size.field
        lines.append("            if _first:")
        lines.append(f"                if _request.{field.name} is None:")
        wrapped = wrap(field.wrappers, str(default))
        lines.append(
            f"                    _request = dataclasses.replace(_request, {field.name}={wrapped})"
        )
        lines.append("            _first = False")
    if pagination.kind == "cursor":
        lines.append(
            f"            _request = dataclasses.replace(_request, {pagination.request.name}=_cursor)"
        )
    else:
        wrapped = wrap(pagination.request.wrappers, "_offset")
        lines.append(
            f"            _request = dataclasses.replace(_request, {pagination.request.name}={wrapped})"
        )
    call = f"self.{plan.method_name}({call_arguments(plan)})"
    if plan.success.headers:
        lines.append(f"            _response = {call}")
        lines.append("            _page = _response.body")
    else:
        lines.append(f"            _page = {call}")
    items = page_path("_page", pagination.items)
    lines.append(f"            _items = {items}")
    lines.append("            for _item in _items:")
    lines.append("                yield _item")
    if pagination.kind == "cursor":
        assert pagination.next is not None
        lines.append(f"            _value = {page_path('_page', pagination.next)}")
        if pagination.next_wrappers:
            lines.append(
                "            if _value is _wire.MISSING or _value is None:"
            )
            lines.append("                return")
            for _ in pagination.next_wrappers:
                lines.append("            _value = _value.value")
            lines.append('            if _value is None or _value == "":')
            lines.append("                return")
        else:
            lines.append(
                '            if _value is _wire.MISSING or _value is None or _value == "":'
            )
            lines.append("                return")
        lines.append(
            f"            _cursor = {wrap(pagination.request.wrappers, '_value')}"
        )
    else:
        lines.append("            if len(_items) == 0:")
        lines.append("                return")
        lines.append("            _offset = _offset + len(_items)")
    return lines


def call_arguments(plan: OperationPlan) -> str:
    parts: list[str] = []
    for param in plan.params:
        if param.kind == "path":
            parts.append(param.name)
        elif param.kind == "query":
            parts.append("query=_request")
        elif param.kind == "header":
            parts.append(f"{param.name}={param.name}")
    if plan.body is not None:
        parts.append(f"{plan.body.name}={plan.body.name}")
    return ", ".join(parts)


def page_path(base: str, path: list[str]) -> str:
    if not path:
        return base
    return base + "." + ".".join(path)


def wrap(wrappers: list[str], expression: str) -> str:
    for name in reversed(wrappers):
        expression = f"_types.{name}({expression})"
    return expression
