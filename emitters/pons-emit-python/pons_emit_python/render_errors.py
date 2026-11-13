"""Render `errors.py` for the generated package."""

from __future__ import annotations

from .analysis import Analysis
from .plan_operations import ErrorPlan
from .document import TypeRef
from .render import docstring, start


def body_hint(analysis: Analysis, ref: TypeRef | None) -> str:
    if ref is None:
        return "None"
    return analysis.wire.client_hint(ref)


def status_value(status: object) -> str:
    if isinstance(status, int):
        return str(status)
    return repr(status)


def render_errors(analysis: Analysis) -> str:
    uses_types = any(
        analysis.errors[key].body is not None and analysis.errors[key].body.id
        for key in analysis.error_order
    )
    lines = [start(f"Errors raised by the client for {analysis.document.title}.")]
    lines.append("from __future__ import annotations")
    lines.append("")
    lines.append("from typing import TYPE_CHECKING, Any")
    if uses_types:
        lines.append("")
        lines.append("if TYPE_CHECKING:")
        lines.append("    from . import types as _types")
    lines.append("")
    lines.extend(
        [
            "",
            "class PonsError(Exception):",
            '    """Base class for every error the client raises."""',
            "",
            "",
            "class TransportError(PonsError):",
            '    """The request did not reach the server or its response was lost."""',
            "",
            "",
            "class ConfigurationError(PonsError):",
            '    """The client is missing a credential an operation requires."""',
            "",
            "",
            "class DecodeError(PonsError):",
            '    """A response body did not match the contract."""',
            "",
            "    def __init__(",
            "        self,",
            "        message: str,",
            "        *,",
            "        path: str = \"\",",
            "        status: int | None = None,",
            "        cause: BaseException | None = None,",
            "    ) -> None:",
            "        self.message = message",
            "        self.path = path",
            "        self.status = status",
            "        self.cause = cause",
            "        text = \"cannot decode the response\"",
            "        if status is not None:",
            "            text += f\" {status}\"",
            "        if path:",
            "            text += f\" at {path}\"",
            "        text += f\": {message}\"",
            "        super().__init__(text)",
            "",
            "    def with_status(self, status: int) -> \"DecodeError\":",
            "        if self.status is None:",
            "            self.status = status",
            "        return self",
            "",
            "",
            "class UnexpectedStatusError(PonsError):",
            '    """The response status is not declared by the operation."""',
            "",
            "    def __init__(",
            "        self,",
            "        status: int,",
            "        headers: dict[str, str],",
            "        body: bytes,",
            "    ) -> None:",
            "        self.status = status",
            "        self.headers = dict(headers)",
            "        self.body = body",
            "        super().__init__(f\"unexpected status {status}\")",
            "",
            "",
            "class ApiError(PonsError):",
            '    """A declared error response."""',
            "",
            "    status: int | str = 0",
            "    message = \"request failed\"",
            "",
            "    def __init__(",
            "        self,",
            "        status: int,",
            "        headers: dict[str, str],",
            "        body: Any,",
            "    ) -> None:",
            "        self.status = status",
            "        self.headers = dict(headers)",
            "        self.body = body",
            "",
            "    def __str__(self) -> str:",
            "        return f\"{self.message} ({self.status})\"",
        ]
    )
    for key in analysis.error_order:
        error = analysis.errors[key]
        lines.extend(render_error(analysis, error))
    return "\n".join(lines).rstrip() + "\n"


def render_error(analysis: Analysis, error: ErrorPlan) -> list[str]:
    hint = body_hint(analysis, error.body)
    return [
        "",
        "",
        f"class {error.name}(ApiError):",
        f"    status = {status_value(error.status)}",
        f"    message = {docstring(error.message)}",
        "",
        "    def __init__(",
        "        self,",
        "        status: int,",
        "        headers: dict[str, str],",
        f"        body: {hint},",
        "    ) -> None:",
        "        super().__init__(status, headers, body)",
    ]
