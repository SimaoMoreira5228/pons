from __future__ import annotations

import re
from typing import Any

_WORD = re.compile(r"[A-Z]+(?=[A-Z][a-z])|[A-Z]?[a-z]+|[A-Z]+|[0-9]+")


def split_words(text: str) -> list[str]:
    words: list[str] = []
    for chunk in re.split(r"[^0-9A-Za-z]+", text):
        if not chunk:
            continue
        words.extend(match.group(0).lower() for match in _WORD.finditer(chunk))
    return words


def ident(text: str, fallback: str = "value") -> list[str]:
    return split_words(text) or [fallback]


def type_base_id(cls: Any) -> str:
    qualname = getattr(cls, "__qualname__", getattr(cls, "__name__", "anonymous"))
    qualname = qualname.replace(".<locals>.", ".")
    module = getattr(cls, "__module__", "python")
    return f"{module}.{qualname}"


def type_name(cls: Any) -> list[str]:
    name = getattr(cls, "__name__", "anonymous").split("[", 1)[0]
    return ident(name, "anonymous")


def operation_path(text: str) -> list[list[str]]:
    path = [words for part in text.split(".") if (words := split_words(part))]
    return path or [["operation"]]


def error_name(text: str) -> list[str]:
    return ident(text, "error")
