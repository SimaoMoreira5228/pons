import importlib
import json
import sys

import pytest

import support


class Generated:
    def __init__(self, directory, document, package):
        self.response = support.emit(
            document, options={"package": package}
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
def custom(tmp_path):
    generated = Generated(tmp_path, support.load("client.json"), "custom_api")
    yield generated
    generated.close()


def session_json():
    return json.dumps({"token": "tok", "expires_at": "later"}).encode("utf-8")


def transport_for(seen):
    def transport(request):
        seen.append(request)
        return 200, {"Content-Type": "application/json"}, session_json()

    return transport


def test_adapter_attaches_headers_query_and_cookies(custom):
    api = custom.api
    seen = []

    def adapter(context):
        return api.AuthAttachment(
            headers={"X-Signature": "sig"},
            query=(("signature", "abc"),),
            cookies={"custom": "1"},
        )

    instance = api.Client(
        "https://api.test",
        transport=transport_for(seen),
        credentials={"handshake": "secret"},
        headers={"Cookie": "existing=1"},
        auth_adapters={"handshake": adapter},
    )
    instance.custom.required()
    request = seen[-1]
    assert request.headers["X-Signature"] == "sig"
    assert request.url == "https://api.test/custom/required?signature=abc"
    assert request.headers["Cookie"] == "existing=1; custom=1"


def test_adapter_sees_context(custom):
    api = custom.api
    contexts = []

    def adapter(context):
        contexts.append(context)
        return api.AuthAttachment()

    instance = api.Client(
        "https://api.test",
        transport=transport_for([]),
        credentials={"handshake": "secret"},
        headers={"X-Base": "base"},
        auth_adapters={"handshake": adapter},
    )
    instance.custom.required()
    context = contexts[-1]
    assert context.scheme == "handshake"
    assert context.credential == "secret"
    assert context.method == "GET"
    assert context.url == "https://api.test/custom/required"
    assert context.headers["Accept"] == "application/json"
    assert context.headers["X-Base"] == "base"


def test_adapter_headers_do_not_replace_protected_headers(custom):
    api = custom.api
    seen = []

    def adapter(context):
        return api.AuthAttachment(
            headers={
                "Authorization": "Custom signature",
                "Cookie": "adapter=1",
                "Content-Type": "text/plain",
                "X-Signature": "sig",
            },
            cookies={"custom": "1"},
        )

    instance = api.Client(
        "https://api.test",
        transport=transport_for(seen),
        credentials={"handshake": "secret"},
        headers={
            "Authorization": "Bearer base",
            "Cookie": "existing=1",
            "Content-Type": "application/custom",
        },
        auth_adapters={"handshake": adapter},
    )
    instance.custom.required()
    request = seen[-1]
    assert request.headers["Authorization"] == "Bearer base"
    assert request.headers["Content-Type"] == "application/custom"
    assert request.headers["Cookie"] == "existing=1; custom=1"
    assert request.headers["X-Signature"] == "sig"


def test_required_custom_auth_without_adapter_id_raises_before_transport(custom):
    api = custom.api
    seen = []
    instance = api.Client(
        "https://api.test",
        transport=transport_for(seen),
        credentials={"unbound": "secret"},
    )
    with pytest.raises(api.ConfigurationError, match="adapter id"):
        instance.custom.unbound()
    assert seen == []


def test_required_custom_auth_with_unconfigured_adapter_raises_before_transport(custom):
    api = custom.api
    seen = []
    instance = api.Client(
        "https://api.test",
        transport=transport_for(seen),
        credentials={"handshake": "secret"},
    )
    with pytest.raises(api.ConfigurationError, match="not configured"):
        instance.custom.required()
    assert seen == []


def test_optional_custom_auth_without_adapter_omits_attachment(custom):
    api = custom.api
    seen = []
    instance = api.Client("https://api.test", transport=transport_for(seen))
    instance.custom.optional()
    request = seen[-1]
    assert request.url == "https://api.test/custom/optional"
    assert "Authorization" not in request.headers
    assert "Cookie" not in request.headers


def test_provided_custom_credential_reaches_the_adapter(custom):
    api = custom.api
    contexts = []

    def adapter(context):
        contexts.append(context)
        return api.AuthAttachment()

    def transport(request):
        if request.url.endswith("/tokens/custom"):
            return (
                200,
                {
                    "Content-Type": "application/json",
                    "X-Custom-Token": "provided-token",
                },
                session_json(),
            )
        return 200, {"Content-Type": "application/json"}, session_json()

    instance = api.Client(
        "https://api.test",
        transport=transport,
        auth_adapters={"handshake": adapter},
    )
    instance.tokens.custom()
    instance.custom.required()
    assert contexts[-1].credential == "provided-token"
