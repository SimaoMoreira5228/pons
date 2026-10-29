from __future__ import annotations

import re
from http import HTTPStatus
from typing import Any

from fastapi import FastAPI
from pydantic import BaseModel
from pydantic_core import PydanticUndefined, to_jsonable_python

from .config import (
    PonsApp,
    PonsAuthScheme,
    PonsConfigurationError,
    PonsError,
    PonsPagination,
    app_config,
)
from .diagnostics import diagnostic
from .introspection import (
    ErrorPlan,
    OperationPlan,
    ParameterPlan,
    ResponsePlan,
    UnsupportedRoute,
    inspect_app,
)
from .media import error_content, media_descriptor, opaque_type
from .naming import error_name, ident, operation_path, type_base_id, type_name
from .errors import _error_response, _errors, _errors_inherit
from .pydantic_types import INPUT, OUTPUT, TypeRegistry

_HEADER_NAME = re.compile(r"^[!#$%&'*+\-.^_`|~0-9A-Za-z]+$")
_SAFE = {"GET", "HEAD", "OPTIONS"}
_IDEMPOTENT = {"GET", "HEAD", "PUT", "DELETE", "OPTIONS"}
_ERROR_CONTENT = {"json", "problem_json", "text", "empty"}


def build_document(app: FastAPI) -> dict[str, Any]:
    config = app_config(app)
    plans, diagnostics = inspect_app(app)
    registry = TypeRegistry()
    for plan in plans:
        _collect_plan(plan, registry)
    for error in config.common_errors:
        _collect_error(error, registry)
    registry.analyze()
    registry.build()

    operations: list[dict[str, Any]] = []
    used_ids: dict[tuple[tuple[str, ...], str], str] = {}
    for plan in plans:
        try:
            operation = _operation(plan, registry, config)
        except UnsupportedRoute as error:
            diagnostics.append(
                diagnostic(
                    "fastapi.unsupported_route",
                    f"{plan.method} {_path_text(plan.path)}: {error}",
                )
            )
            continue
        key = (tuple(tuple(name) for name in operation["id"]), operation["method"])
        if key in used_ids:
            raise PonsConfigurationError(
                f"operations {used_ids[key]} and {plan.endpoint.__module__}.{plan.endpoint.__qualname__} share the id {operation['id']!r}"
            )
        used_ids[key] = (
            f"{plan.endpoint.__module__}.{plan.endpoint.__qualname__} ({plan.method})"
        )
        operations.append(operation)

    operations.sort(key=lambda value: (value["id"], value["method"]))
    common_errors = [
        _error_response(ErrorPlan(status=error.status, annotation=error.body, explicit=error), registry)
        for error in config.common_errors
    ]
    document: dict[str, Any] = {
        "ir_version": "0.2",
        "info": _info(app, config),
        "types": registry.definitions(),
        "auth_schemes": _auth_schemes(config),
        "common_errors": common_errors,
        "operations": operations,
    }
    if diagnostics:
        document["diagnostics"] = diagnostics
    return document


def _collect_plan(plan: OperationPlan, registry: TypeRegistry) -> None:
    for segment in plan.path:
        if segment.annotation is not None:
            registry.collect(segment.annotation, INPUT)
    if plan.query.model is not None:
        registry.collect(plan.query.model, INPUT)
    else:
        for parameter in plan.query.fields:
            registry.collect(parameter.annotation, INPUT)
    for header in plan.headers:
        registry.collect(header.annotation, INPUT)
    if plan.body is not None:
        if plan.body.annotation is not None:
            registry.collect(plan.body.annotation, INPUT)
        for parameter in plan.body.fields:
            registry.collect(parameter.annotation, INPUT)
    for response in plan.responses:
        if response.annotation is not None:
            registry.collect(response.annotation, OUTPUT)
    if plan.stream is not None:
        for annotation in plan.stream.events.values():
            registry.collect(annotation, OUTPUT)
    for error in plan.errors:
        if error.annotation is not None:
            registry.collect(error.annotation, OUTPUT)
    if plan.meta is not None:
        for error in plan.meta.errors:
            _collect_error(error, registry)


def _collect_error(error: PonsError, registry: TypeRegistry) -> None:
    if error.body is not None:
        registry.collect(error.body, OUTPUT)


