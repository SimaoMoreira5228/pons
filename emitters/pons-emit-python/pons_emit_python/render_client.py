"""Render `client.py` for the generated package."""

from __future__ import annotations

from dataclasses import dataclass, field
from urllib.parse import quote

from . import naming
from .analysis import Analysis
from .plan_operations import ErrorPlan, OperationPlan
from .document import TypeRef, decode_type_ref
from .render import literal, start
from .render_client_runtime import (
    provides_constants,
    render_client_helpers,
    render_provides,
    render_runtime_types,
    render_schemes,
)
from .render_pagination import render_iterator


@dataclass
class NamespaceNode:
    names: list[str]
    children: dict[tuple[str, ...], "NamespaceNode"] = field(default_factory=dict)
    methods: list[OperationPlan] = field(default_factory=list)
    class_name: str = ""


def render_client(analysis: Analysis) -> str:
    root = build_tree(analysis)
    assign_class_names(root)
    provides = provides_constants(analysis)
    lines = [start(f"HTTP client for {analysis.document.title}.")]
    lines.extend(render_imports(analysis))
    lines.append("")
    lines.append("_T = TypeVar(\"_T\")")
    lines.append("")
    lines.extend(render_runtime_types())
    lines.append("")
    lines.extend(render_schemes(analysis))
    if provides:
        lines.append("")
        lines.extend(render_provides(analysis, provides))
    query_lines = render_queries(analysis)
    if query_lines:
        lines.append("")
        lines.extend(query_lines)
    lines.append("")
    lines.extend(render_client_class(analysis, root, provides))
    for node in walk_namespaces(root):
        lines.append("")
        lines.extend(render_namespace(analysis, node, provides))
    return "\n".join(lines).rstrip() + "\n"


def render_imports(analysis: Analysis) -> list[str]:
    errors = ["ConfigurationError", "DecodeError", "PonsError", "TransportError", "UnexpectedStatusError"]
    errors.extend(analysis.errors[key].name for key in analysis.error_order)
    paginated = any(plan.pagination is not None for plan in analysis.operations)
    collection_names = "Callable, Iterator, Mapping" if paginated else "Callable, Mapping"
    lines = [
        "from __future__ import annotations",
        "",
        "import base64",
    ]
    if paginated:
        lines.append("import dataclasses")
    lines.extend(
        [
            "import json",
            "import urllib.error",
            "import urllib.parse",
            "import urllib.request",
            f"from collections.abc import {collection_names}",
            "from dataclasses import dataclass",
            "from typing import Any, Generic, TypeVar",
            "",
            "from . import _wire",
            "from . import types as _types",
        ]
    )
    if analysis.validation_plan is not None:
        lines.append("from . import validators as _validators")
    lines.append("from .errors import (")
    for name in errors:
        lines.append(f"    {name},")
    lines.append(")")
    return lines


def render_queries(analysis: Analysis) -> list[str]:
    lines: list[str] = []
    for type_id in sorted(analysis.query_types):
        plan = analysis.query_types[type_id]
        function = analysis.query_functions[type_id]
        lines.append(
            f"def {function}(query: _types.{plan.name}) -> list[tuple[str, str]]:"
        )
        lines.append("    pairs: list[tuple[str, str]] = []")
        for field in plan.fields:
            if field.ty.kind == "array" and not field.ty.id:
                item = decode_type_ref(field.ty.raw["item"], "query item")
                if field.optional:
                    lines.append(f"    if query.{field.name} is not None:")
                    indent = "        "
                else:
                    indent = "    "
                lines.append(f"{indent}for item in query.{field.name}:")
                text = analysis.wire.scalar_text(item, "item")
                lines.append(
                    f"{indent}    pairs.append(({literal(field.wire)}, {text}))"
                )
                continue
            text = analysis.wire.scalar_text(field.ty, f"query.{field.name}")
            if field.optional:
                lines.append(f"    if query.{field.name} is not None:")
                lines.append(
                    f"        pairs.append(({literal(field.wire)}, {text}))"
                )
            else:
                lines.append(f"    pairs.append(({literal(field.wire)}, {text}))")
        lines.append("    return pairs")
    return lines


def build_tree(analysis: Analysis) -> NamespaceNode:
    root = NamespaceNode([])
    for plan in analysis.operations:
        node = root
        raw_path: tuple[str, ...] = ()
        for segment, name in zip(plan.raw_namespace, plan.namespace_path):
            raw_path += (tuple(segment),)
            child = node.children.get(raw_path)
            if child is None:
                child = NamespaceNode(plan.namespace_path[: len(raw_path)])
                node.children[raw_path] = child
            node = child
        node.methods.append(plan)
    return root


def assign_class_names(root: NamespaceNode) -> None:
    registry = naming.NameRegistry({"Client", "Request", "Response", "Transport"})
    for node in walk_namespaces(root):
        base = "".join(naming.pascal([[name]]) for name in node.names) + "Namespace"
        node.class_name = registry.claim(base, separator="")


