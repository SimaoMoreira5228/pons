"""Pagination descriptor resolution for the Python emitter."""

from __future__ import annotations

from dataclasses import dataclass
from typing import TYPE_CHECKING

from .codes import UNSUPPORTED
from .document import (
    Body,
    Operation,
    PageSize as PageSizeDescriptor,
    TypeRef,
    decode_type_ref,
)
from .plan_types import FieldPlan

if TYPE_CHECKING:
    from .analysis import Analysis
    from .plan_operations import OperationPlan


@dataclass(slots=True)
class PaginationField:
    wire: str
    name: str
    wrappers: list[str]


@dataclass(slots=True)
class PageSizePlan:
    field: PaginationField
    default: int | None


@dataclass(slots=True)
class PaginationPlan:
    kind: str
    request: PaginationField
    items: list[str]
    next: list[str] | None
    next_wrappers: list[str]
    item: TypeRef
    page_size: PageSizePlan | None
    all_name: str = ""


def build_pagination(
    analysis: Analysis,
    plan: OperationPlan,
    operation: Operation,
    subject: str,
) -> PaginationPlan | None:
    descriptor = operation.pagination
    if descriptor is None:
        return None
    body = operation.body
    request_wire = descriptor.request.field
    if body is not None and _body_declares(analysis, body, request_wire):
        return _refuse(
            analysis,
            subject,
            f"the pagination request field {request_wire!r} is declared on the request body; the iterator cannot replace a body",
        )
    request_field = _query_field(plan, request_wire)
    if request_field is None:
        return _refuse(
            analysis,
            subject,
            f"the pagination request field {request_wire!r} is not declared on the operation query",
        )
    request_kind = "string" if descriptor.request.kind == "cursor" else "int"
    request = _resolve_field(analysis, request_field, request_kind)
    if isinstance(request, str):
        return _refuse(
            analysis,
            subject,
            f"the pagination request field {request_wire!r} {request}",
        )
    page_size = _build_page_size(analysis, plan, body, descriptor.page_size)
    if isinstance(page_size, str):
        return _refuse(analysis, subject, page_size)
    success_body = plan.success.body if plan.success.status else None
    if success_body is None:
        return _refuse(
            analysis,
            subject,
            "the paginated operation has no JSON success body to read a page from",
        )
    items = _resolve_items(analysis, success_body, descriptor.items)
    if isinstance(items, str):
        return _refuse(analysis, subject, items)
    item_path, item = items
    next_path: list[str] | None = None
    next_wrappers: list[str] = []
    if descriptor.request.kind == "cursor":
        if descriptor.next is None:
            return _refuse(
                analysis,
                subject,
                "cursor pagination needs a next pointer to advance past the first page",
            )
        resolved_next = _resolve_next(analysis, success_body, descriptor.next)
        if isinstance(resolved_next, str):
            return _refuse(analysis, subject, resolved_next)
        next_path, next_wrappers = resolved_next
    elif descriptor.next is not None:
        return _refuse(
            analysis,
            subject,
            "offset pagination cannot declare a next pointer because the iterator advances the offset",
        )
    return PaginationPlan(
        kind=descriptor.request.kind,
        request=request,
        items=item_path,
        next=next_path,
        next_wrappers=next_wrappers,
        item=item,
        page_size=page_size,
    )


def _build_page_size(
    analysis: Analysis,
    plan: OperationPlan,
    body: Body | None,
    descriptor: PageSizeDescriptor | None,
) -> PageSizePlan | str | None:
    if descriptor is None:
        return None
    wire = descriptor.field
    if body is not None and _body_declares(analysis, body, wire):
        return (
            f"the page size field {wire!r} is declared on the request body; "
            "the iterator cannot replace a body"
        )
    field = _query_field(plan, wire)
    if field is None:
        return f"the page size field {wire!r} is not declared on the operation query"
    resolved = _resolve_field(analysis, field, "int")
    if isinstance(resolved, str):
        return f"the page size field {wire!r} {resolved}"
    default = descriptor.default
    if default is not None and default <= 0:
        return f"the page size default {default} is not positive"
    return PageSizePlan(resolved, default)


def _resolve_field(
    analysis: Analysis,
    field: FieldPlan,
    kind: str,
) -> PaginationField | str:
    ref = field.ty
    wrappers: list[str] = []
    seen: set[str] = set()
    while ref.id:
        if ref.variant is not None:
            return f"narrows the enum {ref.id} to a variant"
        if ref.id in seen:
            return "forms a newtype cycle"
        definition = analysis.document.types.get(ref.id)
        if definition is None:
            return f"names the unknown type {ref.id}"
        if definition.kind != "newtype" or definition.inner is None:
            return "is not a string" if kind == "string" else "is not an integer"
        seen.add(ref.id)
        wrappers.append(analysis.type_names[ref.id])
        ref = definition.inner
    if ref.kind == "nullable":
        return "is nullable, which a query field cannot encode"
    if ref.kind != kind:
        return "is not a string" if kind == "string" else "is not an integer"
    return PaginationField(field.wire, field.name, wrappers)


def _query_field(plan: OperationPlan, wire: str) -> FieldPlan | None:
    if plan.query is None:
        return None
    for field in plan.query.fields:
        if field.wire == wire:
            return field
    return None


