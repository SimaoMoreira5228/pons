from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, Callable, Mapping, Sequence

from fastapi import FastAPI

PONS_OPERATION_ATTR = "__pons_operation__"
PONS_APP_ATTR = "pons"


class PonsConfigurationError(ValueError):
    pass


@dataclass(frozen=True)
class PonsError:
    status: int | str
    name: str | None = None
    body: Any = None
    variant: str | None = None
    content: str | None = None
    retryable: bool | None = None
    description: str | None = None


@dataclass(frozen=True)
class PonsProvides:
    scheme: str
    value: str
    expires_at: str | None = None


@dataclass(frozen=True)
class PonsAuth:
    alternatives: tuple[tuple[str, ...], ...] = ()
    optional: bool = False
    scopes: Mapping[str, tuple[str, ...]] = field(default_factory=dict)


@dataclass(frozen=True)
class PonsOperation:
    operation_id: str | None = None
    summary: str | None = None
    description: str | None = None
    tags: tuple[str, ...] | None = None
    deprecated: bool | None = None
    auth: PonsAuth | None = None
    provides: PonsProvides | None = None
    errors: tuple[PonsError, ...] = ()
    errors_inherit: bool = True


@dataclass(frozen=True)
class PonsAuthScheme:
    kind: str
    format: str | None = None
    location: str | None = None
    name: str | None = None
    cookie: str | None = None
    description: str | None = None
    adapter: str | None = None


@dataclass(frozen=True)
class PonsApp:
    auth_schemes: Mapping[str, PonsAuthScheme] = field(default_factory=dict)
    common_errors: tuple[PonsError, ...] = ()
    servers: tuple[str, ...] | None = None


def pons_operation(
    *,
    operation_id: str | None = None,
    summary: str | None = None,
    description: str | None = None,
    tags: Sequence[str] | None = None,
    deprecated: bool | None = None,
    auth: PonsAuth | None = None,
    provides: PonsProvides | None = None,
    errors: Sequence[PonsError] = (),
    errors_inherit: bool = True,
) -> Callable[[Callable[..., Any]], Callable[..., Any]]:
    operation = PonsOperation(
        operation_id=operation_id,
        summary=summary,
        description=description,
        tags=tuple(tags) if tags is not None else None,
        deprecated=deprecated,
        auth=auth,
        provides=provides,
        errors=tuple(errors),
        errors_inherit=errors_inherit,
    )

    def decorate(endpoint: Callable[..., Any]) -> Callable[..., Any]:
        setattr(endpoint, PONS_OPERATION_ATTR, operation)
        return endpoint

    return decorate


def operation_metadata(endpoint: Callable[..., Any]) -> PonsOperation | None:
    found = getattr(endpoint, PONS_OPERATION_ATTR, None)
    if found is None:
        return None
    if not isinstance(found, PonsOperation):
        raise PonsConfigurationError(
            f"{endpoint.__module__}.{endpoint.__qualname__} carries a non-PonsOperation {PONS_OPERATION_ATTR}"
        )
    return found


def app_config(app: FastAPI) -> PonsApp:
    found = getattr(app.state, PONS_APP_ATTR, None)
    if found is None:
        return PonsApp()
    if not isinstance(found, PonsApp):
        raise PonsConfigurationError(
            f"app.state.{PONS_APP_ATTR} is {type(found).__name__}, not PonsApp"
        )
    return found
