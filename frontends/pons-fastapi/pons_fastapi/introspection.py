from __future__ import annotations

from dataclasses import dataclass, field
from http import HTTPStatus
from typing import Any, Callable

from fastapi import FastAPI
from fastapi.params import File, Form
from fastapi.responses import StreamingResponse
from fastapi.routing import APIRoute

from .config import (
    PonsError,
    PonsMedia,
    PonsOperation,
    PonsStream,
    media_metadata,
    operation_metadata,
    stream_metadata,
)
from .diagnostics import diagnostic
from .media import MULTIPART, URLENCODED, content_kind, is_event_stream

_METHOD_ORDER = ("GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS")
_BODYLESS = {204, 205, 304}


class UnsupportedRoute(Exception):
    pass


@dataclass
class ParameterPlan:
    name: str
    wire: str
    annotation: Any
    required: bool
    default: Any
    field_info: Any


@dataclass
class BodyPlan:
    content: str
    required: bool
    annotation: Any | None = None
    field_info: Any = None
    fields: list[ParameterPlan] = field(default_factory=list)
    media_type: str | None = None
    diagnostics: list[dict[str, Any]] = field(default_factory=list)


@dataclass
class StreamPlan:
    events: dict[str, Any]
    media_type: str


@dataclass
class ResponsePlan:
    status: int | str
    annotation: Any | None
    content: str
    media_type: str | None = None
    description: str | None = None


@dataclass
class ErrorPlan:
    status: int | str
    annotation: Any | None
    explicit: PonsError | None = None
    description: str | None = None
    content: str | None = None
    media_type: str | None = None


@dataclass
class PathPlan:
    lit: str | None = None
    param: str | None = None
    annotation: Any | None = None


@dataclass
class QueryPlan:
    fields: list[ParameterPlan] = field(default_factory=list)
    model: Any | None = None


@dataclass
class OperationPlan:
    method: str
    path: list[PathPlan]
    query: QueryPlan
    headers: list[ParameterPlan]
    body: BodyPlan | None
    responses: list[ResponsePlan]
    errors: list[ErrorPlan]
    meta: PonsOperation | None
    endpoint: Callable[..., Any]
    multi_method: bool
    stream: StreamPlan | None = None
    operation_id: str | None = None
    summary: str | None = None
    description: str | None = None
    tags: list[str] = field(default_factory=list)
    deprecated: bool | None = None
    diagnostics: list[dict[str, Any]] = field(default_factory=list)


def inspect_app(app: FastAPI) -> tuple[list[OperationPlan], list[dict[str, Any]]]:
    plans: list[OperationPlan] = []
    diagnostics: list[dict[str, Any]] = []
    for route in app.routes:
        if not isinstance(route, APIRoute):
            continue
        try:
            plans.extend(inspect_route(route))
        except UnsupportedRoute as error:
            for method in _methods(route):
                diagnostics.append(
                    diagnostic(
                        "fastapi.unsupported_route",
                        f"{method} {route.path}: {error}",
                        severity="warning",
                    )
                )
    return plans, diagnostics


def inspect_route(route: APIRoute) -> list[OperationPlan]:
    for flag in (
        "response_model_exclude_none",
        "response_model_exclude_unset",
        "response_model_exclude_defaults",
    ):
        if getattr(route, flag, False):
            raise UnsupportedRoute(f"{flag} changes response presence at runtime")
    if getattr(route, "response_model_include", None) is not None or getattr(
        route, "response_model_exclude", None
    ) is not None:
        raise UnsupportedRoute("response_model_include/exclude is not represented")
    if getattr(route, "response_model_by_alias", True) is False:
        raise UnsupportedRoute("response_model_by_alias=False changes response wire names")

    media = media_metadata(route.endpoint)
    path = _path(route)
    query = _query(route)
    headers = [_parameter(field) for field in route.dependant.header_params]
    stream = _stream(route, stream_metadata(route.endpoint), media)
    body = _body(route, media)
    responses, errors = _responses(route, stream, media)
    metadata = operation_metadata(route.endpoint)
    methods = _methods(route)
    diagnostics = list(body.diagnostics) if body is not None else []
    return [
        OperationPlan(
            method=method,
            path=path,
            query=query,
            headers=headers,
            body=body,
            responses=responses,
            errors=errors,
            meta=metadata,
            endpoint=route.endpoint,
            multi_method=len(methods) > 1,
            stream=stream,
            operation_id=getattr(route, "operation_id", None),
            summary=getattr(route, "summary", None),
            description=getattr(route, "description", None),
            tags=list(getattr(route, "tags", []) or []),
            deprecated=getattr(route, "deprecated", None),
            diagnostics=diagnostics,
        )
        for method in methods
    ]


def _methods(route: APIRoute) -> list[str]:
    found = {method.upper() for method in route.methods}
    return [method for method in _METHOD_ORDER if method in found]


