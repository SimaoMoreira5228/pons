from __future__ import annotations

from datetime import datetime
from enum import Enum
from typing import Annotated
from uuid import UUID

from fastapi import Body, FastAPI, File, Form, Header, Query, UploadFile
from fastapi.responses import StreamingResponse
from pydantic import BaseModel, ConfigDict

from pons_fastapi import (
    PonsApp,
    PonsAuth,
    PonsAuthScheme,
    PonsPageSize,
    PonsPagination,
    PonsProvides,
    pons_media,
    pons_operation,
    pons_stream,
    pons_type,
)

app = FastAPI(
    title="Sample API",
    version="1.4.0",
    description="A small application used by the producer tests.",
    servers=[{"url": "https://api.example.test"}],
)


class Role(str, Enum):
    ADMIN = "admin"
    MEMBER = "member"


class LoginRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    email: str
    password: str
    remember: bool = False


class Session(BaseModel):
    model_config = ConfigDict(extra="forbid")

    token: str
    expires_at: datetime
    user_id: UUID


class LoginError(BaseModel):
    model_config = ConfigDict(extra="forbid")

    code: str
    message: str


class User(BaseModel):
    model_config = ConfigDict(extra="forbid")

    id: UUID
    name: str
    role: Role
    nickname: str | None = None


class UpdateUser(BaseModel):
    model_config = ConfigDict(extra="forbid")

    name: str
    nickname: str | None = None


class UserPage(BaseModel):
    model_config = ConfigDict(extra="forbid")

    users: list[User]
    next_cursor: str | None = None


class TokenEvent(BaseModel):
    model_config = ConfigDict(extra="forbid")

    text: str


class StreamDone(BaseModel):
    model_config = ConfigDict(extra="forbid")

    tokens: int


class UploadResult(BaseModel):
    model_config = ConfigDict(extra="forbid")

    id: UUID


class FeedbackResult(BaseModel):
    model_config = ConfigDict(extra="forbid")

    accepted: bool


class IngestResult(BaseModel):
    model_config = ConfigDict(extra="forbid")

    stored: int


class Invoice(BaseModel):
    model_config = ConfigDict(extra="forbid")

    total: Annotated[str, pons_type("decimal", wire="string")]
    reference: Annotated[str, pons_type("string", format="uuid")]


class EventStreamResponse(StreamingResponse):
    media_type = "text/event-stream"


app.state.pons = PonsApp(
    auth_schemes={"session": PonsAuthScheme(kind="bearer", format="jwt")},
)


@app.post(
    "/auth/login",
    operation_id="auth_login",
    responses={401: {"model": LoginError, "description": "Credentials were rejected."}},
)
@pons_operation(
    operation_id="auth.login",
    summary="Exchange credentials for a session token.",
    tags=("auth",),
    provides=PonsProvides(
        scheme="session",
        value="body:/token",
        expires_at="body:/expires_at",
    ),
)
def login(body: LoginRequest) -> Session:
    return Session(token="t", expires_at=datetime.now(), user_id=UUID(int=0))


@app.get("/users/{user_id}")
@pons_operation(
    operation_id="users.get",
    summary="Read one user.",
    tags=("users",),
    auth=PonsAuth(
        alternatives=(("session",),),
        scopes={"session": ("profile:read",)},
    ),
)
def get_user(
    user_id: UUID,
    limit: int = Query(10, ge=1, le=50),
    role: Role | None = Query(None),
    x_request_id: str = Header(...),
) -> User:
    return User(id=user_id, name="Ada", role=Role.ADMIN)


@app.patch("/users/{user_id}")
@pons_operation(
    operation_id="users.update",
    summary="Update one user.",
    tags=("users",),
    auth=PonsAuth(alternatives=(("session",),)),
)
def update_user(user_id: UUID, body: UpdateUser) -> UpdateUser:
    return body


@app.get("/users", operation_id="users.list")
@pons_operation(
    operation_id="users.list",
    summary="List users.",
    tags=("users",),
    pagination=PonsPagination(
        kind="cursor",
        field="cursor",
        items="/users",
        next="/next_cursor",
        page_size=PonsPageSize(field="limit", default=20, maximum=100),
    ),
)
def list_users(
    cursor: str | None = Query(None),
    limit: int = Query(20, ge=1, le=100),
) -> UserPage:
    return UserPage(users=[])


@app.get("/chat/stream", operation_id="chat.stream", response_class=EventStreamResponse)
@pons_stream(events={"token": TokenEvent, "done": StreamDone})
def stream_chat() -> EventStreamResponse:
    async def events():
        yield b"event: token\ndata: {}\n\n"

    return EventStreamResponse(events())


@app.post("/files", operation_id="files.upload")
def upload_file(file: UploadFile = File(...), label: str = Form(...)) -> UploadResult:
    return UploadResult(id=UUID(int=0))


@app.post("/feedback", operation_id="feedback.create")
def create_feedback(name: str = Form(...), rating: int = Form(1)) -> FeedbackResult:
    return FeedbackResult(accepted=True)


@app.post("/ingest", operation_id="ingest.raw")
@pons_media(request="application/octet-stream")
def ingest(body: bytes = Body(...)) -> IngestResult:
    return IngestResult(stored=len(body))


@app.get("/report.csv", operation_id="reports.csv")
@pons_media(response="text/csv")
def report() -> str:
    return "id,name\n"


@app.get("/invoice", operation_id="billing.invoice")
def invoice(amount: Annotated[str, pons_type("decimal", wire="string")]) -> Invoice:
    return Invoice(total=amount, reference="00000000-0000-0000-0000-000000000000")


@app.get("/legacy")
def legacy():
    return {"ok": True}
