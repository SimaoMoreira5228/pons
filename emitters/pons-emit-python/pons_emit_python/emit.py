"""Emit response assembly and the `__init__.py` renderer."""

from __future__ import annotations

from typing import Any

from . import document as ir
from .analysis import Analysis, Problem
from .codes import RENDER
from .protocol import (
    failure,
    read_options,
    read_request,
    request_failure,
    success,
    validate_request,
)
from .render import literal, start
from .render_client import render_client
from .render_errors import render_errors
from .render_types import render_types
from .render_validation import render_validators
from .render_wire import render_wire


def emit(raw_request: bytes) -> dict[str, Any]:
    try:
        request = read_request(raw_request)
    except ValueError as error:
        return request_failure(str(error), "PY-E001")
    try:
        mode = validate_request(request)
        options = read_options(request.get("options", {}))
    except ValueError as error:
        return request_failure(str(error), "PY-E002")
    try:
        document = ir.decode(request["ir"])
    except ir.IrError as error:
        return request_failure(str(error), "PY-E010")
    except Exception as error:
        return request_failure(f"cannot read the IR document: {error}", "PY-E010")
    analysis = Analysis(document, options, mode)
    if analysis.problems:
        return failure(analysis.problems, analysis)
    try:
        files = generated_files(analysis)
    except Exception as error:
        problem = Problem(
            RENDER,
            f"cannot write the generated package: {error}",
            "",
            "",
        )
        return failure([problem], analysis)
    return success(files, analysis)


def generated_files(analysis: Analysis) -> list[tuple[str, str]]:
    package = analysis.package
    files = [
        (f"{package}/__init__.py", render_init(analysis)),
        (f"{package}/types.py", render_types(analysis)),
        (f"{package}/errors.py", render_errors(analysis)),
        (f"{package}/client.py", render_client(analysis)),
        (f"{package}/_wire.py", render_wire()),
    ]
    if analysis.validation_plan is not None:
        files.append(
            (f"{package}/validators.py", render_validators(analysis))
        )
    for path, _ in files:
        if path.startswith("/") or ".." in path.split("/"):
            raise ValueError(f"unsafe output path {path!r}")
    return files


def render_init(analysis: Analysis) -> str:
    lines = [start(f"Generated Python client for {analysis.document.title}.")]
    lines.append("from .client import Client, Request, Response")
    lines.append("from ._wire import MISSING, Missing")
    error_names = [analysis.errors[key].name for key in analysis.error_order]
    lines.append("from .errors import (")
    for name in [
        "ApiError",
        "ConfigurationError",
        "DecodeError",
        "PonsError",
        "TransportError",
        "UnexpectedStatusError",
    ]:
        lines.append(f"    {name},")
    for name in error_names:
        lines.append(f"    {name},")
    lines.append(")")
    if analysis.types:
        lines.append("from .types import (")
        for plan in analysis.types:
            for name in plan.export_names():
                lines.append(f"    {name},")
        lines.append(")")
    lines.append("")
    exported = [
        "ApiError",
        "Client",
        "ConfigurationError",
        "DecodeError",
        "MISSING",
        "Missing",
        "PonsError",
        "Request",
        "Response",
        "TransportError",
        "UnexpectedStatusError",
    ]
    exported.extend(error_names)
    for plan in analysis.types:
        exported.extend(plan.export_names())
    lines.append("__all__ = [")
    for name in sorted(set(exported)):
        lines.append(f"    {literal(name)},")
    lines.append("]")
    return "\n".join(lines).rstrip() + "\n"
