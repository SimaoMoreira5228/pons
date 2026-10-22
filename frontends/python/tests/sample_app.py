from __future__ import annotations

from datetime import datetime
from enum import Enum
from uuid import UUID

from fastapi import FastAPI, Header, Query
from pydantic import BaseModel, ConfigDict

from pons_fastapi import (
    PonsApp,
    PonsAuth,
    PonsAuthScheme,
    PonsProvides,
    pons_operation,
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


@app.get("/legacy")
def legacy():
    return {"ok": True}
