from __future__ import annotations

import argparse
import importlib
import json
import sys
from typing import Sequence

from fastapi import FastAPI

from .config import PonsConfigurationError
from .document import build_document


def load_app(target: str) -> FastAPI:
    module_name, separator, attribute = target.partition(":")
    if not separator or not module_name or not attribute:
        raise PonsConfigurationError(
            f"{target!r} is not a <module>:<app> target, for example myapp:app"
        )
    module = importlib.import_module(module_name)
    value = module
    for part in attribute.split("."):
        value = getattr(value, part)
    if not isinstance(value, FastAPI):
        raise PonsConfigurationError(
            f"{target!r} resolves to {type(value).__name__}, not a FastAPI application"
        )
    return value


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        prog="python -m pons_fastapi",
        description="Read a FastAPI application and write Pons IR JSON to stdout.",
    )
    parser.add_argument("target", metavar="<module>:<app>")
    parser.add_argument("--pretty", action="store_true", help="indent the JSON output")
    arguments = parser.parse_args(argv)

    try:
        app = load_app(arguments.target)
        document = build_document(app)
    except (PonsConfigurationError, ImportError, AttributeError) as error:
        print(f"pons-fastapi: {error}", file=sys.stderr)
        return 1

    if arguments.pretty:
        text = json.dumps(document, indent=2, sort_keys=True)
    else:
        text = json.dumps(document, separators=(",", ":"), sort_keys=True)
    print(text)
    return 0
