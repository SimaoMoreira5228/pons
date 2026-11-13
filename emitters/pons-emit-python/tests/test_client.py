import base64
import importlib
import json
import sys
from decimal import Decimal

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
        self.types = importlib.import_module(package + ".types")
        self.errors = importlib.import_module(package + ".errors")
        self.wire = importlib.import_module(package + "._wire")

    def close(self):
        sys.path.remove(str(self.directory))
        for name in list(sys.modules):
            if name == self.package or name.startswith(self.package + "."):
                del sys.modules[name]
        importlib.invalidate_caches()


@pytest.fixture
def client(tmp_path):
    generated = Generated(tmp_path, support.load("client.json"), "client_api")
    yield generated
    generated.close()


@pytest.fixture
def exact(tmp_path):
    generated = Generated(
        tmp_path, support.load_spec("exact.json"), "exact_api", mode="lossy"
    )
    yield generated
    generated.close()


def json_bytes(value):
    return json.dumps(value).encode("utf-8")


def session_json(token="tok", expires="later"):
    return json_bytes({"token": token, "expires_at": expires})


def user_payload(**overrides):
    value = {
        "id": "7",
        "big": 340282366920938463463374607431768211455,
        "negative": -5,
        "nickname": "nick",
        "email": None,
        "status": "active",
        "handle": "simao",
        "blob": base64.b64encode(b"abc").decode("ascii"),
        "url_blob": "YWI",
        "hex_blob": "00ff",
        "array_blob": [1, 2],
        "tags": ["a", "b"],
        "labels": {"env": "test"},
        "pair": ["s", 3],
        "ratio": 1.5,
        "amount": "12.50",
        "extra": {"kept": True},
    }
    value.update(overrides)
    return value


def profile_payload():
    return {
        "choice": {"count": 3},
        "marked": {"kind": "payload", "note": "n", "flag": True},
        "beside": {"kind": "pair", "detail": [1, "x"]},
        "alias": None,
    }


def test_login_encodes_path_query_header_and_body(client):
    api = client.api
    requests = []

    def transport(request):
        requests.append(request)
        if request.url.startswith("https://api.test/auth/"):
            return (
                200,
                {"Content-Type": "application/json", "X-Trace-ID": "trace-1"},
                session_json(),
            )
        return (
            200,
            {"Content-Type": "application/vnd.example+json"},
            json_bytes(user_payload()),
        )

    client_instance = api.Client(
        "https://api.test", transport=transport, timeout=3
    )
    response = client_instance.auth.login(
        "tenant one",
        query=api.LoginQuery(limit=5, active=True),
        x_request_id="req-1",
        body=api.LoginRequest(email="a@example.test", password="secret"),
    )
    assert isinstance(response, api.Response)
    assert response.status == 200
    assert response.headers["X-Trace-ID"] == "trace-1"
    assert isinstance(response.body, api.Session)
    assert response.body.token == "tok"

    login = requests[0]
    assert login.method == "POST"
    assert login.url == (
        "https://api.test/auth/tenant%20one/login"
        "?limit=5&active=true"
    )
    assert login.headers["Accept"] == "application/json"
    assert login.headers["Content-Type"] == "application/vnd.example+json"
    assert login.headers["X-Request-ID"] == "req-1"
    assert json.loads(login.body) == {"email": "a@example.test", "password": "secret"}
    assert login.timeout == 3

    user = client_instance.users.get("42")
    assert requests[1].headers["Authorization"] == "Bearer tok"
    assert user.id == 7
    assert user.big == 340282366920938463463374607431768211455
    assert user.negative == -5
    assert user.nickname == "nick"
    assert user.email is None
    assert isinstance(user.status, api.StatusActive)
    assert user.status.to_wire() == "active"
    assert isinstance(user.handle, api.UserHandle)
    assert user.handle.value == "simao"
    assert user.blob == b"abc"
    assert user.url_blob == b"ab"
    assert user.hex_blob == b"\x00\xff"
    assert user.array_blob == b"\x01\x02"
    assert user.tags == ["a", "b"]
    assert user.labels == {"env": "test"}
    assert user.pair == ("s", 3)
    assert user.ratio == 1.5
    assert user.amount == Decimal("12.50")
    assert user.extra == {"kept": True}
    wire = user.to_wire()
    assert wire["status"] == "active"
    assert wire["email"] is None
    assert wire["url_blob"] == "YWI"
    assert "=" not in wire["url_blob"]
    assert wire["amount"] == "12.50"


