"""Request body planning for the Python emitter."""

from __future__ import annotations

from dataclasses import dataclass
from typing import TYPE_CHECKING

from . import naming
from .codes import DOCUMENT, UNSUPPORTED
from .document import Body, Media, TypeRef
from .plan_types import TypePlan, is_nullable

if TYPE_CHECKING:
    from .plan_operations import OperationPlan, OperationPlanner


@dataclass(slots=True)
class BodyPlan:
    name: str
    ty: TypeRef
    required: bool
    media: str
    content: str
    encoder: str = ""


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


class BodyPlanner:
    def __init__(self, planner: OperationPlanner) -> None:
        self.planner = planner
        self.analysis = planner.analysis

    def build(
        self,
        plan: OperationPlan,
        operation: Operation,
        params: naming.NameRegistry,
        subject: str,
    ) -> None:
        body = operation.body
        if body is None:
            return
        content = body.content
        if content not in {"json", "form", "text", "bytes", "multipart"}:
            self.analysis.add(
                UNSUPPORTED,
                subject,
                f"the request body content {content!r} is not supported",
            )
            return
        if not body.required and is_nullable(body.ty):
            self.analysis.add(
                UNSUPPORTED,
                subject,
                "the optional request body is nullable, which this emitter does not support",
            )
            return
        if content == "json" and not is_json_media(body.media):
            self.analysis.add(
                UNSUPPORTED,
                subject,
                f"the JSON request body uses the media type {media_text(body.media)!r}",
            )
            return
        if content == "text":
            media = self.body_media(body.media, "text/plain")
            if not media.startswith("text/"):
                self.analysis.add(
                    UNSUPPORTED,
                    subject,
                    f"the text request body uses the media type {media!r}",
                )
                return
            if not self.is_text_value(body.ty):
                self.analysis.add(
                    UNSUPPORTED,
                    subject,
                    "the text request body is not a string",
                )
                return
        elif content == "bytes":
            if not self.is_bytes_value(body.ty):
                self.analysis.add(
                    UNSUPPORTED,
                    subject,
                    "the raw request body is not bytes",
                )
                return
        elif content == "form":
            if not self.is_form_value(body.ty, subject):
                return
        elif content == "multipart":
            if not self.is_multipart_value(body.ty, subject):
                return
        else:
            if not self.analysis.type_planner.validate_ref(
                body.ty, subject, "the request body"
            ):
                return
        media = self.body_media(body.media, self.default_media(content))
        name = params.claim("body")
        encoder = ""
        if content == "form":
            type_plan = self.analysis.type_by_id.get(body.ty.id or "")
            if type_plan is None:
                self.analysis.add(DOCUMENT, subject, "the form body type is not named")
                return
            self.planner.query_types[type_plan.id] = type_plan
            encoder = self.planner.query_function_name(type_plan)
        elif content == "multipart":
            type_plan = self.analysis.type_by_id.get(body.ty.id or "")
            if type_plan is None:
                self.analysis.add(DOCUMENT, subject, "the multipart body type is not named")
                return
            self.planner.multipart_types[type_plan.id] = type_plan
            encoder = self.multipart_function_name(type_plan)
        plan.body = BodyPlan(name, body.ty, body.required, media, content, encoder)

    def body_media(self, media: Media | None, default: str) -> str:
        return media.text() if media is not None else default

    def default_media(self, content: str) -> str:
        if content == "form":
            return "application/x-www-form-urlencoded"
        if content == "text":
            return "text/plain"
        if content == "bytes":
            return "application/octet-stream"
        return "application/json"

    def is_text_value(self, ref: TypeRef) -> bool:
        if ref.id:
            definition = self.analysis.document.types.get(ref.id)
            if definition is not None and definition.kind == "newtype":
                return definition.inner is not None and self.is_text_value(definition.inner)
            return False
        return ref.kind == "string"

    def is_bytes_value(self, ref: TypeRef) -> bool:
        if ref.id:
            definition = self.analysis.document.types.get(ref.id)
            if definition is not None and definition.kind == "newtype":
                return definition.inner is not None and self.is_bytes_value(definition.inner)
            return False
        return ref.kind == "bytes"

    def multipart_function_name(self, plan: TypePlan) -> str:
        existing = self.planner.multipart_functions.get(plan.id)
        if existing is not None:
            return existing
        name = "_multipart_" + naming.snake(
            [naming.words(plan.id)], naming.parameter_reserved()
        )
        used = set(self.planner.multipart_functions.values())
        base = name
        index = 2
        while name in used:
            name = f"{base}_{index}"
            index += 1
        self.planner.multipart_functions[plan.id] = name
        return name

    def is_multipart_value(self, ref: TypeRef, subject: str) -> bool:
        if not ref.id:
            self.analysis.add(
                UNSUPPORTED, subject, "the multipart body is not a named struct"
            )
            return False
        type_plan = self.analysis.type_by_id.get(ref.id)
        if type_plan is None or type_plan.kind != "struct":
            self.analysis.add(DOCUMENT, subject, "the multipart body type is not a struct")
            return False
        for field in type_plan.fields:
            if not self.is_multipart_field(field.ty):
                self.analysis.add(
                    UNSUPPORTED,
                    subject,
                    f"the multipart field {type_plan.id}.{field.wire} is not a scalar or bytes",
                )
                return False
        return self.analysis.type_planner.validate_ref(
            ref, subject, "the multipart body"
        )

    def is_multipart_field(self, ref: TypeRef) -> bool:
        if self.is_bytes_value(ref):
            return True
        if ref.id:
            definition = self.analysis.document.types.get(ref.id)
            if definition is not None and definition.kind == "newtype":
                return definition.inner is not None and self.is_multipart_field(definition.inner)
            return False
        return ref.kind in {"bool", "int", "float", "decimal", "string"}

    def is_form_value(self, ref: TypeRef, subject: str) -> bool:
        if not ref.id:
            self.analysis.add(
                UNSUPPORTED, subject, "the form body is not a named struct"
            )
            return False
        type_plan = self.analysis.type_by_id.get(ref.id)
        if type_plan is None or type_plan.kind != "struct":
            self.analysis.add(DOCUMENT, subject, "the form body type is not a struct")
            return False
        for field in type_plan.fields:
            if not self.planner.is_query_value(field.ty):
                self.analysis.add(
                    UNSUPPORTED,
                    subject,
                    f"the form field {type_plan.id}.{field.wire} is not a scalar or a scalar array",
                )
                return False
        return self.analysis.type_planner.validate_ref(
            ref, subject, "the form body"
        )
