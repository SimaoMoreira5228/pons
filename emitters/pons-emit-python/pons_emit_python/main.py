"""Command-line entry point for the Python external emitter."""

from __future__ import annotations

import json
import sys

from .emit import emit
from .manifest import build_manifest


def main(argv: list[str]) -> int:
    if len(argv) != 1:
        print("usage: pons-emit-python <describe|emit>", file=sys.stderr)
        return 2
    if argv[0] == "describe":
        write(build_manifest())
        return 0
    if argv[0] == "emit":
        write(emit(sys.stdin.buffer.read()))
        return 0
    print(
        f"unknown command {argv[0]!r}; use describe or emit",
        file=sys.stderr,
    )
    return 2


def write(value: object) -> None:
    json.dump(value, sys.stdout, indent=2, ensure_ascii=False)
    sys.stdout.write("\n")