def _path(route: APIRoute) -> list[PathPlan]:
    by_name = {field.name: field for field in route.dependant.path_params}
    path: list[PathPlan] = []
    for raw in route.path.split("/"):
        if not raw:
            continue
        if raw.startswith("{") and raw.endswith("}"):
            inner = raw[1:-1]
            name = inner.split(":", 1)[0]
            field = by_name.get(name)
            if field is None:
                raise UnsupportedRoute(f"path parameter {name} has no handler field")
            parameter = _parameter(field)
            path.append(PathPlan(param=name, annotation=parameter.annotation))
        else:
            path.append(PathPlan(lit=raw))
    return path


def _query(route: APIRoute) -> QueryPlan:
    fields = route.dependant.query_params
    if not fields:
        return QueryPlan()
    models = [field for field in fields if _is_model(field.field_info.annotation)]
    if models:
        if len(fields) != 1:
            raise UnsupportedRoute("a query model combined with scalar query parameters is not represented")
        return QueryPlan(model=models[0].field_info.annotation)
    return QueryPlan(fields=[_parameter(field) for field in fields])


def _body(route: APIRoute, media: PonsMedia | None) -> BodyPlan | None:
    fields = route.dependant.body_params
    if not fields:
        return None
    declared = media.request if media is not None else None
    if any(isinstance(field.field_info, File) for field in fields):
        content_type = declared or MULTIPART
        if content_kind(content_type) != "multipart":
            raise UnsupportedRoute(
                f"file parameters write multipart bodies, not {content_type!r}"
            )
        return BodyPlan(
            content="multipart",
            required=any(field.field_info.is_required() for field in fields),
            fields=[_parameter(field) for field in fields],
            media_type=content_type,
            diagnostics=[
                diagnostic(
                    "fastapi.opaque_multipart",
                    f"{route.path} accepts multipart parts; Pons has no part schema, so the body is opaque",
                )
            ],
        )
    if all(isinstance(field.field_info, Form) for field in fields):
        content_type = declared or URLENCODED
        if content_kind(content_type) != "form":
            raise UnsupportedRoute(
                f"Form parameters write urlencoded bodies, not {content_type!r}"
            )
        return BodyPlan(
            content="form",
            required=any(field.field_info.is_required() for field in fields),
            fields=[_parameter(field) for field in fields],
            media_type=content_type,
        )
    if len(fields) > 1:
        raise UnsupportedRoute("multiple body parameters are not represented")
    field = fields[0]
    info = field.field_info
    kind = type(info).__name__
    if kind in {"Form", "File"}:
        raise UnsupportedRoute(f"{kind} request bodies are not represented")
    if getattr(info, "embed", False):
        raise UnsupportedRoute("embedded body parameters change the request shape")
    annotation = getattr(info, "annotation", None)
    if annotation is None:
        raise UnsupportedRoute("the request body has no annotation")
    content = content_kind(declared) if declared is not None else "json"
    if content in {"form", "multipart"}:
        raise UnsupportedRoute(f"a {content} request body needs Form or File parameters")
    return BodyPlan(
        content=content,
        required=info.is_required(),
        annotation=annotation,
        field_info=info,
        media_type=declared,
    )


def _stream(
    route: APIRoute,
    declaration: PonsStream | None,
    media: PonsMedia | None,
) -> StreamPlan | None:
    response_class = getattr(route, "response_class", None)
    streaming = isinstance(response_class, type) and issubclass(
        response_class, StreamingResponse
    )
    if declaration is None:
        if streaming:
            raise UnsupportedRoute(
                "a StreamingResponse needs an explicit @pons_stream(events=...) declaration"
            )
        return None
    if not streaming:
        raise UnsupportedRoute("@pons_stream requires a StreamingResponse route")
    status = _status(getattr(route, "status_code", None) or 200)
    declared = _declared_values(route).get(status)
    candidate = (
        (media.response if media is not None else None)
        or _declared_media_type(declared)
        or getattr(response_class, "media_type", None)
    )
    if candidate is None or not is_event_stream(candidate):
        raise UnsupportedRoute(
            "an SSE stream needs an explicit text/event-stream media type"
        )
    if not declaration.events:
        raise UnsupportedRoute("@pons_stream needs at least one event")
    names: set[str] = set()
    for name in declaration.events:
        if not isinstance(name, str) or not name:
            raise UnsupportedRoute("an SSE event name cannot be empty")
        if "\n" in name or "\r" in name:
            raise UnsupportedRoute("an SSE event name cannot contain a line separator")
        if name in names:
            raise UnsupportedRoute(f"SSE event {name!r} appears twice")
        names.add(name)
    return StreamPlan(events=dict(declaration.events), media_type=candidate)