def walk_namespaces(node: NamespaceNode) -> list[NamespaceNode]:
    result: list[NamespaceNode] = []
    for key in sorted(node.children, key=lambda item: node.children[item].names):
        child = node.children[key]
        result.append(child)
        result.extend(walk_namespaces(child))
    return result


def render_client_class(
    analysis: Analysis, root: NamespaceNode, provides: dict[str, str]
) -> list[str]:
    lines = ["class Client:", "    def __init__(", "        self,", "        base_url: str,", "        *,"]
    lines.extend(
        [
            "        auth: Mapping[str, object] | None = None,",
            "        credentials: dict[str, object] | None = None,",
            "        headers: Mapping[str, str] | None = None,",
            "        timeout: float = 10.0,",
            "        transport: Transport | None = None,",
            "    ) -> None:",
            "        self.base_url = base_url.rstrip(\"/\")",
            "        self.credentials = credentials if credentials is not None else {}",
            "        self.headers = dict(headers or {})",
            "        self.timeout = timeout",
            "        self.transport = transport or _urllib_transport",
            "        self._auth = dict(auth or {})",
            "        self._expirations: dict[str, object] = {}",
        ]
    )
    for key in sorted(root.children, key=lambda item: root.children[item].names):
        child = root.children[key]
        lines.append(f"        self._{child.names[-1]}_namespace = {child.class_name}(self)")
    for key in sorted(root.children, key=lambda item: root.children[item].names):
        child = root.children[key]
        lines.append("")
        lines.append("    @property")
        lines.append(f"    def {child.names[-1]}(self) -> {child.class_name}:")
        lines.append(f"        return self._{child.names[-1]}_namespace")
    lines.extend(render_client_helpers())
    for plan in root.methods:
        lines.append("")
        lines.extend(render_operation(analysis, plan, "self", provides))
    return lines


def render_namespace(
    analysis: Analysis, node: NamespaceNode, provides: dict[str, str]
) -> list[str]:
    children = sorted(node.children.values(), key=lambda child: child.names)
    slots = ["_client"] + [f"_{child.names[-1]}_namespace" for child in children]
    slot_text = ", ".join(literal(item) for item in slots) + ","
    lines = [
        f"class {node.class_name}:",
        "    __slots__ = (" + slot_text + ")",
        "",
        "    def __init__(self, client: Client) -> None:",
        "        self._client = client",
    ]
    for child in children:
        lines.append(f"        self._{child.names[-1]}_namespace = {child.class_name}(client)")
    for child in children:
        lines.append("")
        lines.append("    @property")
        lines.append(f"    def {child.names[-1]}(self) -> {child.class_name}:")
        lines.append(f"        return self._{child.names[-1]}_namespace")
    for plan in node.methods:
        lines.append("")
        lines.extend(render_operation(analysis, plan, "self._client", provides))
    return lines


def render_operation(
    analysis: Analysis,
    plan: OperationPlan,
    receiver: str,
    provides: dict[str, str],
) -> list[str]:
    parameters = signature(analysis, plan)
    lines = [f"    def {plan.method_name}({parameters}) -> {return_hint(analysis, plan)}:"]
    summary = operation_summary(plan)
    if summary:
        lines.append(f"        {literal(summary)}")
    lines.append(f"        client = {receiver}")
    lines.extend(render_request(analysis, plan))
    lines.append("")
    lines.extend(render_success(analysis, plan, provides))
    lines.extend(render_errors(analysis, plan))
    if plan.pagination is not None:
        lines.append("")
        lines.extend(render_iterator(analysis, plan, parameters))
    return lines


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


def render_request(analysis: Analysis, plan: OperationPlan) -> list[str]:
    lines = ["        headers: dict[str, str] = {}"]
    lines.append(
        f"        headers[\"Accept\"] = {literal(plan.success.media)}"
    )
    for param in plan.params:
        if param.kind != "header":
            continue
        text = analysis.wire.scalar_text(param.ty, param.name)
        if param.optional:
            lines.append(f"        if {param.name} is not None:")
            lines.append(
                f"            headers[{literal(param.wire)}] = {text}"
            )
        else:
            lines.append(
                f"        headers[{literal(param.wire)}] = {text}"
            )
    if plan.query is not None:
        lines.append(
            f"        pairs: list[tuple[str, str]] = {plan.query_function}(query) if query is not None else []"
        )
    else:
        lines.append("        pairs: list[tuple[str, str]] = []")
    if plan.body is None:
        lines.append("        payload: bytes | None = None")
    elif plan.body.required:
        lines.append(f"        headers[\"Content-Type\"] = {literal(plan.body.media)}")
        encoded = analysis.wire.encode_expr(plan.body.ty, "body")
        lines.append(
            f"        payload = json.dumps({encoded}, separators=(\",\", \":\"), ensure_ascii=False).encode(\"utf-8\")"
        )
    else:
        lines.append("        payload: bytes | None = None")
        lines.append("        if body is not None:")
        lines.append(f"            headers[\"Content-Type\"] = {literal(plan.body.media)}")
        encoded = analysis.wire.encode_expr(plan.body.ty, "body")
        lines.append(
            f"            payload = json.dumps({encoded}, separators=(\",\", \":\"), ensure_ascii=False).encode(\"utf-8\")"
        )
    lines.append(
        "        status, response_headers, response_body = client._perform("
    )
    lines.append(f"            {literal(plan.method)},")
    lines.append(f"            {path_expression(analysis, plan)},")
    lines.append("            query=pairs,")
    lines.append("            headers=headers,")
    lines.append("            body=payload,")
    lines.append(f"            alternatives={alternatives_text(plan)},")
    lines.append(f"            required={plan.required_auth},")
    lines.append("        )")
    return lines


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


