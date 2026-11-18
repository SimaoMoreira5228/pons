"""Operation plans, parameters, auth, and provides for the Python emitter."""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import TYPE_CHECKING, Any

from . import naming
from .codes import DOCUMENT, NAMING, UNSUPPORTED
from .document import (
    ErrorDef,
    Media,
    Operation,
    Provides,
    TypeRef,
    decode_type_ref,
)
from .plan_auth import CredentialPlanner
from .plan_pagination import PaginationPlan, build_pagination
from .plan_stream import StreamPlan, build_stream
from .plan_types import TypePlan, is_nullable

if TYPE_CHECKING:
    from .analysis import Analysis

ERROR_RESERVED = frozenset(
    {
        "PonsError",
        "TransportError",
        "DecodeError",
        "UnexpectedStatusError",
        "ApiError",
        "ConfigurationError",
    }
)


@dataclass(slots=True)
class ResponseHeaderPlan:
    wire: str
    ty: TypeRef
    optional: bool


@dataclass(slots=True)
class ParamPlan:
    kind: str
    name: str
    wire: str
    ty: TypeRef
    optional: bool


@dataclass(slots=True)
class BodyPlan:
    name: str
    ty: TypeRef
    required: bool
    media: str


@dataclass(slots=True)
class SuccessPlan:
    status: int
    body: TypeRef
    media: str
    headers: list[ResponseHeaderPlan]


@dataclass(slots=True)
class ErrorPlan:
    key: str
    name: str
    raw_name: list[str]
    status: object
    content: str
    body: TypeRef | None
    media: str
    message: str
    headers: list[ResponseHeaderPlan]


@dataclass(slots=True)
class OperationPlan:
    id: str
    method: str
    raw_namespace: tuple[tuple[str, ...], ...]
    raw_method: tuple[str, ...]
    path: list[tuple[str, str, TypeRef | None]]
    params: list[ParamPlan]
    query: TypePlan | None
    query_function: str
    body: BodyPlan | None
    success: SuccessPlan
    errors: list[ErrorPlan]
    alternatives: list[list[str]]
    required_auth: bool
    provides: Provides | None
    pagination: PaginationPlan | None
    doc: dict[str, Any] | None
    stream: StreamPlan | None
    namespace_path: list[str] = field(default_factory=list)
    method_name: str = ""


def parse_status(value: object, path: str) -> object:
    if isinstance(value, bool):
        raise ValueError(f"{path} is not an HTTP status")
    if isinstance(value, int):
        if value < 100 or value > 599:
            raise ValueError(f"{path} {value} is out of range")
        return value
    if isinstance(value, str):
        if value in {"default", "unknown", "1xx", "2xx", "3xx", "4xx", "5xx"}:
            return value
    raise ValueError(f"{path} is not an HTTP status")


def status_key(status: object) -> str:
    return str(status)


def is_exact_success(status: object) -> bool:
    return isinstance(status, int) and 200 <= status <= 299


def is_json_media(media: Media | None) -> bool:
    if media is None:
        return True
    if media.codec is not None:
        return False
    if media.type.lower() != "application":
        return False
    subtype = media.subtype.lower()
    return subtype == "json" or subtype.endswith("+json")


def media_text(media: Media | None) -> str:
    return media.text() if media is not None else "application/json"


def ref_signature(ref: TypeRef) -> str:
    if ref.id:
        return "ref:" + ref.id + "#" + (ref.variant or "")
    raw = ref.raw
    kind = ref.kind
    if kind in {"nullable", "array"}:
        inner = raw.get("inner") if kind == "nullable" else raw.get("item")
        return f"{kind}({ref_signature_ref(inner)})"
    if kind == "map":
        return f"map({ref_signature_ref(raw.get('key'))},{ref_signature_ref(raw.get('value'))})"
    if kind == "tuple":
        return "tuple(" + ",".join(
            ref_signature_ref(item) for item in raw.get("items", [])
        ) + ")"
    if kind == "struct":
        parts = [
            f"{field.get('wire')}={ref_signature_ref(field.get('ty'))}"
            for field in raw.get("fields", [])
        ]
        parts.append("open(" + str(raw.get("openness", "reject")) + ")")
        return "struct(" + ",".join(parts) + ")"
    return str(kind)


def ref_signature_ref(value: object) -> str:
    try:
        return ref_signature(decode_type_ref(value, "ref"))
    except Exception:
        return "invalid"


