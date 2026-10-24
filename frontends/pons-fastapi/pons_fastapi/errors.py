from __future__ import annotations

from http import HTTPStatus
from typing import Any

from .config import PonsApp, PonsConfigurationError, PonsError
from .diagnostics import diagnostic
from .introspection import ErrorPlan, OperationPlan, UnsupportedRoute
from .media import error_content, media_descriptor
from .naming import error_name, ident, type_name
from .pydantic_types import OUTPUT, TypeRegistry, _is_enum, _is_model

_ERROR_CONTENT = {"json", "problem_json", "text", "empty"}


def _is_named(annotation: Any) -> bool:
    return _is_model(annotation) or _is_enum(annotation)

def _errors(plan: OperationPlan, registry: TypeRegistry) -> list[dict[str, Any]]:
    errors: dict[int | str, dict[str, Any]] = {}
    for error in plan.errors:
        converted = _error_response(error, registry)
        errors[_normalize_status(error.status)] = converted
    if plan.meta is not None:
        for error in plan.meta.errors:
            normalized = _normalize_status(error.status)
            errors[normalized] = _error_response(
                ErrorPlan(status=normalized, annotation=error.body, explicit=error),
                registry,
            )
    return list(errors.values())


def _error_response(error: ErrorPlan, registry: TypeRegistry) -> dict[str, Any]:
    explicit = error.explicit
    status = _normalize_status(explicit.status if explicit is not None else error.status)
    annotation = explicit.body if explicit is not None else error.annotation
    content = explicit.content if explicit is not None and explicit.content else None
    media_type = error.media_type
    if content is None and media_type is not None:
        try:
            content = error_content(media_type)
        except ValueError:
            raise UnsupportedRoute(
                f"error media type {media_type!r} has no IR error content"
            ) from None
    if content is None:
        content = "json" if annotation is not None else "empty"
    if content not in _ERROR_CONTENT:
        raise PonsConfigurationError(f"error content {content!r} is not an IR error content")
    if content == "empty" and annotation is not None:
        raise PonsConfigurationError("an empty error response cannot carry a body")
    if content != "empty" and annotation is None:
        raise PonsConfigurationError(f"error content {content!r} needs a body annotation")
    if annotation is not None and _body_forbidden(status):
        raise PonsConfigurationError(f"error status {status!r} cannot carry a body")
    if content == "empty" and media_type is not None:
        raise PonsConfigurationError("an empty error response cannot declare a media type")

    value: dict[str, Any] = {
        "status": status,
        "name": _error_ident(explicit, annotation, status),
        "content": content,
    }
    value["media"] = media_descriptor(media_type)
    if annotation is not None:
        ref, _ = registry.reference(
            annotation,
            OUTPUT,
            site=f"error.{status}",
        )
        if explicit is not None and explicit.variant is not None:
            if not _is_enum(annotation):
                raise PonsConfigurationError(
                    f"error variant {explicit.variant!r} narrows {annotation!r}, which is not an enum"
                )
            ref = dict(ref)
            ref["variant"] = explicit.variant
        value["body"] = ref
    description = explicit.description if explicit is not None else error.description
    if description:
        value["doc"] = {"description": description}
    if explicit is not None and explicit.retryable is not None:
        value["retryable"] = explicit.retryable
    return value


def _error_ident(
    explicit: PonsError | None,
    annotation: Any,
    status: int | str,
) -> list[str]:
    if explicit is not None and explicit.name:
        return error_name(explicit.name)
    if _is_named(annotation):
        return type_name(annotation)
    if isinstance(status, int):
        try:
            return error_name(HTTPStatus(status).phrase)
        except ValueError:
            return ["error"]
    if isinstance(status, str) and status.endswith("xx"):
        return error_name(
            {
                "1": "informational",
                "2": "success",
                "3": "redirect",
                "4": "client",
                "5": "server",
            }.get(status[0], "error")
            + " error"
        )
    return ["error"]


def _errors_inherit(
    plan: OperationPlan,
    errors: list[dict[str, Any]],
    config: PonsApp,
) -> tuple[bool, list[dict[str, Any]]]:
    inherit = plan.meta.errors_inherit if plan.meta is not None else True
    if not inherit or not config.common_errors:
        return inherit, []
    diagnostics: list[dict[str, Any]] = []
    for error in errors:
        if any(_shadows(common.status, error["status"]) for common in config.common_errors):
            inherit = False
            diagnostics.append(
                diagnostic(
                    "fastapi.common_error_collision",
                    f"operation {plan.endpoint.__qualname__} declares error {error['status']!r}, which a common error already answers for; common errors are not inherited",
                )
            )
            break
    if inherit:
        for response in plan.responses:
            if any(
                _shadows(common.status, response.status)
                for common in config.common_errors
            ):
                inherit = False
                diagnostics.append(
                    diagnostic(
                        "fastapi.common_error_collision",
                        f"operation {plan.endpoint.__qualname__} has success response {response.status!r}, which a common error already answers for; common errors are not inherited",
                    )
                )
                break
    return inherit, diagnostics


def _shadows(left: int | str, right: int | str) -> bool:
    left = _normalize_status(left)
    right = _normalize_status(right)
    if left == right or left == "default" or right == "default":
        return True
    if isinstance(left, int) and isinstance(right, str) and right.endswith("xx"):
        return left // 100 == int(right[0])
    if isinstance(right, int) and isinstance(left, str) and left.endswith("xx"):
        return right // 100 == int(left[0])
    return False


def _body_forbidden(status: int | str) -> bool:
    if isinstance(status, str):
        return status.endswith("xx") and status[0] in {"1", "2", "3"}
    return 100 <= status < 200 or status in {204, 205, 304}


def _normalize_status(value: int | str | HTTPStatus) -> int | str:
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
    raise PonsConfigurationError(f"status {value!r} is not an IR status")