def render_success(
    analysis: Analysis, plan: OperationPlan, provides: dict[str, str]
) -> list[str]:
    lines = [f"        if status == {plan.success.status}:"]
    codec = codec_expr(analysis, plan.success.body)
    validator = validator_expr(analysis, plan.success.body)
    tail = f", {validator}" if validator is not None else ""
    lines.append(
        f"            value = client._decode(response_body, status, response_headers, "
        f"{literal(plan.success.media)}, {codec}{tail})"
    )
    if plan.id in provides:
        lines.append(
            f"            client._store_provides({provides[plan.id]}, response_headers, response_body)"
        )
    for header in plan.success.headers:
        check = header_validator_expr(analysis, header.ty)
        lines.append(
            f"            client._check_header(response_headers, {literal(header.wire)}, "
            f"status, {literal('headers.' + header.wire)}, "
            f"{'False' if header.optional else 'True'}, {check})"
        )
    if plan.success.headers:
        lines.append(
            "            return Response(status=status, body=value, headers=response_headers)"
        )
    else:
        lines.append("            return value")
    return lines


def validator_expr(analysis: Analysis, ref: TypeRef) -> str | None:
    if analysis.validation_plan is None:
        return None
    name = analysis.validation_plan.function_for(ref)
    return None if name is None else f"_validators.{name}"


def header_validator_expr(analysis: Analysis, ref: TypeRef) -> str | None:
    if analysis.validation_plan is None:
        return None
    name = analysis.validation_plan.header_for(ref)
    return None if name is None else f"_validators.{name}"


def codec_expr(analysis: Analysis, ref: TypeRef) -> str:
    if ref.id:
        if ref.variant is not None:
            return f"_types.{analysis.type_planner.variant_class(ref)}.from_wire"
        if analysis.type_by_id[ref.id].kind == "enum":
            return f"_types.decode_{analysis.type_names[ref.id]}"
        return f"_types.{analysis.type_names[ref.id]}.from_wire"
    return f"lambda v, p: {analysis.wire.decode_expr(ref, 'v', 'p')}"


def render_errors(analysis: Analysis, plan: OperationPlan) -> list[str]:
    lines: list[str] = []
    exact: list[ErrorPlan] = []
    classes: list[ErrorPlan] = []
    default: ErrorPlan | None = None
    for error in plan.errors:
        if isinstance(error.status, int):
            exact.append(error)
        elif error.status == "default":
            if default is None:
                default = error
        else:
            classes.append(error)
    exact.sort(key=lambda error: int(error.status))
    classes.sort(key=lambda error: str(error.status))
    for error in exact:
        lines.extend(render_error_case(analysis, plan, error, f"status == {error.status}"))
    for error in classes:
        number = int(str(error.status)[0])
        lines.extend(
            render_error_case(
                analysis,
                plan,
                error,
                f"{number * 100} <= status < {(number + 1) * 100}",
            )
        )
    if default is not None:
        lines.extend(render_error_case(analysis, plan, default, None))
    else:
        lines.append(
            "        raise UnexpectedStatusError(status, response_headers, response_body)"
        )
    return lines


def render_error_case(
    analysis: Analysis, plan: OperationPlan, error: ErrorPlan, condition: str | None
) -> list[str]:
    indent = "        "
    lines: list[str] = []
    if condition is not None:
        lines.append(f"{indent}if {condition}:")
        indent = "            "
    if error.body is not None:
        codec = codec_expr(analysis, error.body)
        validator = validator_expr(analysis, error.body)
        tail = f", {validator}" if validator is not None else ""
        lines.append(
            f"{indent}error_body = client._decode(response_body, status, response_headers, "
            f"{literal(error.media)}, {codec}{tail})"
        )
        for header in error.headers:
            check = header_validator_expr(analysis, header.ty)
            lines.append(
                f"{indent}client._check_header(response_headers, {literal(header.wire)}, "
                f"status, {literal('headers.' + header.wire)}, "
                f"{'False' if header.optional else 'True'}, {check})"
            )
        lines.append(
            f"{indent}raise {error.name}(status, response_headers, error_body)"
        )
    else:
        for header in error.headers:
            check = header_validator_expr(analysis, header.ty)
            lines.append(
                f"{indent}client._check_header(response_headers, {literal(header.wire)}, "
                f"status, {literal('headers.' + header.wire)}, "
                f"{'False' if header.optional else 'True'}, {check})"
            )
        lines.append(f"{indent}raise {error.name}(status, response_headers, None)")
    return lines