def plan_stream_media(operation: Operation) -> str | None:
    stream = operation.stream
    if stream is None:
        return None
    return stream.media or stream.content_type


class OperationPlanner:
    def __init__(self, analysis: Analysis) -> None:
        self.analysis = analysis
        self.credentials = CredentialPlanner(analysis)
        self.operations: list[OperationPlan] = []
        self.errors: dict[str, ErrorPlan] = {}
        self.error_order: list[str] = []
        self.query_types: dict[str, TypePlan] = {}
        self.query_functions: dict[str, str] = {}

    def build(self) -> None:
        for operation in self.analysis.document.operations:
            plan = self.build_operation(operation)
            if plan is not None:
                self.operations.append(plan)
        self.assign_error_names()
        self.assign_namespace_names()

    def build_operation(self, operation: Operation) -> OperationPlan | None:
        if not operation.id or any(not segment for segment in operation.id):
            self.analysis.add(
                DOCUMENT, "operations", "an operation has an empty id path"
            )
            return None
        operation_id = naming.operation_path(operation.id)
        subject = f"operations.{operation_id}"
        plan = OperationPlan(
            id=operation_id,
            method=operation.method,
            raw_namespace=tuple(tuple(segment) for segment in operation.id[:-1]),
            raw_method=tuple(operation.id[-1]),
            path=[],
            params=[],
            query=None,
            query_function="",
            body=None,
            success=None,  # type: ignore[arg-type]
            errors=[],
            alternatives=[],
            required_auth=False,
            provides=operation.provides,
            pagination=None,
            doc=operation.doc,
            stream=None,
        )
        if operation.method not in {
            "GET",
            "POST",
            "PUT",
            "PATCH",
            "DELETE",
            "HEAD",
            "OPTIONS",
        }:
            self.analysis.add(
                UNSUPPORTED,
                subject,
                f"the operation uses the unsupported HTTP method {operation.method!r}",
            )
        plan.stream = build_stream(self.analysis, operation.stream, operation, subject)
        params = naming.NameRegistry(naming.parameter_reserved())
        self.build_path(plan, operation, params, subject)
        self.build_query(plan, operation, params, subject)
        self.build_headers(plan, operation, params, subject)
        self.build_body(plan, operation, params, subject)
        success = self.build_success(operation, subject)
        if success is not None:
            plan.success = success
        else:
            plan.success = SuccessPlan(0, TypeRef(kind="any"), "application/json", [])
        plan.pagination = build_pagination(self.analysis, plan, operation, subject)
        self.build_errors(plan, operation, subject)
        self.credentials.build_auth(plan, operation, subject)
        self.credentials.build_provides(plan, operation, subject)
        return plan

    def build_path(
        self,
        plan: OperationPlan,
        operation: Operation,
        params: naming.NameRegistry,
        subject: str,
    ) -> None:
        for segment in operation.path:
            if segment.literal is not None:
                plan.path.append(("lit", segment.literal, None))
                continue
            if segment.parameter is None or segment.ty is None:
                self.analysis.add(
                    DOCUMENT, subject, "a path segment has neither lit nor param"
                )
                continue
            if is_nullable(segment.ty) or not self.analysis.type_planner.is_scalar(
                segment.ty
            ):
                self.analysis.add(
                    UNSUPPORTED,
                    subject,
                    f"the path parameter {segment.parameter} is not a scalar",
                )
                continue
            if not self.analysis.type_planner.validate_ref(
                segment.ty, subject, f"the path parameter {segment.parameter}"
            ):
                continue
            name = params.claim(
                naming.snake(
                    [naming.words_from_wire(segment.parameter)],
                    naming.parameter_reserved(),
                )
            )
            plan.params.append(
                ParamPlan("path", name, segment.parameter, segment.ty, False)
            )
            plan.path.append(("param", name, segment.ty))

    def build_query(
        self,
        plan: OperationPlan,
        operation: Operation,
        params: naming.NameRegistry,
        subject: str,
    ) -> None:
        query = operation.query
        if query is None:
            return
        if not query.id:
            self.analysis.add(
                UNSUPPORTED,
                subject,
                "the query is not a named struct, which this emitter requires",
            )
            return
        type_plan = self.analysis.type_by_id.get(query.id)
        if type_plan is None or type_plan.kind != "struct":
            self.analysis.add(
                DOCUMENT,
                subject,
                f"the query type {query.id} is not a struct",
            )
            return
        for field in type_plan.definition.fields:
            if not self.is_query_value(field.ty):
                self.analysis.add(
                    UNSUPPORTED,
                    subject,
                    f"the query field {query.id}.{field.wire} is not a scalar or a scalar array",
                )
        name = params.claim("query")
        plan.query = type_plan
        plan.query_function = self.query_function_name(type_plan)
        plan.params.append(ParamPlan("query", name, "", query, True))
        self.query_types[type_plan.id] = type_plan

    def is_query_value(self, ref: TypeRef) -> bool:
        if is_nullable(ref):
            return False
        planner = self.analysis.type_planner
        if ref.kind == "array" and not ref.id:
            item = ref.raw.get("item")
            if item is None:
                return False
            return planner.is_scalar(decode_type_ref(item, "query item"))
        return planner.is_scalar(ref)

    def query_function_name(self, plan: TypePlan) -> str:
        existing = self.query_functions.get(plan.id)
        if existing is not None:
            return existing
        name = "_query_" + naming.snake(
            [naming.words(plan.id)], naming.parameter_reserved()
        )
        used = set(self.query_functions.values())
        base = name
        index = 2
        while name in used:
            name = f"{base}_{index}"
            index += 1
        self.query_functions[plan.id] = name
        return name

    def build_headers(
        self,
        plan: OperationPlan,
        operation: Operation,
        params: naming.NameRegistry,
        subject: str,
    ) -> None:
        planner = self.analysis.type_planner
        for header in operation.headers:
            optional = header.presence == "optional"
            if optional and is_nullable(header.ty):
                self.analysis.add(
                    UNSUPPORTED,
                    subject,
                    f"the request header {header.wire} is optional and nullable, which this emitter does not support",
                )
                continue
            if is_nullable(header.ty) or not planner.is_scalar(header.ty):
                self.analysis.add(
                    UNSUPPORTED,
                    subject,
                    f"the request header {header.wire} is not a scalar",
                )
                continue
            if not planner.validate_ref(
                header.ty, subject, f"the request header {header.wire}"
            ):
                continue
            words = naming.words_from_wire(header.wire)
            name = params.claim(
                naming.snake([words], naming.parameter_reserved())
            )
            plan.params.append(
                ParamPlan("header", name, header.wire, header.ty, optional)
            )

    def build_body(
        self,
        plan: OperationPlan,
        operation: Operation,
        params: naming.NameRegistry,
        subject: str,
    ) -> None:
        body = operation.body
        if body is None:
            return
        if body.content != "json":
            self.analysis.add(
                UNSUPPORTED,
                subject,
                f"the request body content {body.content!r} is not supported; this emitter reads JSON bodies",
            )
            return
        if not is_json_media(body.media):
            self.analysis.add(
                UNSUPPORTED,
                subject,
                f"the request body uses the media type {media_text(body.media)!r}; this emitter writes JSON",
            )
            return
        if not body.required and is_nullable(body.ty):
            self.analysis.add(
                UNSUPPORTED,
                subject,
                "the optional request body is nullable, which this emitter does not support",
            )
            return
        if not self.analysis.type_planner.validate_ref(
            body.ty, subject, "the request body"
        ):
            return
        name = params.claim("body")
        plan.body = BodyPlan(name, body.ty, body.required, media_text(body.media))

    def build_success(self, operation: Operation, subject: str) -> SuccessPlan | None:
        if operation.stream is not None:
            return self.build_stream_success(operation, subject)
        if len(operation.responses) != 1:
            self.analysis.add(
                UNSUPPORTED,
                subject,
                f"the operation has {len(operation.responses)} success responses; this emitter generates one",
            )
            return None
        response = operation.responses[0]
        if not is_exact_success(response.status):
            self.analysis.add(
                UNSUPPORTED,
                subject,
                f"the success status {status_key(response.status)} is not an exact 2xx status",
            )
            return None
        if response.content != "json":
            self.analysis.add(
                UNSUPPORTED,
                subject,
                f"the success response content {response.content!r} is not supported; this emitter reads JSON bodies",
            )
            return None
        if not is_json_media(response.media):
            self.analysis.add(
                UNSUPPORTED,
                subject,
                f"the success response uses the media type {media_text(response.media)!r}; this emitter reads JSON",
            )
            return None
        if response.body is None:
            self.analysis.add(
                DOCUMENT, subject, "the JSON success response has no body type"
            )
            return None
        planner = self.analysis.type_planner
        if not planner.validate_ref(response.body, subject, "the success body"):
            return None
        headers: list[ResponseHeaderPlan] = []
        for header in response.headers:
            if is_nullable(header.ty) or not planner.is_scalar(header.ty):
                self.analysis.add(
                    UNSUPPORTED,
                    subject,
                    f"the success header {header.wire} is not a scalar",
                )
                continue
            if not planner.validate_ref(
                header.ty, subject, f"the success header {header.wire}"
            ):
                continue
            headers.append(
                ResponseHeaderPlan(header.wire, header.ty, header.presence == "optional")
            )
        status = response.status
        assert isinstance(status, int)
        return SuccessPlan(status, response.body, media_text(response.media), headers)

    def build_stream_success(
        self, operation: Operation, subject: str
    ) -> SuccessPlan | None:
        if len(operation.responses) != 1:
            self.analysis.add(
                UNSUPPORTED,
                subject,
                "the streaming operation needs exactly one success response",
            )
            return None
        response = operation.responses[0]
        if not is_exact_success(response.status):
            self.analysis.add(
                UNSUPPORTED,
                subject,
                f"the stream success status {status_key(response.status)} is not exact",
            )
            return None
        if response.content != "stream":
            self.analysis.add(
                UNSUPPORTED,
                subject,
                f"the streaming response content is {response.content!r}",
            )
            return None
        declared = media_text(response.media) if response.media is not None else ""
        media = declared or plan_stream_media(operation) or ""
        headers: list[ResponseHeaderPlan] = []
        for header in response.headers:
            if is_nullable(header.ty) or not self.analysis.type_planner.is_scalar(header.ty):
                self.analysis.add(
                    UNSUPPORTED,
                    subject,
                    f"the stream response header {header.wire} is not a scalar",
                )
                continue
            if not self.analysis.type_planner.validate_ref(
                header.ty, subject, f"the stream response header {header.wire}"
            ):
                continue
            headers.append(
                ResponseHeaderPlan(header.wire, header.ty, header.presence == "optional")
            )
        return SuccessPlan(
            int(response.status), TypeRef(kind="any"), media, headers
        )

    def build_errors(
        self, plan: OperationPlan, operation: Operation, subject: str
    ) -> None:
        errors = list(operation.errors)
        if operation.errors_inherit:
            errors.extend(self.analysis.document.common_errors)
        seen: set[str] = set()
        for error in errors:
            error_plan = self.observe_error(error, subject)
            if error_plan is None:
                continue
            if error_plan.key in seen:
                continue
            seen.add(error_plan.key)
            plan.errors.append(error_plan)
        statuses: set[str] = set()
        deduplicated: list[ErrorPlan] = []
        for error_plan in plan.errors:
            key = status_key(error_plan.status)
            if key in statuses:
                self.analysis.add(
                    DOCUMENT,
                    subject,
                    f"the operation declares more than one error for status {key}",
                )
                continue
            statuses.add(key)
            deduplicated.append(error_plan)
        plan.errors = deduplicated

    def observe_error(self, error: ErrorDef, subject: str) -> ErrorPlan | None:
        try:
            status = parse_status(error.status, "error status")
        except ValueError as failure:
            self.analysis.add(DOCUMENT, subject, str(failure))
            return None
        if status == "unknown":
            self.analysis.add(
                UNSUPPORTED,
                subject,
                "the error status unknown is not supported",
            )
            return None
        if error.content not in {"json", "empty"}:
            self.analysis.add(
                UNSUPPORTED,
                subject,
                f"the error content {error.content!r} is not supported",
            )
            return None
        if not is_json_media(error.media):
            self.analysis.add(
                UNSUPPORTED,
                subject,
                f"the error uses the media type {media_text(error.media)!r}; this emitter reads JSON",
            )
            return None
        body = error.body
        if error.content == "json":
            if body is None:
                self.analysis.add(
                    DOCUMENT, subject, "the JSON error has no body type"
                )
                return None
            if not self.analysis.type_planner.validate_ref(
                body, subject, "the error body"
            ):
                return None
        elif body is not None:
            self.analysis.add(
                DOCUMENT, subject, "the empty error declares a body type"
            )
            return None
        planner = self.analysis.type_planner
        headers: list[ResponseHeaderPlan] = []
        for header in error.headers:
            if is_nullable(header.ty) or not planner.is_scalar(header.ty):
                self.analysis.add(
                    UNSUPPORTED,
                    subject,
                    f"the error header {header.wire} is not a scalar",
                )
                continue
            if not planner.validate_ref(
                header.ty, subject, f"the error header {header.wire}"
            ):
                continue
            headers.append(
                ResponseHeaderPlan(header.wire, header.ty, header.presence == "optional")
            )
        key = (
            status_key(status)
            + "|"
            + "_".join(error.name)
            + "|"
            + error.content
            + "|"
            + (ref_signature(body) if body is not None else "")
        )
        if key not in self.errors:
            self.errors[key] = ErrorPlan(
                key=key,
                name="",
                raw_name=list(error.name),
                status=status,
                content=error.content,
                body=body,
                media=media_text(error.media),
                message=self.error_message(error),
                headers=headers,
            )
            self.error_order.append(key)
        return self.errors[key]

    def error_message(self, error: ErrorDef) -> str:
        if error.doc is not None:
            summary = error.doc.get("summary")
            if isinstance(summary, str) and summary:
                return " ".join(summary.split())
        if error.name:
            return " ".join(error.name).lower()
        return "request failed"

    def assign_error_names(self) -> None:
        planner = self.analysis.type_planner
        reserved = set(ERROR_RESERVED) | set(planner.type_names.values())
        reserved |= set(planner.variant_names.values())
        registry = naming.NameRegistry(reserved)
        for key in sorted(self.error_order):
            error = self.errors[key]
            if error.raw_name:
                base = naming.pascal([error.raw_name], ERROR_RESERVED)
            else:
                base = self.fallback_error_name(error)
            if base in registry.used:
                base += "Exception" if base.endswith("Error") else "Error"
            error.name = registry.claim(base, separator="")
        self.error_order.sort(key=lambda key: self.errors[key].name)

    def fallback_error_name(self, error: ErrorPlan) -> str:
        return "Http" + "".join(
            char for char in str(error.status) if char.isdigit()
        ) + "Error"

    def assign_namespace_names(self) -> None:
        root = naming.NameRegistry(naming.client_reserved())
        namespace_names: dict[tuple[tuple[str, ...], ...], str] = {}
        namespace_registries: dict[
            tuple[tuple[str, ...], ...], naming.NameRegistry
        ] = {}

        def registry_for(path: tuple[tuple[str, ...], ...]) -> naming.NameRegistry:
            if not path:
                return root
            return namespace_registries[path]

        for plan in self.operations:
            path: tuple[tuple[str, ...], ...] = ()
            names: list[str] = []
            for segment in plan.raw_namespace:
                key = path + (segment,)
                name = namespace_names.get(key)
                if name is None:
                    parent = registry_for(path)
                    reserved = (
                        naming.client_reserved()
                        if not path
                        else naming.namespace_reserved()
                    )
                    name = parent.claim(naming.snake([segment], reserved))
                    namespace_names[key] = name
                    namespace_registries[key] = naming.NameRegistry(
                        naming.namespace_reserved()
                    )
                names.append(name)
                path = key
            plan.namespace_path = names
            method_registry = registry_for(path)
            method_name = naming.snake([plan.raw_method], naming.field_reserved())
            plan.method_name = method_registry.claim(method_name)
        self.name_iterators(namespace_names)

    def name_iterators(
        self, namespace_names: dict[tuple[tuple[str, ...], ...], str]
    ) -> None:
        members: dict[tuple[tuple[str, ...], ...], set[str]] = {}
        for plan in self.operations:
            path = tuple(plan.raw_namespace)
            members.setdefault(path, set()).add(plan.method_name)
        for key, name in namespace_names.items():
            members.setdefault(key[:-1], set()).add(name)
        for plan in self.operations:
            if plan.pagination is None:
                continue
            path = tuple(plan.raw_namespace)
            all_name = plan.method_name + "_all"
            if all_name in members.get(path, set()):
                self.analysis.add(
                    NAMING,
                    f"operations.{plan.id}",
                    f"the iterator method {all_name} collides with another member generated in the same namespace",
                )
                continue
            plan.pagination.all_name = all_name