def _responses(
    route: APIRoute,
    stream: StreamPlan | None,
    media: PonsMedia | None,
) -> tuple[list[ResponsePlan], list[ErrorPlan]]:
    status_value = getattr(route, "status_code", None)
    status = _status(status_value) if status_value is not None else 200
    declared = _declared_values(route)
    primary = _declared_media_type(declared.get(status))
    if primary is None and media is not None:
        primary = media.response
    if primary is None:
        primary = getattr(getattr(route, "response_class", None), "media_type", None)

    if stream is not None:
        responses = [
            ResponsePlan(status=status, annotation=None, content="stream")
        ]
        errors: list[ErrorPlan] = []
        for declared_status, value in declared.items():
            if declared_status == status:
                continue
            model = _declared_model(value)
            description = _declared_description(value)
            media_type = _declared_media_type(value) or primary
            if _bodyless_status(declared_status):
                raise UnsupportedRoute(
                    f"response {declared_status!r} cannot carry a declared body"
                )
            if model is not None:
                content = _response_content(media_type)
                if isinstance(declared_status, int) and 200 <= declared_status < 400:
                    responses.append(
                        ResponsePlan(
                            status=declared_status,
                            annotation=model,
                            content=content,
                            media_type=media_type,
                            description=description,
                        )
                    )
                else:
                    errors.append(
                        ErrorPlan(
                            status=declared_status,
                            annotation=model,
                            description=description,
                            media_type=media_type,
                        )
                    )
            elif isinstance(declared_status, int) and 200 <= declared_status < 400:
                raise UnsupportedRoute(
                    f"response {declared_status} has no model, so its body shape is unknown"
                )
            else:
                errors.append(
                    ErrorPlan(
                        status=declared_status,
                        annotation=None,
                        description=description,
                        media_type=media_type,
                    )
                )
        return responses, errors

    content = _response_content(primary)
    responses = []
    errors = []
    bodyless = status in _BODYLESS or route.methods == {"HEAD"}
    model = getattr(route, "response_model", None)
    if status in declared and _declared_model(declared[status]) is not None:
        model = _declared_model(declared[status])
    if bodyless:
        responses.append(
            ResponsePlan(status=status, annotation=None, content="empty")
        )
    elif model is not None:
        _check_text_model(content, model)
        responses.append(
            ResponsePlan(
                status=status,
                annotation=model,
                content=content,
                media_type=primary,
            )
        )
    else:
        raise UnsupportedRoute("no response model is declared")

    primary_status = responses[0].status
    for declared_status, value in declared.items():
        if declared_status == primary_status:
            continue
        if _bodyless_status(declared_status):
            raise UnsupportedRoute(f"response {declared_status!r} cannot carry a declared body")
        model = _declared_model(value)
        description = _declared_description(value)
        media_type = _declared_media_type(value) or primary
        declared_content = _response_content(media_type)
        if model is not None:
            _check_text_model(declared_content, model)
            if isinstance(declared_status, int) and 200 <= declared_status < 400:
                responses.append(
                    ResponsePlan(
                        status=declared_status,
                        annotation=model,
                        content=declared_content,
                        media_type=media_type,
                        description=description,
                    )
                )
            else:
                errors.append(
                    ErrorPlan(
                        status=declared_status,
                        annotation=model,
                        description=description,
                        media_type=media_type,
                    )
                )
        elif isinstance(declared_status, int) and 200 <= declared_status < 400:
            raise UnsupportedRoute(
                f"response {declared_status} has no model, so its body shape is unknown"
            )
        else:
            errors.append(
                ErrorPlan(
                    status=declared_status,
                    annotation=None,
                    description=description,
                    media_type=media_type,
                )
            )
    return responses, errors


def _declared_values(route: APIRoute) -> dict[int | str, Any]:
    return {
        _status(key): value for key, value in getattr(route, "responses", {}).items()
    }


def _declared_media_type(value: Any) -> str | None:
    if not isinstance(value, dict):
        return None
    media_type = value.get("media_type")
    return media_type if isinstance(media_type, str) and media_type else None


def _response_content(media_type: str | None) -> str:
    return "json" if media_type is None else content_kind(media_type)


def _check_text_model(content: str, model: Any) -> None:
    if content == "text" and model not in (str, bytes):
        raise UnsupportedRoute("a text response must use a string body")


def _status(value: Any) -> int | str:
    if isinstance(value, HTTPStatus):
        return int(value)
    if isinstance(value, int):
        return value
    if isinstance(value, str):
        lowered = value.lower()
        if lowered in {"1xx", "2xx", "3xx", "4xx", "5xx", "default"}:
            return lowered
        if value.isdigit():
            return int(value)
    raise UnsupportedRoute(f"response status {value!r} is not representable")


def _bodyless_status(status: int | str) -> bool:
    return isinstance(status, int) and (100 <= status < 200 or status in _BODYLESS)


def _declared_model(value: Any) -> Any | None:
    if not isinstance(value, dict):
        return None
    return value.get("model")


def _declared_description(value: Any) -> str | None:
    if not isinstance(value, dict):
        return None
    description = value.get("description")
    return description if isinstance(description, str) and description else None


def _parameter(field: Any) -> ParameterPlan:
    info = field.field_info
    annotation = getattr(info, "annotation", None)
    if annotation is None:
        raise UnsupportedRoute(f"parameter {field.name} has no annotation")
    alias = getattr(info, "alias", None)
    wire = alias or field.name
    return ParameterPlan(
        name=field.name,
        wire=wire,
        annotation=annotation,
        required=info.is_required(),
        default=info.default,
        field_info=info,
    )


def _is_model(annotation: Any) -> bool:
    from pydantic import BaseModel

    return isinstance(annotation, type) and issubclass(annotation, BaseModel)
