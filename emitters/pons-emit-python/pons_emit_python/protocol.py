"""Emit responses, diagnostics, coverage, and option handling."""

from __future__ import annotations

import json
import keyword
from typing import Any

from .analysis import Analysis, Problem
from .manifest import PROTOCOL_VERSION, TARGET


def coverage_block(analysis: Analysis | None) -> dict[str, list[str]]:
    if analysis is None:
        return {"exact": [], "opaque": [], "lossy": [], "unsupported": []}
    return analysis.coverage()


def diagnostics(problems: list[Problem]) -> list[dict[str, str]]:
    result: list[dict[str, str]] = []
    for problem in problems:
        item = {
            "severity": "error",
            "code": problem.code,
            "message": problem.message,
        }
        if problem.origin:
            item["origin"] = problem.origin
        result.append(item)
    return result


def failure(
    problems: list[Problem], analysis: Analysis | None = None
) -> dict[str, Any]:
    messages = [problem.message for problem in problems]
    return {
        "protocol": PROTOCOL_VERSION,
        "files": [],
        "coverage": coverage_block(analysis),
        "diagnostics": diagnostics(problems),
        "error": "; ".join(messages) or "emission failed",
    }


def request_failure(message: str, code: str = "PY-E001") -> dict[str, Any]:
    return failure([Problem(code, message, "", "")])


def success(files: list[tuple[str, str]], analysis: Analysis) -> dict[str, Any]:
    return {
        "protocol": PROTOCOL_VERSION,
        "files": [
            {"path": path, "content": content, "encoding": "utf8"}
            for path, content in files
        ],
        "coverage": analysis.coverage(),
        "diagnostics": [],
    }


def package_option(options: object) -> str:
    name = "pons_client"
    if options is None:
        options = {}
    if not isinstance(options, dict):
        raise ValueError("options must be an object")
    for key in sorted(options):
        if key != "package":
            raise ValueError(
                f"option {key!r} is not one of the options this emitter declares: package"
            )
        value = options[key]
        if not isinstance(value, str):
            raise ValueError('option "package" is not a string')
        name = value
    validate_package(name)
    return name


def validate_package(name: str) -> None:
    if not name or not name.isidentifier():
        raise ValueError(f"the package option {name!r} is not a Python identifier")
    if keyword.iskeyword(name):
        raise ValueError(f"the package option {name!r} is a Python keyword")
    if name.startswith("__") and name.endswith("__"):
        raise ValueError(f"the package option {name!r} is a dunder")
    if name == "main":
        raise ValueError("the package option cannot be main")


def read_request(raw: bytes) -> dict[str, Any]:
    try:
        value = json.loads(raw)
    except (ValueError, UnicodeDecodeError) as error:
        raise ValueError(f"cannot read the emit request: {error}") from error
    if not isinstance(value, dict):
        raise ValueError("the emit request must be an object")
    allowed = {"protocol", "source_ir_version", "mode", "ir", "options", "target"}
    unknown = sorted(set(value) - allowed)
    if unknown:
        raise ValueError(f"the emit request has the unknown member {unknown[0]!r}")
    return value


def validate_request(request: dict[str, Any]) -> str:
    if request.get("protocol") != PROTOCOL_VERSION:
        raise ValueError(
            f"the request uses protocol {request.get('protocol')!r}; this emitter speaks protocol {PROTOCOL_VERSION}"
        )
    mode = request.get("mode")
    if mode not in {"strict", "lossy"}:
        raise ValueError(f"the request mode {mode!r} is not strict or lossy")
    if request.get("target") != TARGET:
        raise ValueError(
            f"this emitter is named {TARGET}, but the request targets {request.get('target')!r}"
        )
    if request.get("ir") is None:
        raise ValueError("the request has no IR document")
    return mode
