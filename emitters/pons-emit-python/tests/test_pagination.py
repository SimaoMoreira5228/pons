import importlib
import json
import sys

import pytest

import support


class Generated:
    def __init__(self, directory, document, package, *, mode="strict"):
        self.response = support.emit(
            document, mode=mode, options={"package": package}
        )
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


@pytest.fixture
def cursor_client(tmp_path):
    generated = Generated(
        tmp_path, support.load_spec("paginated.json"), "paginated_api"
    )
    yield generated
    generated.close()


def offset_document():
    document = support.load_spec("paginated.json")
    document["types"]["PageQuery"]["fields"][0] = {
        "wire": "offset",
        "ident": ["offset"],
        "ty": {"kind": "int", "bits": 32, "signed": False, "wire": "number"},
        "presence": "optional",
    }
    document["operations"][0]["semantics"]["pagination"] = {
        "request": {"kind": "offset", "field": "offset"},
        "items": "/items",
        "page_size": {"field": "limit", "default": 20, "maximum": 100},
    }
    return document


@pytest.fixture
def offset_client(tmp_path):
    generated = Generated(tmp_path, offset_document(), "offset_api")
    yield generated
    generated.close()


def user(user_id):
    return {"id": user_id, "handle": f"user-{user_id}"}


def page(items, next_cursor=...):
    body = {"items": items}
    if next_cursor is not ...:
        body["next_cursor"] = next_cursor
    return body


def serve(pages, requests):
    def transport(request):
        requests.append(request)
        body = pages[len(requests) - 1]
        return (
            200,
            {"Content-Type": "application/json"},
            json.dumps(body).encode("utf-8"),
        )

    return transport


def test_cursor_iterator_yields_every_item(cursor_client):
    requests = []
    pages = [
        page([user(1), user(2)], "c2"),
        page([user(3)], None),
    ]
    instance = cursor_client.api.Client(
        "https://api.test", transport=serve(pages, requests)
    )
    items = list(instance.users.list_all())
    assert [item.id for item in items] == [1, 2, 3]
    assert len(requests) == 2
    assert "cursor" not in requests[0].url
    assert requests[1].url.endswith("?cursor=c2")


def test_cursor_iterator_uses_the_caller_cursor(cursor_client):
    api = cursor_client.api
    requests = []
    pages = [page([user(1)], "c2"), page([], None)]
    instance = api.Client(
        "https://api.test", transport=serve(pages, requests)
    )
    query = api.PageQuery(cursor="start", limit=5)
    list(instance.users.list_all(query=query))
    assert requests[0].url.endswith("?cursor=start&limit=5")
    assert requests[1].url.endswith("?cursor=c2&limit=5")


def test_cursor_iterator_stops_on_an_empty_cursor(cursor_client):
    requests = []
    pages = [page([user(1)], "")]
    list(
        cursor_client.api.Client(
            "https://api.test", transport=serve(pages, requests)
        ).users.list_all()
    )
    assert len(requests) == 1


def test_cursor_iterator_stops_on_a_missing_cursor(cursor_client):
    requests = []
    pages = [page([user(1)])]
    list(
        cursor_client.api.Client(
            "https://api.test", transport=serve(pages, requests)
        ).users.list_all()
    )
    assert len(requests) == 1


def test_cursor_iterator_stops_on_a_null_cursor(cursor_client):
    requests = []
    pages = [page([user(1)], None)]
    list(
        cursor_client.api.Client(
            "https://api.test", transport=serve(pages, requests)
        ).users.list_all()
    )
    assert len(requests) == 1


def test_offset_iterator_advances_and_keeps_going_after_a_short_page(
    offset_client,
):
    requests = []
    pages = [
        page([user(1), user(2)]),
        page([user(3)]),
        page([]),
    ]
    items = list(
        offset_client.api.Client(
            "https://api.test", transport=serve(pages, requests)
        ).users.list_all()
    )
    assert [item.id for item in items] == [1, 2, 3]
    assert len(requests) == 3
    assert requests[0].url.endswith("?offset=0&limit=20")
    assert requests[1].url.endswith("?offset=2")
    assert requests[2].url.endswith("?offset=3")


def test_offset_iterator_starts_at_the_caller_offset(offset_client):
    api = offset_client.api
    requests = []
    pages = [page([user(1), user(2)]), page([])]
    query = api.PageQuery(offset=4)
    list(
        api.Client(
            "https://api.test", transport=serve(pages, requests)
        ).users.list_all(query=query)
    )
    assert requests[0].url.endswith("?offset=4&limit=20")
    assert requests[1].url.endswith("?offset=6")


def test_page_size_default_is_sent_only_on_the_first_request(cursor_client):
    requests = []
    pages = [page([user(1)], "c2"), page([], None)]
    list(
        cursor_client.api.Client(
            "https://api.test", transport=serve(pages, requests)
        ).users.list_all()
    )
    assert "limit=20" in requests[0].url
    assert "limit=" not in requests[1].url
    assert "maximum" not in requests[0].url


def test_caller_page_size_stays_on_every_request(cursor_client):
    api = cursor_client.api
    requests = []
    pages = [page([user(1)], "c2"), page([], None)]
    query = api.PageQuery(limit=5)
    list(
        api.Client(
            "https://api.test", transport=serve(pages, requests)
        ).users.list_all(query=query)
    )
    assert "limit=5" in requests[0].url
    assert "limit=5" in requests[1].url


def test_page_size_default_does_not_mutate_the_caller_query(cursor_client):
    api = cursor_client.api
    requests = []
    pages = [page([user(1)], None)]
    query = api.PageQuery()
    list(
        api.Client(
            "https://api.test", transport=serve(pages, requests)
        ).users.list_all(query=query)
    )
    assert query.limit is None
    assert query.cursor is None


