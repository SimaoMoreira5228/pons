"""The IR 0.3 document subset the Python emitter reads."""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any


class IrError(Exception):
    def __init__(self, message: str, origin: str = "") -> None:
        super().__init__(message)
        self.message = message
        self.origin = origin


def _mapping(value: object, path: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise IrError(f"{path} must be an object", path)
    return value


def _sequence(value: object, path: str) -> list[Any]:
    if not isinstance(value, list):
        raise IrError(f"{path} must be an array", path)
    return value


def _text(value: object, path: str, *, default: str | None = None) -> str:
    if value is None and default is not None:
        return default
    if not isinstance(value, str):
        raise IrError(f"{path} must be a string", path)
    return value


def _flag(value: object, path: str, *, default: bool = False) -> bool:
    if value is None:
        return default
    if not isinstance(value, bool):
        raise IrError(f"{path} must be a boolean", path)
    return value


def _integer(value: object, path: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int):
        raise IrError(f"{path} must be an integer", path)
    return value


def _name(value: object, path: str) -> list[str]:
    items = _sequence(value, path)
    result: list[str] = []
    for index, item in enumerate(items):
        result.append(_text(item, f"{path}[{index}]"))
    return result


@dataclass(slots=True)
class TypeRef:
    id: str | None = None
    variant: str | None = None
    kind: str | None = None
    raw: dict[str, Any] = field(default_factory=dict)


@dataclass(slots=True)
class PathSegment:
    literal: str | None = None
    parameter: str | None = None
    ty: TypeRef | None = None


@dataclass(slots=True)
class Media:
    type: str
    subtype: str
    parameters: dict[str, str]
    codec: str | None

    def text(self) -> str:
        value = f"{self.type}/{self.subtype}"
        for name in sorted(self.parameters):
            value += f"; {name}={self.parameters[name]}"
        return value


@dataclass(slots=True)
class Field:
    wire: str
    ident: list[str]
    ty: TypeRef
    presence: str
    access: str | None


@dataclass(slots=True)
class Variant:
    wire: str
    ident: list[str]
    payload_kind: str
    payload_ty: TypeRef | None
    payload_items: list[TypeRef]
    payload_fields: list[Field]


@dataclass(slots=True)
class TypeDef:
    id: str
    name: list[str]
    direction: str
    kind: str
    fields: list[Field]
    flattened: list[TypeRef]
    openness: object
    repr_kind: str | None
    tag: str
    content: str
    variants: list[Variant]
    inner: TypeRef | None


@dataclass(slots=True)
class HeaderParameter:
    wire: str
    ident: list[str]
    ty: TypeRef
    presence: str


@dataclass(slots=True)
class Body:
    ty: TypeRef
    content: str
    required: bool
    media: Media | None


@dataclass(slots=True)
class ResponseDef:
    status: object
    content: str
    media: Media | None
    body: TypeRef | None
    headers: list[HeaderParameter]


@dataclass(slots=True)
class ErrorDef:
    status: object
    name: list[str]
    content: str
    media: Media | None
    body: TypeRef | None
    headers: list[HeaderParameter]
    doc: dict[str, Any] | None


@dataclass(slots=True)
class Locator:
    source: str
    value: str


@dataclass(slots=True)
class Provides:
    scheme: str
    value: Locator
    expires_at: Locator | None


@dataclass(slots=True)
class AuthRequirement:
    alternatives: list[list[str]]
    optional: bool


@dataclass(slots=True)
class AuthScheme:
    kind: str
    location: str = ""
    name: str = ""


@dataclass(slots=True)
class PaginationRequest:
    kind: str
    field: str


@dataclass(slots=True)
class PageSize:
    field: str
    default: int | None
    maximum: int | None


@dataclass(slots=True)
class Pagination:
    request: PaginationRequest
    items: str
    next: str | None
    page_size: PageSize | None


@dataclass(slots=True)
class Operation:
    id: list[list[str]]
    method: str
    path: list[PathSegment]
    query: TypeRef | None
    headers: list[HeaderParameter]
    body: Body | None
    responses: list[ResponseDef]
    errors: list[ErrorDef]
    errors_inherit: bool
    stream: bool
    auth: AuthRequirement
    provides: Provides | None
    pagination: Pagination | None
    doc: dict[str, Any] | None


@dataclass(slots=True)
class Document:
    ir_version: str
    title: str
    version: str
    types: dict[str, TypeDef]
    auth_schemes: dict[str, AuthScheme]
    common_errors: list[ErrorDef]
    operations: list[Operation]


def decode_type_ref(value: object, path: str) -> TypeRef:
    data = _mapping(value, path)
    if "ref" in data:
        if "kind" in data:
            raise IrError(f"{path} has both ref and kind", path)
        ref = TypeRef(id=_text(data["ref"], f"{path}.ref"))
        if data.get("variant") is not None:
            ref.variant = _text(data["variant"], f"{path}.variant")
        return ref
    if "kind" in data:
        return TypeRef(kind=_text(data["kind"], f"{path}.kind"), raw=data)
    raise IrError(f"{path} needs ref or kind", path)


def _decode_media(value: object, path: str) -> Media | None:
    if value is None:
        return None
    data = _mapping(value, path)
    parameters: dict[str, str] = {}
    raw_parameters = data.get("parameters")
    if raw_parameters is not None:
        for key, item in _mapping(raw_parameters, f"{path}.parameters").items():
            parameters[_text(key, f"{path}.parameters")] = _text(
                item, f"{path}.parameters.{key}"
            )
    codec = None
    if data.get("codec") is not None:
        codec = _text(data["codec"], f"{path}.codec")
    return Media(
        type=_text(data.get("type"), f"{path}.type"),
        subtype=_text(data.get("subtype"), f"{path}.subtype"),
        parameters=parameters,
        codec=codec,
    )


def _decode_field(value: object, path: str) -> Field:
    data = _mapping(value, path)
    return Field(
        wire=_text(data.get("wire"), f"{path}.wire"),
        ident=_name(data.get("ident", []), f"{path}.ident"),
        ty=decode_type_ref(data.get("ty"), f"{path}.ty"),
        presence=_text(data.get("presence"), f"{path}.presence"),
        access=None if data.get("access") is None else _text(data["access"], f"{path}.access"),
    )


def _decode_variant(value: object, path: str) -> Variant:
    data = _mapping(value, path)
    payload = _mapping(data.get("payload"), f"{path}.payload")
    kind = _text(payload.get("kind"), f"{path}.payload.kind")
    ty = None
    if kind == "newtype":
        ty = decode_type_ref(payload.get("ty"), f"{path}.payload.ty")
    items: list[TypeRef] = []
    if kind == "tuple":
        for index, item in enumerate(_sequence(payload.get("items"), f"{path}.payload.items")):
            items.append(decode_type_ref(item, f"{path}.payload.items[{index}]"))
    fields: list[Field] = []
    if kind == "struct":
        for index, item in enumerate(_sequence(payload.get("fields"), f"{path}.payload.fields")):
            fields.append(_decode_field(item, f"{path}.payload.fields[{index}]"))
    return Variant(
        wire=_text(data.get("wire"), f"{path}.wire"),
        ident=_name(data.get("ident", []), f"{path}.ident"),
        payload_kind=kind,
        payload_ty=ty,
        payload_items=items,
        payload_fields=fields,
    )


def _decode_type(value: object, type_id: str) -> TypeDef:
    path = f"types.{type_id}"
    data = _mapping(value, path)
    kind = _text(data.get("kind"), f"{path}.kind")
    fields: list[Field] = []
    flattened: list[TypeRef] = []
    openness: object = "reject"
    repr_kind: str | None = None
    tag: str = ""
    content: str = ""
    variants: list[Variant] = []
    inner: TypeRef | None = None
    if kind == "struct":
        for index, item in enumerate(_sequence(data.get("fields", []), f"{path}.fields")):
            fields.append(_decode_field(item, f"{path}.fields[{index}]"))
        for index, item in enumerate(
            _sequence(data.get("flattened", []), f"{path}.flattened")
        ):
            flattened.append(decode_type_ref(item, f"{path}.flattened[{index}]"))
        openness = data.get("openness", "reject")
    elif kind == "enum":
        repr_data = _mapping(data.get("repr"), f"{path}.repr")
        repr_kind = _text(repr_data.get("kind"), f"{path}.repr.kind")
        tag = repr_data.get("tag", "")
        content = repr_data.get("content", "")
        for index, item in enumerate(_sequence(data.get("variants"), f"{path}.variants")):
            variants.append(_decode_variant(item, f"{path}.variants[{index}]"))
    elif kind == "newtype":
        inner = decode_type_ref(data.get("inner"), f"{path}.inner")
    return TypeDef(
        id=type_id,
        name=_name(data.get("name"), f"{path}.name"),
        direction=_text(data.get("direction"), f"{path}.direction"),
        kind=kind,
        fields=fields,
        flattened=flattened,
        openness=openness,
        repr_kind=repr_kind,
        tag=_text(tag, f"{path}.repr.tag", default=""),
        content=_text(content, f"{path}.repr.content", default=""),
        variants=variants,
        inner=inner,
    )


def _decode_header(value: object, path: str) -> HeaderParameter:
    data = _mapping(value, path)
    return HeaderParameter(
        wire=_text(data.get("wire"), f"{path}.wire"),
        ident=_name(data.get("ident", []), f"{path}.ident"),
        ty=decode_type_ref(data.get("ty"), f"{path}.ty"),
        presence=_text(data.get("presence"), f"{path}.presence"),
    )


def _decode_body(value: object, path: str) -> Body:
    data = _mapping(value, path)
    return Body(
        ty=decode_type_ref(data.get("ty"), f"{path}.ty"),
        content=_text(data.get("content"), f"{path}.content"),
        required=_flag(data.get("required"), f"{path}.required"),
        media=_decode_media(data.get("media"), f"{path}.media"),
    )


def _decode_response(value: object, path: str) -> ResponseDef:
    data = _mapping(value, path)
    headers = [
        _decode_header(item, f"{path}.headers[{index}]")
        for index, item in enumerate(_sequence(data.get("headers", []), f"{path}.headers"))
    ]
    return ResponseDef(
        status=data.get("status"),
        content=_text(data.get("content"), f"{path}.content"),
        media=_decode_media(data.get("media"), f"{path}.media"),
        body=None
        if data.get("body") is None
        else decode_type_ref(data["body"], f"{path}.body"),
        headers=headers,
    )


def _decode_error(value: object, path: str) -> ErrorDef:
    data = _mapping(value, path)
    headers = [
        _decode_header(item, f"{path}.headers[{index}]")
        for index, item in enumerate(_sequence(data.get("headers", []), f"{path}.headers"))
    ]
    doc = data.get("doc")
    return ErrorDef(
        status=data.get("status"),
        name=_name(data.get("name"), f"{path}.name"),
        content=_text(data.get("content"), f"{path}.content"),
        media=_decode_media(data.get("media"), f"{path}.media"),
        body=None
        if data.get("body") is None
        else decode_type_ref(data["body"], f"{path}.body"),
        headers=headers,
        doc=_mapping(doc, f"{path}.doc") if doc is not None else None,
    )


def _decode_locator(value: object, path: str) -> Locator:
    data = _mapping(value, path)
    source = _text(data.get("from"), f"{path}.from")
    if source == "body":
        return Locator(source, _text(data.get("pointer"), f"{path}.pointer"))
    if source in {"header", "set_cookie"}:
        return Locator(source, _text(data.get("name"), f"{path}.name"))
    raise IrError(f"{path}.from is {source!r}", path)


def _decode_provides(value: object, path: str) -> Provides:
    data = _mapping(value, path)
    return Provides(
        scheme=_text(data.get("scheme"), f"{path}.scheme"),
        value=_decode_locator(data.get("value"), f"{path}.value"),
        expires_at=None
        if data.get("expires_at") is None
        else _decode_locator(data["expires_at"], f"{path}.expires_at"),
    )


def _decode_page_size(value: object, path: str) -> PageSize:
    data = _mapping(value, path)
    default = data.get("default")
    maximum = data.get("maximum")
    return PageSize(
        field=_text(data.get("field"), f"{path}.field"),
        default=None if default is None else _integer(default, f"{path}.default"),
        maximum=None if maximum is None else _integer(maximum, f"{path}.maximum"),
    )


def _decode_pagination(value: object, path: str) -> Pagination:
    data = _mapping(value, path)
    request = _mapping(data.get("request"), f"{path}.request")
    kind = _text(request.get("kind"), f"{path}.request.kind")
    if kind not in {"cursor", "offset"}:
        raise IrError(f"{path}.request.kind is {kind!r}", f"{path}.request.kind")
    page_size = data.get("page_size")
    return Pagination(
        request=PaginationRequest(
            kind=kind,
            field=_text(request.get("field"), f"{path}.request.field"),
        ),
        items=_text(data.get("items"), f"{path}.items"),
        next=None if data.get("next") is None else _text(data["next"], f"{path}.next"),
        page_size=None
        if page_size is None
        else _decode_page_size(page_size, f"{path}.page_size"),
    )


def _decode_operation(value: object, path: str) -> Operation:
    data = _mapping(value, path)
    segments: list[list[str]] = []
    for index, item in enumerate(_sequence(data.get("id"), f"{path}.id")):
        segments.append(_name(item, f"{path}.id[{index}]"))
    path_segments: list[PathSegment] = []
    for index, item in enumerate(_sequence(data.get("path"), f"{path}.path")):
        segment_path = f"{path}.path[{index}]"
        segment = _mapping(item, segment_path)
        if segment.get("lit") is not None:
            path_segments.append(
                PathSegment(literal=_text(segment["lit"], f"{segment_path}.lit"))
            )
        elif segment.get("param") is not None and segment.get("ty") is not None:
            path_segments.append(
                PathSegment(
                    parameter=_text(segment["param"], f"{segment_path}.param"),
                    ty=decode_type_ref(segment["ty"], f"{segment_path}.ty"),
                )
            )
        else:
            raise IrError(f"{segment_path} needs lit or param with ty", segment_path)
    query = None
    if data.get("query") is not None:
        query = decode_type_ref(data["query"], f"{path}.query")
    headers = [
        _decode_header(item, f"{path}.headers[{index}]")
        for index, item in enumerate(_sequence(data.get("headers", []), f"{path}.headers"))
    ]
    body = None
    if data.get("body") is not None:
        body = _decode_body(data["body"], f"{path}.body")
    responses = [
        _decode_response(item, f"{path}.responses[{index}]")
        for index, item in enumerate(_sequence(data.get("responses", []), f"{path}.responses"))
    ]
    errors = [
        _decode_error(item, f"{path}.errors[{index}]")
        for index, item in enumerate(_sequence(data.get("errors", []), f"{path}.errors"))
    ]
    auth_data = _mapping(data.get("auth", {}), f"{path}.auth")
    alternatives: list[list[str]] = []
    for index, item in enumerate(
        _sequence(auth_data.get("alternatives", []), f"{path}.auth.alternatives")
    ):
        alternatives.append(_name(item, f"{path}.auth.alternatives[{index}]"))
    semantics = _mapping(data.get("semantics", {}), f"{path}.semantics")
    doc = data.get("doc")
    return Operation(
        id=segments,
        method=_text(data.get("method"), f"{path}.method"),
        path=path_segments,
        query=query,
        headers=headers,
        body=body,
        responses=responses,
        errors=errors,
        errors_inherit=_flag(data.get("errors_inherit"), f"{path}.errors_inherit", default=True),
        stream=data.get("stream") is not None,
        auth=AuthRequirement(
            alternatives=alternatives,
            optional=_flag(auth_data.get("optional"), f"{path}.auth.optional"),
        ),
        provides=None
        if data.get("provides") is None
        else _decode_provides(data["provides"], f"{path}.provides"),
        pagination=None
        if semantics.get("pagination") is None
        else _decode_pagination(semantics["pagination"], f"{path}.semantics.pagination"),
        doc=_mapping(doc, f"{path}.doc") if doc is not None else None,
    )


def _decode_scheme(value: object, path: str) -> AuthScheme:
    data = _mapping(value, path)
    kind = _text(data.get("kind"), f"{path}.kind")
    if kind == "api_key":
        return AuthScheme(
            kind=kind,
            location=_text(data.get("in"), f"{path}.in"),
            name=_text(data.get("name"), f"{path}.name"),
        )
    if kind == "cookie_session":
        return AuthScheme(
            kind=kind,
            location="cookie",
            name=_text(data.get("cookie"), f"{path}.cookie"),
        )
    return AuthScheme(kind=kind)


def decode(value: object) -> Document:
    data = _mapping(value, "document")
    ir_version = _text(data.get("ir_version"), "ir_version")
    if ir_version != "0.3":
        raise IrError(
            f"unsupported IR version {ir_version!r}; this emitter reads 0.3", "ir_version"
        )
    info = _mapping(data.get("info", {}), "info")
    types: dict[str, TypeDef] = {}
    for type_id, definition in _mapping(data.get("types", {}), "types").items():
        types[type_id] = _decode_type(definition, type_id)
    schemes: dict[str, AuthScheme] = {}
    for scheme_id, scheme in _mapping(data.get("auth_schemes", {}), "auth_schemes").items():
        schemes[scheme_id] = _decode_scheme(scheme, f"auth_schemes.{scheme_id}")
    common_errors = [
        _decode_error(item, f"common_errors[{index}]")
        for index, item in enumerate(_sequence(data.get("common_errors", []), "common_errors"))
    ]
    operations = [
        _decode_operation(item, f"operations[{index}]")
        for index, item in enumerate(_sequence(data.get("operations", []), "operations"))
    ]
    return Document(
        ir_version=ir_version,
        title=_text(info.get("title"), "info.title", default="the API"),
        version=_text(info.get("version"), "info.version", default=""),
        types=types,
        auth_schemes=schemes,
        common_errors=common_errors,
        operations=operations,
    )
