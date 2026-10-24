from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, Callable, Mapping, Sequence

from fastapi import FastAPI

PONS_OPERATION_ATTR = "__pons_operation__"
PONS_STREAM_ATTR = "__pons_stream__"
PONS_MEDIA_ATTR = "__pons_media__"
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
class PonsStream:
    events: Mapping[str, Any]


@dataclass(frozen=True)
class PonsMedia:
    request: str | None = None
    response: str | None = None


@dataclass(frozen=True)
class PonsType:
    kind: str
    wire: str | None = None
    format: str | None = None
    bits: int | None = None
    signed: bool | None = None
    codec: str | None = None
    encoding: str | None = None


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


def pons_stream(*, events: Mapping[str, Any]) -> Callable[[Callable[..., Any]], Callable[..., Any]]:
    declaration = PonsStream(events=dict(events))

    def decorate(endpoint: Callable[..., Any]) -> Callable[..., Any]:
        setattr(endpoint, PONS_STREAM_ATTR, declaration)
        return endpoint

    return decorate


def stream_metadata(endpoint: Callable[..., Any]) -> PonsStream | None:
    found = getattr(endpoint, PONS_STREAM_ATTR, None)
    if found is None:
        return None
    if not isinstance(found, PonsStream):
        raise PonsConfigurationError(
            f"{endpoint.__module__}.{endpoint.__qualname__} carries a non-PonsStream {PONS_STREAM_ATTR}"
        )
    return found


def pons_media(
    *,
    request: str | None = None,
    response: str | None = None,
) -> Callable[[Callable[..., Any]], Callable[..., Any]]:
    declaration = PonsMedia(request=request, response=response)

    def decorate(endpoint: Callable[..., Any]) -> Callable[..., Any]:
        setattr(endpoint, PONS_MEDIA_ATTR, declaration)
        return endpoint

    return decorate


def media_metadata(endpoint: Callable[..., Any]) -> PonsMedia | None:
    found = getattr(endpoint, PONS_MEDIA_ATTR, None)
    if found is None:
        return None
    if not isinstance(found, PonsMedia):
        raise PonsConfigurationError(
            f"{endpoint.__module__}.{endpoint.__qualname__} carries a non-PonsMedia {PONS_MEDIA_ATTR}"
        )
    return found


_TYPE_FIELDS = {
    "bool": (),
    "int": ("bits", "signed", "wire"),
    "float": ("bits",),
    "decimal": ("wire",),
    "string": ("format",),
    "bytes": ("encoding",),
    "any": (),
    "opaque": ("codec", "wire"),
}


def pons_type(
    kind: str,
    *,
    wire: str | None = None,
    format: str | None = None,
    bits: int | None = None,
    signed: bool | None = None,
    codec: str | None = None,
    encoding: str | None = None,
) -> PonsType:
    if kind not in _TYPE_FIELDS:
        raise PonsConfigurationError(f"pons_type kind {kind!r} is not one of {sorted(_TYPE_FIELDS)}")
    declaration = PonsType(
        kind=kind,
        wire=wire,
        format=format,
        bits=bits,
        signed=signed,
        codec=codec,
        encoding=encoding,
    )
    used = {
        "wire": wire,
        "format": format,
        "bits": bits,
        "signed": signed,
        "codec": codec,
        "encoding": encoding,
    }
    unexpected = [
        name
        for name, value in used.items()
        if value is not None and name not in _TYPE_FIELDS[kind]
    ]
    if unexpected:
        raise PonsConfigurationError(
            f"pons_type kind {kind!r} does not accept {', '.join(sorted(unexpected))}"
        )
    if kind == "opaque" and not codec:
        raise PonsConfigurationError("pons_type kind 'opaque' needs a codec")
    return declaration


def app_config(app: FastAPI) -> PonsApp:
    found = getattr(app.state, PONS_APP_ATTR, None)
    if found is None:
        return PonsApp()
    if not isinstance(found, PonsApp):
        raise PonsConfigurationError(
            f"app.state.{PONS_APP_ATTR} is {type(found).__name__}, not PonsApp"
        )
    return found