def test_optional_non_nullable_field_uses_none(client):
    api = client.api

    def transport(request):
        payload = user_payload()
        payload.pop("nickname")
        return 200, {"Content-Type": "application/vnd.example+json"}, json_bytes(payload)

    user = api.Client(
        "https://api.test", transport=transport, credentials={"session": "tok"}
    ).users.get("42")
    assert user.nickname is None
    assert "nickname" not in user.to_wire()


def test_optional_nullable_field_uses_missing(client):
    api = client.api
    assert api.MISSING is client.wire.MISSING

    def transport_without_alias(request):
        payload = profile_payload()
        payload.pop("alias")
        return 200, {"Content-Type": "application/json"}, json_bytes(payload)

    profile = api.Client(
        "https://api.test", transport=transport_without_alias
    ).profiles.get()
    assert profile.alias is client.wire.MISSING
    assert "alias" not in profile.to_wire()

    def transport_with_null(request):
        return 200, {"Content-Type": "application/json"}, json_bytes(profile_payload())

    profile = api.Client(
        "https://api.test", transport=transport_with_null
    ).profiles.get()
    assert profile.alias is None
    assert profile.to_wire()["alias"] is None


def test_string_wire_integer_encodes_to_a_string(client):
    user = client.types.User.from_wire(user_payload())
    assert user.id == 7
    assert user.to_wire()["id"] == "7"


def test_enum_representations_round_trip(client):
    types = client.types

    choices = [
        ("none", types.ChoiceNone),
        ({"count": 3}, types.ChoiceCount),
        ({"pair": ["a", True]}, types.ChoicePair),
        ({"record": {"note": "n"}}, types.ChoiceRecord),
    ]
    for value, kind in choices:
        decoded = types.decode_Choice(value)
        assert isinstance(decoded, kind)
        assert decoded.to_wire() == value

    marked = [
        ({"kind": "empty"}, types.MarkedEmpty),
        ({"kind": "meta", "label": "x"}, types.MarkedMeta),
        ({"kind": "payload", "note": "n"}, types.MarkedPayload),
    ]
    for value, kind in marked:
        decoded = types.decode_Marked(value)
        assert isinstance(decoded, kind)
        assert decoded.to_wire() == value

    beside = [
        ({"kind": "off"}, types.BesideOff),
        ({"kind": "text", "detail": "x"}, types.BesideText),
        ({"kind": "pair", "detail": [1, "x"]}, types.BesidePair),
        ({"kind": "record", "detail": {"note": "x"}}, types.BesideRecord),
    ]
    for value, kind in beside:
        decoded = types.decode_Beside(value)
        assert isinstance(decoded, kind)
        assert decoded.to_wire() == value

    with pytest.raises(client.errors.DecodeError) as caught:
        types.decode_Choice({"ghost": 1})
    assert "ghost" in str(caught.value)

    with pytest.raises(client.errors.DecodeError):
        types.BesideText.from_wire({"kind": "off"})
    with pytest.raises(client.errors.DecodeError):
        types.MarkedMeta.from_wire({"kind": "empty"})


def test_enum_body_is_encoded_and_decoded(client):
    api = client.api
    captured = []

    def transport(request):
        captured.append(request)
        return (
            201,
            {"Content-Type": "application/vnd.example+json"},
            json_bytes(profile_payload()),
        )

    profile = api.Profile(
        choice=api.ChoiceCount(3),
        marked=api.MarkedPayload(api.Payload(note="n", flag=True)),
        beside=api.BesidePair((1, "x")),
        alias=None,
    )
    result = api.Client("https://api.test", transport=transport).profiles.echo(
        body=profile
    )
    request = captured[0]
    assert request.method == "POST"
    assert request.url == "https://api.test/profiles"
    assert request.headers["Accept"] == "application/vnd.example+json"
    assert request.headers["Content-Type"] == "application/vnd.example+json"
    assert json.loads(request.body) == {
        "choice": {"count": 3},
        "marked": {"note": "n", "flag": True, "kind": "payload"},
        "beside": {"kind": "pair", "detail": [1, "x"]},
        "alias": None,
    }
    assert isinstance(result, api.Profile)
    assert isinstance(result.marked, api.MarkedPayload)
    assert isinstance(result.marked.value, api.Payload)
    assert result.marked.value.flag is True
    assert result.beside.value == (1, "x")


