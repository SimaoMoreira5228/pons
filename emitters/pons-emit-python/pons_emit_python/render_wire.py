"""Load the runtime template that becomes the generated `_wire.py`."""

from __future__ import annotations

from pathlib import Path

from .render import MARKER

_TEMPLATE = Path(__file__).with_name("wire_runtime.py")


def render_wire() -> str:
    return MARKER + "\n\n" + _TEMPLATE.read_text(encoding="utf-8")