def _info(app: FastAPI, config: PonsApp) -> dict[str, Any]:
    value: dict[str, Any] = {"title": app.title, "version": app.version}
    if app.description:
        value["description"] = app.description
    if config.servers is not None:
        servers = list(config.servers)
    else:
        servers = [
            entry["url"]
            for entry in getattr(app, "servers", [])
            if isinstance(entry, dict) and isinstance(entry.get("url"), str)
        ]
    if servers:
        value["servers"] = servers
    return value


def _auth_schemes(config: PonsApp) -> dict[str, dict[str, Any]]:
    schemes: dict[str, dict[str, Any]] = {}
    for scheme_id, scheme in config.auth_schemes.items():
        schemes[scheme_id] = _auth_scheme(scheme_id, scheme)
    return schemes


def _auth_scheme(scheme_id: str, scheme: PonsAuthScheme) -> dict[str, Any]:
    kind = scheme.kind
    if kind == "bearer":
        value: dict[str, Any] = {"kind": "bearer"}
        if scheme.format is not None:
            value["format"] = scheme.format
        return value
    if kind == "basic":
        return {"kind": "basic"}
    if kind == "api_key":
        if scheme.location not in {"header", "query", "cookie"}:
            raise PonsConfigurationError(
                f"auth scheme {scheme_id!r} needs location header, query, or cookie"
            )
        if not scheme.name:
            raise PonsConfigurationError(f"auth scheme {scheme_id!r} needs a name")
        return {"kind": "api_key", "in": scheme.location, "name": scheme.name}
    if kind == "cookie_session":
        if not scheme.cookie:
            raise PonsConfigurationError(f"auth scheme {scheme_id!r} needs a cookie")
        return {"kind": "cookie_session", "cookie": scheme.cookie}
    if kind == "custom":
        if not scheme.description:
            raise PonsConfigurationError(f"auth scheme {scheme_id!r} needs a description")
        value = {"kind": "custom", "description": scheme.description}
        if scheme.adapter is not None:
            value["adapter"] = scheme.adapter
        return value
    raise PonsConfigurationError(
        f"auth scheme {scheme_id!r} uses kind {kind!r}, which the Python frontend does not describe"
    )


def _operation(
    plan: OperationPlan,
    registry: TypeRegistry,
    config: PonsApp,
) -> dict[str, Any]:
    path = _path(plan, registry)
    query = _query(plan, registry)
    headers = _headers(plan, registry)
    diagnostics = list(plan.diagnostics)
    body = _body(plan, registry, diagnostics)
    stream = _stream(plan, registry)
    if plan.method == "HEAD":
        if (
            any(response.annotation is not None for response in plan.responses)
            or any(error.annotation is not None for error in plan.errors)
            or (
                plan.meta is not None
                and any(error.body is not None for error in plan.meta.errors)
            )
        ):
            raise UnsupportedRoute("HEAD operations cannot carry a response body")
        responses = [
            {"status": response.status, "content": "empty"}
            for response in plan.responses
        ]
    else:
        responses = [
            _response(response, registry, diagnostics) for response in plan.responses
        ]
    errors = _errors(plan, registry)
    errors_inherit, error_diagnostics = _errors_inherit(plan, errors, config)
    auth = _auth(plan, config)
    provides = _provides(plan, config)

    raw_id = (
        plan.meta.operation_id
        if plan.meta is not None and plan.meta.operation_id
        else plan.operation_id or plan.endpoint.__name__
    )
    identifiers = operation_path(raw_id)
    if plan.multi_method:
        identifiers[-1] = identifiers[-1] + [plan.method.lower()]

    operation: dict[str, Any] = {
        "id": identifiers,
        "method": plan.method,
        "path": path,
        "responses": responses,
        "errors": errors,
        "auth": auth,
        "semantics": {
            "safe": plan.method in _SAFE,
            "idempotent": plan.method in _IDEMPOTENT,
        },
    }
    pagination = _pagination(plan.meta.pagination if plan.meta is not None else None)
    if pagination is not None:
        operation["semantics"]["pagination"] = pagination
    if query is not None:
        operation["query"] = query
    if headers:
        operation["headers"] = headers
    if body is not None:
        operation["body"] = body
    if stream is not None:
        operation["stream"] = stream
    if provides is not None:
        operation["provides"] = provides
    if not errors_inherit:
        operation["errors_inherit"] = False
    doc = _operation_doc(plan)
    if doc is not None:
        operation["doc"] = doc
    diagnostics = diagnostics + error_diagnostics
    if diagnostics:
        operation["diagnostics"] = diagnostics
    return operation


