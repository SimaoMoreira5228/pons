"""Stream plans for the Python emitter."""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import TYPE_CHECKING

from . import naming
from .codes import UNSUPPORTED
from .document import Stream, TypeRef

if TYPE_CHECKING:
    from .analysis import Analysis


@dataclass(slots=True)
class StreamEventPlan:
    wire: str
    class_name: str
    ty: TypeRef


@dataclass(slots=True)
class StreamPlan:
    kind: str
    media: str
    events: list[StreamEventPlan] = field(default_factory=list)
    item: TypeRef | None = None
    content_type: str = ""
    client: TypeRef | None = None
    server: TypeRef | None = None


def build_stream(
    analysis: Analysis, stream: Stream | None, operation: object, subject: str
) -> StreamPlan | None:
    if stream is None:
        return None
    if stream.kind == "websocket":
        if stream.client is None or stream.server is None:
            analysis.add(
                UNSUPPORTED, subject, "the WebSocket stream has no message types"
            )
            return None
        if not analysis.type_planner.validate_ref(
            stream.client, subject, "the WebSocket client message"
        ) or not analysis.type_planner.validate_ref(
            stream.server, subject, "the WebSocket server message"
        ):
            return None
        return StreamPlan(
            "websocket", "", client=stream.client, server=stream.server
        )
    if stream.kind == "sse":
        events: list[StreamEventPlan] = []
        for item in stream.events:
            if not analysis.type_planner.validate_ref(
                item.data, subject, f"the SSE event {item.event!r}"
            ):
                return None
            words = [
                *getattr(operation, "id", []),
                naming.words_from_wire(item.event),
                ["event"],
            ]
            class_name = analysis.stream_names.claim(
                naming.pascal(words, naming.type_reserved())
            )
            events.append(StreamEventPlan(item.event, class_name, item.data))
        return StreamPlan("sse", stream.media, events=events)
    if stream.kind == "ndjson":
        if stream.item is None:
            analysis.add(
                UNSUPPORTED, subject, "the NDJSON stream has no item type"
            )
            return None
        if not analysis.type_planner.validate_ref(
            stream.item, subject, "the NDJSON item"
        ):
            return None
        return StreamPlan("ndjson", stream.media, item=stream.item)
    if stream.kind == "bytes":
        return StreamPlan(
            "bytes", stream.media or stream.content_type, content_type=stream.content_type
        )
    analysis.add(
        UNSUPPORTED,
        subject,
        f"the stream kind {stream.kind!r} is not supported",
    )
    return None
