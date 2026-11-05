from __future__ import annotations

import datetime as dt
import ipaddress
import pathlib
from decimal import Decimal
from typing import Any
from uuid import UUID

from ..config import PonsConfigurationError, PonsType
from ..diagnostics import diagnostic
from .constraints import _constraint, _int_bound, _pattern


class ScalarTypes:
    def _scalar(
        self,
        annotation: Any,
        constraints: list[Any],
        diagnostics: list[dict[str, Any]],
        site: str,
        config: dict[str, Any] | None,
    ) -> dict[str, Any] | None:
        if annotation is bool:
            return {"kind": "bool"}
        if annotation is int:
            return self._integer(constraints, diagnostics, site)
        if annotation is float:
            if any(
                _constraint(constraints, name) is not None
                for name in ("ge", "le", "gt", "lt")
            ):
                diagnostics.append(
                    diagnostic(
                        "fastapi.float_constraint",
                        f"{site} carries a numeric bound that IR floats cannot express",
                    )
                )
            return {"kind": "float", "bits": 64}
        if annotation is str:
            return self._string(constraints)
        if annotation is bytes:
            return self._bytes(config, diagnostics, site)
        if annotation is dt.datetime:
            return {"kind": "string", "format": "date-time"}
        if annotation is dt.date:
            return {"kind": "string", "format": "date"}
        if annotation is dt.time:
            return {"kind": "string", "format": "time"}
        if annotation is dt.timedelta:
            return {"kind": "string", "format": "duration"}
        if annotation is UUID:
            return {"kind": "string", "format": "uuid"}
        if annotation is Decimal:
            return {"kind": "decimal", "wire": "string"}
        if isinstance(annotation, type) and issubclass(annotation, pathlib.PurePath):
            return {"kind": "string"}
        if annotation is ipaddress.IPv4Address:
            return {"kind": "string", "format": "ipv4"}
        if annotation is ipaddress.IPv6Address:
            return {"kind": "string", "format": "ipv6"}

        return self._pydantic_special(annotation)

    def _type_override(self, override: PonsType, site: str) -> dict[str, Any]:
        kind = override.kind
        if kind == "bool":
            return {"kind": "bool"}
        if kind == "int":
            bits = override.bits if override.bits is not None else 64
            if bits not in {8, 16, 32, 64, 128}:
                raise PonsConfigurationError(f"{site} pons_type int needs bits 8, 16, 32, 64, or 128")
            wire = override.wire if override.wire is not None else "number"
            if wire not in {"number", "string"}:
                raise PonsConfigurationError(f"{site} pons_type int wire must be number or string")
            return {
                "kind": "int",
                "bits": bits,
                "signed": True if override.signed is None else override.signed,
                "wire": wire,
            }
        if kind == "float":
            bits = override.bits if override.bits is not None else 64
            if bits not in {32, 64}:
                raise PonsConfigurationError(f"{site} pons_type float needs bits 32 or 64")
            return {"kind": "float", "bits": bits}
        if kind == "decimal":
            wire = override.wire if override.wire is not None else "string"
            if wire not in {"number", "string"}:
                raise PonsConfigurationError(f"{site} pons_type decimal wire must be number or string")
            return {"kind": "decimal", "wire": wire}
        if kind == "string":
            value: dict[str, Any] = {"kind": "string"}
            if override.format is not None:
                if override.format not in {
                    "uuid",
                    "date-time",
                    "date",
                    "time",
                    "duration",
                    "uri",
                    "email",
                    "ipv4",
                    "ipv6",
                }:
                    raise PonsConfigurationError(
                        f"{site} pons_type string format {override.format!r} is not an IR format"
                    )
                value["format"] = override.format
            return value
        if kind == "bytes":
            encoding = override.encoding if override.encoding is not None else "base64"
            if encoding not in {"base64", "base64url", "hex", "array"}:
                raise PonsConfigurationError(f"{site} pons_type bytes encoding {encoding!r} is not an IR encoding")
            return {"kind": "bytes", "encoding": encoding}
        if kind == "any":
            return {"kind": "any"}
        if kind == "opaque":
            wire = override.wire if override.wire is not None else "unknown"
            if wire not in {"json", "text", "bytes", "unknown"}:
                raise PonsConfigurationError(f"{site} pons_type opaque wire {wire!r} is not an IR wire")
            return {"kind": "opaque", "codec": override.codec, "wire": wire}
        raise PonsConfigurationError(f"{site} pons_type kind {kind!r} is not supported")

    def _integer(
        self,
        constraints: list[Any],
        diagnostics: list[dict[str, Any]],
        site: str,
    ) -> dict[str, Any]:
        bits = 64
        minimum = _constraint(constraints, "ge")
        maximum = _constraint(constraints, "le")
        greater = _constraint(constraints, "gt")
        less = _constraint(constraints, "lt")
        if isinstance(greater, int) and not isinstance(greater, bool):
            minimum = greater + 1
        if isinstance(less, int) and not isinstance(less, bool):
            maximum = less - 1
        if minimum is not None:
            bound = _int_bound(minimum, bits, True)
            if bound is None:
                diagnostics.append(
                    diagnostic(
                        "fastapi.int_bound",
                        f"{site} has a minimum outside the 64-bit range; it is omitted",
                    )
                )
                minimum = None
            else:
                minimum = bound
        if maximum is not None:
            bound = _int_bound(maximum, bits, True)
            if bound is None:
                diagnostics.append(
                    diagnostic(
                        "fastapi.int_bound",
                        f"{site} has a maximum outside the 64-bit range; it is omitted",
                    )
                )
                maximum = None
            else:
                maximum = bound
        value: dict[str, Any] = {
            "kind": "int",
            "bits": bits,
            "signed": True,
            "wire": "number",
        }
        if minimum is not None:
            value["minimum"] = minimum
        if maximum is not None:
            value["maximum"] = maximum
        multiple_of = _constraint(constraints, "multiple_of")
        if multiple_of is not None:
            value["multiple_of"] = multiple_of
        return value

    def _string(self, constraints: list[Any]) -> dict[str, Any]:
        value: dict[str, Any] = {"kind": "string"}
        minimum = _constraint(constraints, "min_length")
        maximum = _constraint(constraints, "max_length")
        pattern = _pattern(constraints)
        if minimum is not None:
            value["min_length"] = minimum
        if maximum is not None:
            value["max_length"] = maximum
        if pattern is not None:
            value["pattern"] = pattern
        return value

    def _bytes(
        self,
        config: dict[str, Any] | None,
        diagnostics: list[dict[str, Any]],
        site: str,
    ) -> dict[str, Any]:
        encoding = (config or {}).get("ser_json_bytes", "utf8")
        encoding = getattr(encoding, "value", encoding)
        if encoding == "base64":
            return {"kind": "bytes", "encoding": "base64url"}
        if encoding == "hex":
            return {"kind": "bytes", "encoding": "hex"}
        diagnostics.append(
            diagnostic(
                "fastapi.bytes_utf8",
                f"{site} uses Pydantic's default UTF-8 bytes encoding; Pons records it as a string",
            )
        )
        return {"kind": "string"}

    def _pydantic_special(self, annotation: Any) -> dict[str, Any] | None:
        module = getattr(annotation, "__module__", "")
        if not module.startswith("pydantic"):
            return None
        name = getattr(annotation, "__qualname__", getattr(annotation, "__name__", ""))
        if name in {"EmailStr"}:
            return {"kind": "string", "format": "email"}
        if name.endswith("Url") and name not in {"Url"}:
            return {"kind": "string", "format": "uri"}
        if name == "IPvAnyAddress":
            return {"kind": "string"}
        if name in {"SecretStr", "SecretBytes"}:
            return {"kind": "string"}
        return None