def _operation_doc(plan: OperationPlan) -> dict[str, Any] | None:
    meta = plan.meta
    summary = meta.summary if meta is not None and meta.summary is not None else plan.summary
    description = (
        meta.description
        if meta is not None and meta.description is not None
        else plan.description
    )
    tags = list(meta.tags) if meta is not None and meta.tags is not None else list(plan.tags)
    deprecated = (
        meta.deprecated
        if meta is not None and meta.deprecated is not None
        else plan.deprecated
    )
    value: dict[str, Any] = {}
    if summary:
        value["summary"] = summary
    if description:
        value["description"] = description
    if tags:
        value["tags"] = tags
    if deprecated:
        value["deprecated"] = True
    return value or None


def _pagination(declaration: PonsPagination | None) -> dict[str, Any] | None:
    if declaration is None:
        return None
    value: dict[str, Any] = {
        "request": {"kind": declaration.kind, "field": declaration.field},
        "items": declaration.items,
    }
    if declaration.next is not None:
        value["next"] = declaration.next
    if declaration.page_size is not None:
        page_size: dict[str, Any] = {"field": declaration.page_size.field}
        if declaration.page_size.default is not None:
            page_size["default"] = declaration.page_size.default
        if declaration.page_size.maximum is not None:
            page_size["maximum"] = declaration.page_size.maximum
        value["page_size"] = page_size
    return value


def _path(
    plan: OperationPlan,
    registry: TypeRegistry,
) -> list[dict[str, Any]]:
    path: list[dict[str, Any]] = []
    names: set[str] = set()
    for segment in plan.path:
        if segment.lit is not None:
            path.append({"lit": segment.lit})
            continue
        name = segment.param or ""
        if name in names:
            raise UnsupportedRoute(f"path parameter {name} appears twice")
        names.add(name)
        ref, _ = registry.reference(
            segment.annotation,
            INPUT,
            site=f"{type_base_id(plan.endpoint)}.{name}",
        )
        if _is_nullable(ref):
            raise UnsupportedRoute(f"path parameter {name} is required and nullable")
        if not _is_flat_ref(ref, registry.definitions()):
            raise UnsupportedRoute(
                f"path parameter {name} uses a type that a path segment cannot encode"
            )
        path.append({"param": name, "ty": ref})
    return path


def _query(plan: OperationPlan, registry: TypeRegistry) -> dict[str, Any] | None:
    if plan.query.model is not None:
        ref, _ = registry.reference(
            plan.query.model,
            INPUT,
            site=f"{type_base_id(plan.endpoint)}::query",
        )
        if not _is_query_struct(ref, registry.definitions()):
            raise UnsupportedRoute("the query model is not an object of scalar fields")
        return ref
    if not plan.query.fields:
        return None
    type_id = f"{type_base_id(plan.endpoint)}::query"
    fields = [
        _query_field(plan, parameter, registry) for parameter in plan.query.fields
    ]
    registry.define_struct(
        type_id,
        type_name(plan.endpoint) + ["query"],
        fields,
        direction=INPUT,
        openness="reject",
    )
    return {"ref": type_id}


def _query_field(
    plan: OperationPlan,
    parameter: ParameterPlan,
    registry: TypeRegistry,
) -> dict[str, Any]:
    ref, diagnostics = registry.reference(
        parameter.annotation,
        INPUT,
        constraints=list(getattr(parameter.field_info, "metadata", [])),
        site=f"{type_base_id(plan.endpoint)}::query.{parameter.name}",
    )
    if parameter.required and _is_nullable(ref):
        raise UnsupportedRoute(
            f"query field {parameter.wire} is required and nullable; a query string has no present null"
        )
    if not _is_flat_ref(ref, registry.definitions()):
        raise UnsupportedRoute(
            f"query field {parameter.wire} uses a type that a query string cannot encode"
        )
    field: dict[str, Any] = {
        "wire": parameter.wire,
        "ident": ident(parameter.name),
        "ty": ref,
        "presence": "required" if parameter.required else "optional",
    }
    if not parameter.required:
        default = _default(parameter.default)
        if default is not PydanticUndefined:
            field["default"] = default
    description = getattr(parameter.field_info, "description", None)
    if description:
        field["doc"] = {"description": description}
    if diagnostics:
        field["diagnostics"] = diagnostics
    return field