def _body_declares(analysis: Analysis, body: Body, wire: str) -> bool:
    return _declares(analysis, body.ty, wire, set())


def _declares(
    analysis: Analysis, ref: TypeRef, wire: str, seen: set[str]
) -> bool:
    if ref.id:
        if ref.id in seen:
            return False
        definition = analysis.document.types.get(ref.id)
        if definition is None:
            return False
        if definition.kind == "newtype" and definition.inner is not None:
            return _declares(analysis, definition.inner, wire, seen | {ref.id})
        if definition.kind == "struct":
            return any(field.wire == wire for field in definition.fields)
        return False
    if ref.kind == "nullable":
        inner = ref.raw.get("inner")
        return inner is not None and _declares(
            analysis, decode_type_ref(inner, "body field"), wire, seen
        )
    if ref.kind == "struct":
        return any(
            isinstance(field, dict) and field.get("wire") == wire
            for field in ref.raw.get("fields", [])
        )
    return False


def _resolve_items(
    analysis: Analysis, body: TypeRef, pointer: str
) -> tuple[list[str], TypeRef] | str:
    walked = _walk(analysis, body, pointer)
    if isinstance(walked, str):
        return f"the items pointer {pointer!r} does not resolve: {walked}"
    path, current = walked
    if current.id:
        definition = analysis.document.types.get(current.id)
        kind = definition.kind if definition is not None else "unknown"
        return (
            f"the items pointer {pointer!r} does not end at an array: "
            f"it passes through the {kind} {current.id}"
        )
    if current.kind != "array":
        return (
            f"the items pointer {pointer!r} does not end at an array: "
            f"it passes through {_describe(current)}"
        )
    item = current.raw.get("item")
    if item is None:
        return f"the items pointer {pointer!r} does not end at an array"
    return path, decode_type_ref(item, "items item")


def _resolve_next(
    analysis: Analysis, body: TypeRef, pointer: str
) -> tuple[list[str], list[str]] | str:
    walked = _walk(analysis, body, pointer)
    if isinstance(walked, str):
        return f"the next pointer {pointer!r} does not resolve: {walked}"
    path, current = walked
    wrappers: list[str] = []
    seen: set[str] = set()
    while True:
        if current.id:
            if current.variant is not None:
                return f"the next pointer {pointer!r} is not a string"
            if current.id in seen:
                return f"the next pointer {pointer!r} forms a newtype cycle"
            definition = analysis.document.types.get(current.id)
            if definition is None:
                return f"the next pointer {pointer!r} names the unknown type {current.id}"
            if definition.kind != "newtype" or definition.inner is None:
                return f"the next pointer {pointer!r} is not a string"
            seen.add(current.id)
            wrappers.append(analysis.type_names[current.id])
            current = definition.inner
            continue
        if current.kind == "nullable":
            inner = current.raw.get("inner")
            if inner is None:
                return f"the next pointer {pointer!r} has a nullable wrapper with no inner type"
            current = decode_type_ref(inner, "next cursor")
            continue
        if current.kind == "string":
            return path, wrappers
        return f"the next pointer {pointer!r} is not a string"


def _walk(
    analysis: Analysis, body: TypeRef, pointer: str
) -> tuple[list[str], TypeRef] | str:
    segments = _pointer_segments(pointer)
    if isinstance(segments, str):
        return segments
    path: list[str] = []
    current = body
    for segment in segments:
        if not current.id:
            return f"passes through {_describe(current)}"
        definition = analysis.document.types.get(current.id)
        if definition is None:
            return f"names the unknown type {current.id}"
        if definition.kind != "struct":
            return f"passes through the {definition.kind} {current.id}"
        type_plan = analysis.type_by_id.get(current.id)
        if type_plan is None:
            return f"passes through the unsupported type {current.id}"
        field = next(
            (candidate for candidate in type_plan.fields if candidate.wire == segment),
            None,
        )
        if field is None:
            return f"does not find the field {segment!r}"
        path.append(field.name)
        current = field.ty
    return path, current


def _describe(ref: TypeRef) -> str:
    if ref.kind == "nullable":
        return "a nullable wrapper"
    if ref.kind == "array":
        return "an array"
    if ref.kind == "map":
        return "a map"
    if ref.kind == "tuple":
        return "a tuple"
    if ref.kind == "struct":
        return "an inline struct"
    return f"a {ref.kind}"


def _pointer_segments(pointer: str) -> list[str] | str:
    if pointer == "":
        return []
    if not pointer.startswith("/"):
        return "is not a JSON pointer"
    segments: list[str] = []
    for raw in pointer[1:].split("/"):
        segment: list[str] = []
        index = 0
        while index < len(raw):
            character = raw[index]
            if character != "~":
                segment.append(character)
                index += 1
                continue
            if index + 1 >= len(raw):
                return "has an incomplete JSON pointer escape"
            escaped = raw[index + 1]
            if escaped == "0":
                segment.append("~")
            elif escaped == "1":
                segment.append("/")
            else:
                return "has an invalid JSON pointer escape"
            index += 2
        segments.append("".join(segment))
    return segments


def _refuse(analysis: Analysis, subject: str, message: str) -> None:
    analysis.add(UNSUPPORTED, subject, message)
    return None
