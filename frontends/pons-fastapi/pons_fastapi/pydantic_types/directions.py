from __future__ import annotations

from typing import Literal

INPUT = "input"
OUTPUT = "output"
BOTH = "both"
Direction = Literal["input", "output", "both"]


def direction_for(usage: set[Direction]) -> Direction:
    if INPUT in usage and OUTPUT in usage:
        return BOTH
    if OUTPUT in usage:
        return OUTPUT
    return INPUT