def _headers(
    plan: OperationPlan,
    registry: TypeRegistry,
) -> list[dict[str, Any]]:
    headers: list[dict[str, Any]] = []
    names: set[str] = set()
    for parameter in plan.headers:
        wire = parameter.wire
        lowered = wire.lower()
        if not _HEADER_NAME.fullmatch(wire):
            raise UnsupportedRoute(f"header name {wire!r} is not a valid HTTP field name")
        if lowered in names:
            raise UnsupportedRoute(f"request header {wire} appears twice")
        names.add(lowered)
        if lowered in {"accept", "authorization", "cookie"} or (
            lowered == "content-type" and plan.body is not None
        ):
            raise UnsupportedRoute(f"request header {wire} is owned by the body or security model")
        ref, _ = registry.reference(
            parameter.annotation,
            INPUT,
            constraints=list(getattr(parameter.field_info, "metadata", [])),
            site=f"{type_base_id(plan.endpoint)}.header.{parameter.name}",
        )
        if parameter.required and _is_nullable(ref):
            raise UnsupportedRoute(f"request header {wire} is required and nullable")
        headers.append(
            {
                "wire": wire,
                "ident": ident(parameter.name),
                "ty": ref,
                "presence": "required" if parameter.required else "optional",
            }
        )
    return headers


def _body(
    plan: OperationPlan,
    registry: TypeRegistry,
    diagnostics: list[dict[str, Any]],
) -> dict[str, Any] | None:
    if plan.body is None:
        return None
    body = plan.body
    if body.content == "form":
        type_id = f"{type_base_id(plan.endpoint)}::form"
        fields = [
            _form_field(plan, parameter, registry) for parameter in body.fields
        ]
        registry.define_struct(
            type_id,
            type_name(plan.endpoint) + ["form"],
            fields,
            direction=INPUT,
            openness="reject",
        )
        return {
            "ty": {"ref": type_id},
            "content": "form",
            "required": body.required,
            "media": media_descriptor(body.media_type),
        }
    if body.content == "multipart":
        return {
            "ty": opaque_type("fastapi.multipart", "unknown"),
            "content": "multipart",
            "required": body.required,
            "media": media_descriptor(body.media_type),
        }
    ref, _ = registry.reference(
        body.annotation,
        INPUT,
        constraints=list(getattr(body.field_info, "metadata", [])),
        site=f"{type_base_id(plan.endpoint)}::body",
    )
    ty = _typed_body(
        ref,
        body.content,
        "fastapi.request",
        registry.definitions(),
        diagnostics,
        "request body",
    )
    value: dict[str, Any] = {
        "ty": ty,
        "content": body.content,
        "required": body.required,
    }
    value["media"] = media_descriptor(body.media_type)
    return value


def _form_field(
    plan: OperationPlan,
    parameter: ParameterPlan,
    registry: TypeRegistry,
) -> dict[str, Any]:
    ref, diagnostics = registry.reference(
        parameter.annotation,
        INPUT,
        constraints=list(getattr(parameter.field_info, "metadata", [])),
        site=f"{type_base_id(plan.endpoint)}::form.{parameter.name}",
    )
    if parameter.required and _is_nullable(ref):
        raise UnsupportedRoute(
            f"form field {parameter.wire} is required and nullable; a urlencoded form has no present null"
        )
    if not _is_flat_ref(ref, registry.definitions(), array=True):
        raise UnsupportedRoute(
            f"form field {parameter.wire} uses a type that a urlencoded form cannot encode"
        )
    field: dict[str, Any] = {
        "wire": parameter.wire,
        "ident": ident(parameter.name),
        "ty": ref,
        "presence": "required" if parameter.required else "optional",
    }
    if not parameter.required:
        default = _default(parameter.default)
        if default is not PydanticUndefined:
            field["default"] = default
    description = getattr(parameter.field_info, "description", None)
    if description:
        field["doc"] = {"description": description}
    if diagnostics:
        field["diagnostics"] = diagnostics
    return field