def test_page_error_raises_and_ends_the_iterator(tmp_path):
    document = support.load_spec("paginated.json")
    document["types"]["Failure"] = {
        "kind": "struct",
        "name": ["failure"],
        "direction": "output",
        "openness": "reject",
        "fields": [
            {
                "wire": "message",
                "ident": ["message"],
                "ty": {"kind": "string"},
                "presence": "required",
            }
        ],
    }
    document["operations"][0]["errors"] = [
        {
            "status": 429,
            "name": ["rate", "limited"],
            "content": "json",
            "body": {"ref": "Failure"},
        }
    ]
    generated = Generated(tmp_path, document, "failure_api")
    try:
        api = generated.api
        requests = []
        pages = [page([user(1)], "c2")]

        def transport(request):
            requests.append(request)
            if len(requests) == 1:
                return (
                    200,
                    {"Content-Type": "application/json"},
                    json.dumps(pages[0]).encode("utf-8"),
                )
            return (
                429,
                {"Content-Type": "application/json"},
                json.dumps({"message": "slow down"}).encode("utf-8"),
            )

        iterator = api.Client(
            "https://api.test", transport=transport
        ).users.list_all()
        assert next(iterator).id == 1
        with pytest.raises(api.RateLimited):
            next(iterator)
        with pytest.raises(StopIteration):
            next(iterator)
        assert len(requests) == 2
    finally:
        generated.close()


def test_a_page_response_with_headers_is_unwrapped(tmp_path):
    document = support.load_spec("paginated.json")
    document["operations"][0]["responses"][0]["headers"] = [
        {
            "wire": "X-Trace-ID",
            "ident": ["trace"],
            "ty": {"kind": "string"},
            "presence": "required",
        }
    ]
    generated = Generated(tmp_path, document, "header_api")
    try:
        api = generated.api
        requests = []

        def transport(request):
            requests.append(request)
            body = page([user(1)], None)
            return (
                200,
                {"Content-Type": "application/json", "X-Trace-ID": "trace-1"},
                json.dumps(body).encode("utf-8"),
            )

        instance = api.Client("https://api.test", transport=transport)
        assert isinstance(instance.users.list(), api.Response)
        assert [item.id for item in instance.users.list_all()] == [1]
    finally:
        generated.close()


def refused(document, marker):
    response = support.emit(document)
    assert response["files"] == []
    assert response["error"]
    assert all(
        diagnostic["code"].startswith("PY-E")
        for diagnostic in response["diagnostics"]
    )
    messages = " ".join(
        diagnostic["message"] for diagnostic in response["diagnostics"]
    )
    assert marker in messages, messages
    return response


def test_a_pagination_field_on_the_body_is_refused():
    document = support.load_spec("paginated.json")
    document["types"]["BodyPage"] = {
        "kind": "struct",
        "name": ["body", "page"],
        "direction": "input",
        "openness": "reject",
        "fields": [
            {
                "wire": "cursor",
                "ident": ["cursor"],
                "ty": {"kind": "string"},
                "presence": "optional",
            }
        ],
    }
    document["operations"][0]["body"] = {
        "ty": {"ref": "BodyPage"},
        "content": "json",
        "required": True,
    }
    refused(document, "request body")


def test_an_items_pointer_through_a_nullable_wrapper_is_refused():
    document = support.load_spec("paginated.json")
    document["types"]["Page"]["fields"][0]["ty"] = {
        "kind": "nullable",
        "inner": {"kind": "array", "item": {"ref": "User"}},
    }
    response = refused(document, "nullable wrapper")
    messages = " ".join(
        diagnostic["message"] for diagnostic in response["diagnostics"]
    )
    assert "/items" in messages


def test_an_items_pointer_through_a_newtype_is_refused():
    document = support.load_spec("paginated.json")
    document["types"]["PageItems"] = {
        "kind": "newtype",
        "name": ["page", "items"],
        "direction": "output",
        "inner": {"kind": "array", "item": {"ref": "User"}},
    }
    document["types"]["Page"]["fields"][0]["ty"] = {"ref": "PageItems"}
    response = refused(document, "newtype")
    messages = " ".join(
        diagnostic["message"] for diagnostic in response["diagnostics"]
    )
    assert "/items" in messages


def test_a_next_pointer_that_is_not_a_string_is_refused():
    document = support.load_spec("paginated.json")
    document["types"]["Page"]["fields"][1]["ty"] = {
        "kind": "int",
        "bits": 32,
        "signed": False,
        "wire": "number",
    }
    response = refused(document, "not a string")
    messages = " ".join(
        diagnostic["message"] for diagnostic in response["diagnostics"]
    )
    assert "/next_cursor" in messages


def test_a_page_size_field_that_does_not_exist_is_refused():
    document = support.load_spec("paginated.json")
    document["operations"][0]["semantics"]["pagination"]["page_size"] = {
        "field": "page_size"
    }
    refused(document, "page size field")


def test_a_page_size_field_on_the_body_is_refused():
    document = support.load_spec("paginated.json")
    document["types"]["BodyPage"] = {
        "kind": "struct",
        "name": ["body", "page"],
        "direction": "input",
        "openness": "reject",
        "fields": [
            {
                "wire": "limit",
                "ident": ["limit"],
                "ty": {"kind": "int", "bits": 32, "signed": False, "wire": "number"},
                "presence": "optional",
            }
        ],
    }
    document["operations"][0]["body"] = {
        "ty": {"ref": "BodyPage"},
        "content": "json",
        "required": True,
    }
    refused(document, "page size field")
