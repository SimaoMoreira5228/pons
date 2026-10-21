from __future__ import annotations

from typing import Any


def diagnostic(
    code: str,
    message: str,
    severity: str = "warning",
    origin: dict[str, Any] | None = None,
) -> dict[str, Any]:
    value: dict[str, Any] = {
        "severity": severity,
        "code": code,
        "message": message,
    }
    if origin is not None:
        value["origin"] = origin
    return value


def unsupported(code: str, message: str) -> dict[str, Any]:
    return diagnostic(code, message, severity="info")
