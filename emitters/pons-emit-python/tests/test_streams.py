import importlib
import json
import sys

import pytest

import support


class Generated:
    def __init__(self, directory, document, package, *, mode="strict", options=None):
        merged = {"package": package}
        if options is not None:
            merged.update(options)
        self.response = support.emit(document, mode=mode, options=merged)
        assert self.response.get("error") is None, self.response.get("error")
        support.write_files(self.response, directory)
        self.package = package
        self.directory = directory
        sys.path.insert(0, str(directory))
        importlib.invalidate_caches()
        self.api = importlib.import_module(package)

    def close(self):
        sys.path.remove(str(self.directory))
        for name in list(sys.modules):
            if name == self.package or name.startswith(self.package + "."):
                del sys.modules[name]
        importlib.invalidate_caches()


def sse_document():
    return support.load_spec("login.json")


def response(api, status, content_type, chunks, extra=None):
    headers = {"Content-Type": content_type}
    if extra:
        headers.update(extra)
    return api.StreamResponse(status, headers, chunks)


def client(api, transport, **options):
    return api.Client(
        "https://api.test",
        credentials={"tenant": "t"},
        stream_transport=transport,
        **options,
    )


@pytest.fixture
def sse(tmp_path):
    generated = Generated(tmp_path, sse_document(), "sse_api", mode="lossy")
    yield generated
    generated.close()


def test_sse_stream_yields_typed_events(sse):
    api = sse.api
    events = list(
        client(
            api,
            lambda request: response(
                api,
                200,
                "text/event-stream",
                [b'event: user\ndata: {"id":"7"}\n\n'],
                {"X-Trace-ID": "trace"},
            ),
        ).events.watch("cursor-1")
    )
    assert len(events) == 1
    assert events[0].event == "user"
    assert events[0].data.id == 7


def test_sse_frames_split_across_chunks(sse):
    api = sse.api
    chunks = [
        b"event: user\ndata: {\"id\":",
        b" \"7\"}\n\n",
    ]
    events = list(
        client(
            api,
            lambda request: response(
                api, 200, "text/event-stream", chunks, {"X-Trace-ID": "t"}
            ),
        ).events.watch("c")
    )
    assert [event.data.id for event in events] == [7]


def test_unknown_sse_event_raises(sse):
    api = sse.api
    with pytest.raises(api.DecodeError, match="unknown SSE event"):
        list(
            client(
                api,
                lambda request: response(
                    api,
                    200,
                    "text/event-stream",
                    [b"event: other\ndata: {}\n\n"],
                    {"X-Trace-ID": "t"},
                ),
            ).events.watch("c")
        )


def test_sse_bad_json_raises_with_the_frame_path(sse):
    api = sse.api
    with pytest.raises(api.DecodeError) as caught:
        list(
            client(
                api,
                lambda request: response(
                    api,
                    200,
                    "text/event-stream",
                    [b"event: user\ndata: {nope}\n\n"],
                    {"X-Trace-ID": "t"},
                ),
            ).events.watch("c")
        )
    assert caught.value.path == "event"


def test_stream_content_type_is_checked(sse):
    api = sse.api
    with pytest.raises(api.DecodeError) as caught:
        list(
            client(
                api,
                lambda request: response(
                    api,
                    200,
                    "application/json",
                    [b"{}"],
                    {"X-Trace-ID": "t"},
                ),
            ).events.watch("c")
        )
    assert caught.value.path == "headers.content-type"


def test_stream_error_status_raises_before_iteration(sse):
    api = sse.api
    with pytest.raises(api.UnexpectedStatusError) as caught:
        list(
            client(
                api,
                lambda request: response(
                    api, 500, "application/json", [b'{"message":"broken"}']
                ),
            ).events.watch("c")
        )
    assert caught.value.status == 500


def test_stream_transport_is_lazy(sse):
    api = sse.api
    calls = []

    def transport(request):
        calls.append(request)
        return response(
            api, 200, "text/event-stream", [b'event: user\ndata: {"id":"7"}\n\n'], {"X-Trace-ID": "t"}
        )

    iterator = client(api, transport).events.watch("c")
    assert calls == []
    assert next(iterator).data.id == 7
    assert len(calls) == 1


def test_ndjson_stream_yields_typed_items(tmp_path):
    document = sse_document()
    document["operations"][1]["stream"] = {
        "kind": "ndjson",
        "item": {"ref": "User"},
    }
    generated = Generated(tmp_path, document, "ndjson_api", mode="lossy")
    try:
        api = generated.api
        items = list(
            client(
                api,
                lambda request: response(
                    api,
                    200,
                    "application/x-ndjson",
                    [b'{"id":"7"}\n{"id":"8"}\n'],
                    {"X-Trace-ID": "t"},
                ),
            ).events.watch("c")
        )
        assert [item.id for item in items] == [7, 8]
    finally:
        generated.close()


def test_bytes_stream_yields_chunks(tmp_path):
    document = sse_document()
    document["operations"][1]["stream"] = {
        "kind": "bytes",
        "content_type": "application/octet-stream",
    }
    generated = Generated(tmp_path, document, "bytes_api", mode="lossy")
    try:
        api = generated.api
        chunks = list(
            client(
                api,
                lambda request: response(
                    api,
                    200,
                    "application/octet-stream",
                    [b"one", b"two"],
                    {"X-Trace-ID": "t"},
                ),
            ).events.watch("c")
        )
        assert chunks == [b"one", b"two"]
    finally:
        generated.close()


def test_stream_error_uses_the_declared_type(tmp_path):
    document = sse_document()
    document["operations"][1]["errors"] = [
        {
            "status": 429,
            "name": ["rate", "limited"],
            "content": "json",
            "body": {"ref": "User"},
        }
    ]
    generated = Generated(tmp_path, document, "stream_error_api", mode="lossy")
    try:
        api = generated.api
        with pytest.raises(api.RateLimited) as caught:
            list(
                client(
                    api,
                    lambda request: response(
                        api, 429, "application/json", [b'{"id":"7"}']
                    ),
                ).events.watch("c")
            )
        assert caught.value.status == 429
        assert caught.value.body.id == 7
    finally:
        generated.close()


def test_stream_items_are_validated(tmp_path):
    document = sse_document()
    document["types"]["Notice"] = {
        "kind": "struct",
        "name": ["notice"],
        "direction": "output",
        "openness": "reject",
        "fields": [
            {
                "wire": "email",
                "ident": ["email"],
                "ty": {"kind": "string", "format": "email"},
                "presence": "required",
            }
        ],
    }
    document["operations"][1]["stream"]["events"].append(
        {"event": "notice", "data": {"ref": "Notice"}}
    )
    generated = Generated(
        tmp_path,
        document,
        "stream_validation_api",
        mode="lossy",
        options={"validation": {"backend": "builtin"}},
    )
    try:
        api = generated.api
        coverage = generated.response["validation_coverage"]
        assert "operations.events.watch" in coverage["exact"], coverage
        with pytest.raises(api.DecodeError) as caught:
            list(
                client(
                    api,
                    lambda request: response(
                        api,
                        200,
                        "text/event-stream",
                        [
                            b'event: notice\ndata: {"email":"nope"}\n\n'
                        ],
                        {"X-Trace-ID": "t"},
                    ),
                ).events.watch("c")
            )
        assert caught.value.path == "email"
    finally:
        generated.close()