def _stream(plan: OperationPlan, registry: TypeRegistry) -> dict[str, Any] | None:
    if plan.stream is None:
        return None
    events = []
    for name in sorted(plan.stream.events):
        ref, _ = registry.reference(
            plan.stream.events[name],
            OUTPUT,
            site=f"{type_base_id(plan.endpoint)}::stream.{name}",
        )
        events.append({"event": name, "data": ref})
    return {"kind": "sse", "events": events}


def _typed_body(
    ref: dict[str, Any],
    content: str,
    codec: str,
    definitions: dict[str, dict[str, Any]],
    diagnostics: list[dict[str, Any]],
    label: str,
) -> dict[str, Any]:
    if content == "bytes" and not _is_bytes_ref(ref, definitions):
        diagnostics.append(
            diagnostic(
                "fastapi.opaque_media",
                f"the {label} declares a byte media type, but {ref!r} is not a byte type; the body is opaque",
            )
        )
        return opaque_type(codec, "bytes")
    if content == "text" and not _is_string_ref(ref, definitions):
        diagnostics.append(
            diagnostic(
                "fastapi.opaque_media",
                f"the {label} declares a text media type, but {ref!r} is not a string type; the body is opaque",
            )
        )
        return opaque_type(codec, "text")
    return ref


def _response(
    response: ResponsePlan,
    registry: TypeRegistry,
    diagnostics: list[dict[str, Any]],
) -> dict[str, Any]:
    value: dict[str, Any] = {"status": response.status, "content": response.content}
    value["media"] = media_descriptor(response.media_type)
    if response.annotation is not None:
        ref, _ = registry.reference(
            response.annotation,
            OUTPUT,
            site=f"response.{response.status}",
        )
        value["body"] = _typed_body(
            ref,
            response.content,
            "fastapi.response",
            registry.definitions(),
            diagnostics,
            f"response {response.status}",
        )
    return value


def _auth(plan: OperationPlan, config: PonsApp) -> dict[str, Any]:
    if plan.meta is None or plan.meta.auth is None:
        return {"alternatives": [], "optional": False}
    requirement = plan.meta.auth
    alternatives: list[list[str]] = []
    for alternative in requirement.alternatives:
        schemes = [scheme for scheme in alternative]
        if not schemes:
            raise PonsConfigurationError("an auth alternative cannot be empty")
        for scheme in schemes:
            if scheme not in config.auth_schemes:
                raise PonsConfigurationError(
                    f"operation {plan.endpoint.__qualname__} names unknown auth scheme {scheme!r}"
                )
        alternatives.append(schemes)
    scopes: dict[str, list[str]] = {}
    for scheme, values in requirement.scopes.items():
        if scheme not in config.auth_schemes:
            raise PonsConfigurationError(
                f"operation {plan.endpoint.__qualname__} scopes unknown auth scheme {scheme!r}"
            )
        if not any(scheme in alternative for alternative in alternatives):
            raise PonsConfigurationError(
                f"operation {plan.endpoint.__qualname__} scopes {scheme!r} without requiring it"
            )
        if not values:
            raise PonsConfigurationError(
                f"operation {plan.endpoint.__qualname__} declares an empty scope list for {scheme!r}"
            )
        scopes[scheme] = [value for value in values]
    value: dict[str, Any] = {
        "alternatives": alternatives,
        "optional": requirement.optional,
    }
    if scopes:
        value["scopes"] = scopes
    return value


def _provides(plan: OperationPlan, config: PonsApp) -> dict[str, Any] | None:
    if plan.meta is None or plan.meta.provides is None:
        return None
    provides = plan.meta.provides
    if provides.scheme not in config.auth_schemes:
        raise PonsConfigurationError(
            f"operation {plan.endpoint.__qualname__} provides unknown auth scheme {provides.scheme!r}"
        )
    value: dict[str, Any] = {
        "scheme": provides.scheme,
        "value": _locator(provides.value),
    }
    if provides.expires_at is not None:
        value["expires_at"] = _locator(provides.expires_at)
    return value