def test_variant_narrowed_error_decodes_only_that_variant(client):
    api = client.api

    def transport(request):
        return 400, {"Content-Type": "application/json"}, json_bytes(
            {"code": "locked", "retry_after": 9}
        )

    instance = api.Client("https://api.test", transport=transport)
    with pytest.raises(api.ProfileFailure) as caught:
        instance.profiles.get()
    error = caught.value
    assert error.status == 400
    assert isinstance(error.body, api.FailureLocked)
    assert error.body.retry_after == 9
    assert error.message == "profile failure"

    def other_transport(request):
        return 400, {"Content-Type": "application/json"}, json_bytes(
            {"code": "invalid"}
        )

    with pytest.raises(api.DecodeError):
        api.Client("https://api.test", transport=other_transport).profiles.get()


def test_auth_schemes_are_attached(client):
    api = client.api
    seen = []

    def transport(request):
        seen.append(request)
        return 200, {"Content-Type": "application/json"}, session_json()

    credentials = {
        "tenant": "tenant",
        "query_key": "q",
        "cookie_key": "c",
        "basic": "u:p",
        "oauth": "o",
        "cookie_session": "s",
        "session": "tok",
    }
    instance = api.Client(
        "https://api.test", transport=transport, credentials=credentials
    )
    instance.keys.header()
    assert seen[-1].headers["X-Tenant"] == "tenant"
    instance.keys.query()
    assert seen[-1].url == "https://api.test/keys/query?api_key=q"
    instance.keys.cookie()
    assert seen[-1].headers["Cookie"] == "sid=c"
    instance.auth.basic()
    assert seen[-1].headers["Authorization"] == (
        "Basic " + base64.b64encode(b"u:p").decode("ascii")
    )
    instance.auth.oauth()
    assert seen[-1].headers["Authorization"] == "Bearer o"
    instance.auth.cookie()
    assert seen[-1].headers["Cookie"] == "session=s"
    instance.users.me()
    assert seen[-1].headers["X-Tenant"] == "tenant"

    anonymous = api.Client("https://api.test", transport=transport)
    anonymous.public.optional()
    assert "Authorization" not in seen[-1].headers
    anonymous = api.Client(
        "https://api.test", transport=transport, credentials={"session": "tok"}
    )
    anonymous.public.optional()
    assert seen[-1].headers["Authorization"] == "Bearer tok"

    before = len(seen)
    with pytest.raises(api.ConfigurationError):
        api.Client("https://api.test", transport=transport).users.get("42")
    assert len(seen) == before


def test_provides_from_header_and_set_cookie(client):
    api = client.api
    requests = []

    def transport(request):
        requests.append(request)
        if request.url.endswith("/tokens/header"):
            return (
                200,
                {"Content-Type": "application/json", "X-Token": "header-token"},
                session_json(),
            )
        if request.url.endswith("/tokens/cookie"):
            return (
                200,
                {
                    "Content-Type": "application/json",
                    "Set-Cookie": "cookie_key=cookie-value; Path=/; HttpOnly",
                },
                session_json(),
            )
        if request.url.endswith("/42"):
            return (
                200,
                {"Content-Type": "application/vnd.example+json"},
                json_bytes(user_payload()),
            )
        return 200, {"Content-Type": "application/json"}, session_json()

    instance = api.Client("https://api.test", transport=transport)
    instance.tokens.header()
    instance.users.get("42")
    assert requests[-1].headers["Authorization"] == "Bearer header-token"

    instance.tokens.cookie()
    instance.keys.cookie()
    assert requests[-1].headers["Cookie"] == "sid=cookie-value"


def test_set_cookie_reads_repeated_headers(client):
    api = client.api
    requests = []

    def transport(request):
        requests.append(request)
        if request.url.endswith("/tokens/cookie"):
            return (
                200,
                {
                    "Content-Type": "application/json",
                    "Set-Cookie": [
                        "other=first; Path=/",
                        "cookie_key=repeated; Path=/; HttpOnly",
                    ],
                },
                session_json(),
            )
        return 200, {"Content-Type": "application/json"}, session_json()

    instance = api.Client("https://api.test", transport=transport)
    instance.tokens.cookie()
    instance.keys.cookie()
    assert requests[-1].headers["Cookie"] == "sid=repeated"


