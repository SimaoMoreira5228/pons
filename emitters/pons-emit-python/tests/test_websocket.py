import base64
import hashlib
import importlib
import json
import socket
import sys
import threading

import pytest

import support


class Generated:
    def __init__(self, directory, document, package, *, mode="lossy"):
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
def socket_api(tmp_path):
    generated = Generated(tmp_path, support.load_spec("unsupported.json"), "socket_api")
    yield generated
    generated.close()


class FakeConnection:
    def __init__(self, incoming=None, binary_incoming=None):
        self.sent = []
        self.sent_binary = []
        self.incoming = list(incoming or [])
        self.binary_incoming = list(binary_incoming or [])
        self.closed = False

    def send_text(self, text):
        self.sent.append(text)

    def send_binary(self, payload):
        self.sent_binary.append(payload)

    def recv_message(self):
        if self.incoming:
            return False, self.incoming.pop(0).encode("utf-8")
        if self.binary_incoming:
            return True, self.binary_incoming.pop(0)
        return None

    def close(self):
        self.closed = True


def tenant():
    return {"tenant": "t"}


def test_typed_websocket_sends_and_receives(socket_api):
    api = socket_api.api
    connection = FakeConnection(['{"id":"7"}'])
    requests = []

    def transport(request):
        requests.append(request)
        return connection

    instance = api.Client(
        "https://api.test",
        credentials=tenant(),
        websocket_transport=transport,
    )
    websocket = instance.events.watch("c")
    assert isinstance(websocket, api.TypedWebSocket)
    assert requests[0].url == "wss://api.test/events/c"
    assert requests[0].headers["X-Tenant"] == "t"

    websocket.send(api.LoginRequest(email="ada@example.test"))
    assert json.loads(connection.sent[0]) == {"email": "ada@example.test"}

    user = websocket.recv()
    assert user.id == 7

    with pytest.raises(StopIteration):
        next(websocket)
    websocket.close()
    assert connection.closed


def test_websocket_handshake_and_frames(socket_api):
    api = socket_api.api
    listener = socket.socket()
    listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    listener.bind(("127.0.0.1", 0))
    listener.listen(1)
    listener.settimeout(5)
    port = listener.getsockname()[1]
    seen = {}

    def serve():
        connection, _ = listener.accept()
        try:
            request = b""
            while b"\r\n\r\n" not in request:
                request += connection.recv(4096)
            headers = {}
            for line in request.split(b"\r\n")[1:]:
                if not line:
                    continue
                name, _, value = line.decode("latin1").partition(":")
                headers[name.strip().lower()] = value.strip()
            key = headers["sec-websocket-key"]
            accept = base64.b64encode(
                hashlib.sha1(
                    (key + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11").encode()
                ).digest()
            ).decode()
            connection.sendall(
                (
                    "HTTP/1.1 101 Switching Protocols\r\n"
                    "Upgrade: websocket\r\n"
                    "Connection: Upgrade\r\n"
                    f"Sec-WebSocket-Accept: {accept}\r\n\r\n"
                ).encode()
            )
            first, second = recv(connection, 2)
            length = second & 0x7F
            mask = recv(connection, 4)
            payload = bytes(
                byte ^ mask[index % 4]
                for index, byte in enumerate(recv(connection, length))
            )
            seen["sent"] = json.loads(payload)
            body = json.dumps({"id": "7"}).encode()
            connection.sendall(bytes([0x81, len(body)]) + body)
            recv(connection, 2)
        finally:
            connection.close()
            listener.close()

    thread = threading.Thread(target=serve)
    thread.start()
    try:
        instance = api.Client(
            f"http://127.0.0.1:{port}",
            credentials=tenant(),
        )
        websocket = instance.events.watch("c")
        websocket.send(api.LoginRequest(email="real@example.test"))
        assert websocket.recv().id == 7
        assert seen["sent"] == {"email": "real@example.test"}
        websocket.close()
    finally:
        thread.join(5)
        assert not thread.is_alive()


def recv(connection, count):
    chunks = []
    remaining = count
    while remaining:
        chunk = connection.recv(remaining)
        if not chunk:
            raise AssertionError("socket closed early")
        chunks.append(chunk)
        remaining -= len(chunk)
    return b"".join(chunks)


def test_websocket_binary_messages(tmp_path):
    document = support.load_spec("unsupported.json")
    document["operations"][1]["stream"]["client"] = {"kind": "bytes", "encoding": "array"}
    document["operations"][1]["stream"]["server"] = {"kind": "bytes", "encoding": "array"}
    generated = Generated(tmp_path, document, "binary_api")
    try:
        api = generated.api
        connection = FakeConnection(binary_incoming=[b"\x00\x01"])
        instance = api.Client(
            "https://api.test",
            credentials=tenant(),
            websocket_transport=lambda request: connection,
        )
        websocket = instance.events.watch("c")
        websocket.send(b"abc")
        assert connection.sent_binary == [b"abc"]
        assert websocket.recv() == b"\x00\x01"
    finally:
        generated.close()


def test_socket_connection_frames_binary(socket_api):
    client_module = importlib.import_module(socket_api.package + ".client")
    left, right = socket.socketpair()
    connection = client_module._SocketConnection(left)
    received = {}

    def peer():
        try:
            first, second = recv(right, 2)
            length = second & 0x7F
            mask = recv(right, 4)
            payload = bytes(
                byte ^ mask[index % 4]
                for index, byte in enumerate(recv(right, length))
            )
            received["opcode"] = first & 0x0F
            received["payload"] = payload
            body = b"\x00\x01"
            right.sendall(bytes([0x82, len(body)]) + body)
        finally:
            right.close()

    thread = threading.Thread(target=peer)
    thread.start()
    try:
        connection.send_binary(b"abc")
        assert connection.recv_message() == (True, b"\x00\x01")
        connection.close()
    finally:
        thread.join(5)
    assert received == {"opcode": 2, "payload": b"abc"}