def _locator(text: str) -> dict[str, Any]:
    source, separator, name = text.partition(":")
    if not separator or not name:
        raise PonsConfigurationError(
            f"credential locator {text!r} must be body:/pointer, header:Name, or set_cookie:Name"
        )
    if source == "body":
        return {"from": "body", "pointer": name}
    if source == "header":
        return {"from": "header", "name": name}
    if source == "set_cookie":
        return {"from": "set_cookie", "name": name}
    raise PonsConfigurationError(f"credential locator {text!r} has an unknown source")


def _default(value: Any) -> Any:
    if value is PydanticUndefined:
        return PydanticUndefined
    try:
        return to_jsonable_python(value)
    except Exception:
        return PydanticUndefined


def _is_nullable(ref: dict[str, Any]) -> bool:
    return ref.get("kind") == "nullable"


def _is_flat_ref(
    ref: dict[str, Any],
    definitions: dict[str, dict[str, Any]],
    *,
    array: bool = False,
) -> bool:
    kind = ref.get("kind")
    if kind == "nullable":
        return _is_flat_ref(ref["inner"], definitions, array=array)
    if kind == "array":
        return array and _is_flat_ref(ref["item"], definitions)
    if kind in {"bool", "int", "float", "decimal", "string"}:
        return True
    if kind == "bytes":
        return ref.get("encoding") in {"base64", "base64url", "hex"}
    if "ref" not in ref:
        return False
    definition = definitions.get(ref["ref"])
    if definition is None:
        return False
    if definition["kind"] == "enum":
        return definition["repr"]["kind"] == "string"
    if definition["kind"] == "newtype":
        return _is_flat_ref(definition["inner"], definitions, array=array)
    return False


def _is_bytes_ref(ref: dict[str, Any], definitions: dict[str, dict[str, Any]]) -> bool:
    kind = ref.get("kind")
    if kind == "nullable":
        return _is_bytes_ref(ref["inner"], definitions)
    if kind == "bytes":
        return True
    if kind == "opaque":
        return ref.get("wire") == "bytes"
    if "ref" not in ref:
        return False
    definition = definitions.get(ref["ref"])
    if definition is None or definition["kind"] != "newtype":
        return False
    return _is_bytes_ref(definition["inner"], definitions)


def _is_string_ref(ref: dict[str, Any], definitions: dict[str, dict[str, Any]]) -> bool:
    kind = ref.get("kind")
    if kind == "nullable":
        return _is_string_ref(ref["inner"], definitions)
    if kind in {"string", "bytes"}:
        return True
    if kind == "opaque":
        return ref.get("wire") in {"text", "bytes"}
    if "ref" not in ref:
        return False
    definition = definitions.get(ref["ref"])
    if definition is None:
        return False
    if definition["kind"] == "enum":
        return definition["repr"]["kind"] == "string"
    if definition["kind"] == "newtype":
        return _is_string_ref(definition["inner"], definitions)
    return False


def _is_query_struct(ref: dict[str, Any], definitions: dict[str, dict[str, Any]]) -> bool:
    if "ref" not in ref:
        return False
    definition = definitions.get(ref["ref"])
    if definition is None or definition["kind"] != "struct":
        return False
    if definition.get("openness") != "reject" or definition.get("flattened"):
        return False
    return all(
        not (field["presence"] == "required" and _is_nullable(field["ty"]))
        and _is_flat_ref(field["ty"], definitions)
        for field in definition.get("fields", [])
    )


def _is_model(annotation: Any) -> bool:
    return isinstance(annotation, type) and issubclass(annotation, BaseModel)


def _is_enum(annotation: Any) -> bool:
    import enum

    return isinstance(annotation, type) and issubclass(annotation, enum.Enum)


def _is_named(annotation: Any) -> bool:
    return _is_model(annotation) or _is_enum(annotation)


def _path_text(path: list[Any]) -> str:
    parts = []
    for segment in path:
        if segment.lit is not None:
            parts.append(segment.lit)
        else:
            parts.append("{" + (segment.param or "") + "}")
    return "/" + "/".join(parts)