def test_typed_errors(client):
    api = client.api

    def respond(status, body, errors_inherit=True):
        def transport(request):
            return status, {"Content-Type": "application/json"}, json_bytes(body)

        return transport

    with pytest.raises(api.NotFoundError) as caught:
        api.Client(
            "https://api.test",
            transport=respond(404, {"message": "gone"}),
            credentials={"session": "tok"},
        ).users.get("42")
    assert caught.value.status == 404
    assert caught.value.body.message == "gone"

    with pytest.raises(api.InvalidCredentials) as caught:
        api.Client(
            "https://api.test", transport=respond(401, {"message": "no"})
        ).auth.login("t", body=api.LoginRequest(email="e", password="p"))
    assert caught.value.body.message == "no"

    with pytest.raises(api.ClientError) as caught:
        api.Client(
            "https://api.test", transport=respond(418, {"message": "teapot"})
        ).errors.class_()
    assert caught.value.status == 418

    with pytest.raises(api.ServerError) as caught:
        api.Client(
            "https://api.test", transport=respond(503, {"message": "down"})
        ).errors.default()
    assert caught.value.status == 503

    with pytest.raises(api.RateLimited) as caught:
        api.Client(
            "https://api.test",
            transport=respond(429, {"message": "slow"}),
            credentials={"basic": "u:p"},
        ).auth.basic()
    assert caught.value.status == 429

    with pytest.raises(api.UnexpectedStatusError) as caught:
        api.Client(
            "https://api.test",
            transport=respond(418, {"message": "no"}),
            credentials={"session": "tok"},
        ).users.me()
    assert caught.value.status == 418


def test_decode_errors_carry_paths(client):
    api = client.api

    def transport(body, content_type="application/vnd.example+json", status=200):
        def send(request):
            return status, {"Content-Type": content_type}, body

        return send

    missing = user_payload()
    missing.pop("id")
    with pytest.raises(api.DecodeError) as caught:
        api.Client(
            "https://api.test",
            transport=transport(json_bytes(missing)),
            credentials={"session": "tok"},
        ).users.get("42")
    assert caught.value.path == "id"

    bad_integer = user_payload(id="07")
    with pytest.raises(api.DecodeError) as caught:
        api.Client(
            "https://api.test",
            transport=transport(json_bytes(bad_integer)),
            credentials={"session": "tok"},
        ).users.get("42")
    assert caught.value.path == "id"

    too_wide = user_payload(big=1 << 128)
    with pytest.raises(api.DecodeError) as caught:
        api.Client(
            "https://api.test",
            transport=transport(json_bytes(too_wide)),
            credentials={"session": "tok"},
        ).users.get("42")
    assert caught.value.path == "big"

    bad_bytes = user_payload(blob="!!!")
    with pytest.raises(api.DecodeError) as caught:
        api.Client(
            "https://api.test",
            transport=transport(json_bytes(bad_bytes)),
            credentials={"session": "tok"},
        ).users.get("42")
    assert caught.value.path == "blob"

    with pytest.raises(api.DecodeError) as caught:
        api.Client(
            "https://api.test", transport=transport(b"not json")
        ).users.decode()
    assert caught.value.status == 200

    with pytest.raises(api.DecodeError) as caught:
        api.Client(
            "https://api.test",
            transport=transport(json_bytes(user_payload()), content_type="text/plain"),
            credentials={"session": "tok"},
        ).users.get("42")
    assert caught.value.path == "headers.content-type"

    with pytest.raises(api.UnexpectedStatusError):
        api.Client(
            "https://api.test", transport=transport(b"", status=500)
        ).users.decode()


def test_transport_failures_are_wrapped(client):
    api = client.api

    def broken(request):
        raise RuntimeError("connection reset")

    with pytest.raises(api.TransportError):
        api.Client("https://api.test", transport=broken).public.open()


def test_exact_fixture_optional_nullable_and_internal_enum(exact):
    api = exact.api

    def user(body):
        def transport(request):
            return (
                200,
                {"Content-Type": "application/json", "X-Trace-ID": "trace-1"},
                json_bytes(body),
            )

        return transport

    absent = api.Client(
        "https://api.test",
        transport=user({"id": "7"}),
        credentials={"tenant": "t"},
    ).events.watch("cursor")
    assert isinstance(absent, api.Response)
    assert absent.headers["X-Trace-ID"] == "trace-1"
    assert absent.body.nickname is exact.wire.MISSING

    explicit_null = api.Client(
        "https://api.test",
        transport=user({"id": "7", "nickname": None}),
        credentials={"tenant": "t"},
    ).events.watch("cursor")
    assert explicit_null.body.nickname is None

    def error(body):
        def transport(request):
            return 400, {"Content-Type": "application/json"}, json_bytes(body)

        return transport

    with pytest.raises(api.LoginErrorException) as caught:
        api.Client(
            "https://api.test", transport=error({"code": "invalid"})
        ).auth.login(body=api.LoginRequest(email="e"))
    assert isinstance(caught.value.body, api.LoginErrorInvalid)

    with pytest.raises(api.DecodeError):
        api.Client(
            "https://api.test",
            transport=error({"code": "locked", "retry_after": 1}),
        ).auth.login(body=api.LoginRequest(email="e"))
