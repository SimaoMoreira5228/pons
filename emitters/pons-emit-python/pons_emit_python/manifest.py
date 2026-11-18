"""The manifest the Python emitter prints for `describe`."""

from __future__ import annotations

PROTOCOL_VERSION = 3
IR_VERSION = "0.3"
TARGET = "python"
EMITTER_VERSION = "0.1.0"

VALIDATION_CAPABILITIES = {
    "validation.scalars": "exact",
    "validation.any": "exact",
    "validation.integer": "exact",
    "validation.float_bounds": "exact",
    "validation.wide_integer": "exact",
    "validation.decimal": "exact",
    "validation.formats": "exact",
    "validation.string_constraints": "exact",
    "validation.bytes": "exact",
    "validation.nullable": "exact",
    "validation.array": "exact",
    "validation.unique_array": "exact",
    "validation.map": "exact",
    "validation.tuple": "exact",
    "validation.object": "exact",
    "validation.openness.preserve_typed": "exact",
    "validation.openness.preserve_opaque": "exact",
    "validation.enum.string": "exact",
    "validation.enum.external": "exact",
    "validation.enum.internal": "exact",
    "validation.enum.adjacent": "exact",
    "validation.enum.untagged": "exact",
    "validation.opaque": "exact",
    "validation.recursion": "exact",
    "validation.variant_narrowing": "exact",
    "validation.headers": "exact",
    "validation.stream": "exact",
}


def build_manifest() -> dict:
    return {
        "name": TARGET,
        "version": EMITTER_VERSION,
        "protocol": {"min": "3.0", "max": "3.0"},
        "ir_version": {"min": "0.3", "max": "0.3"},
        "capabilities": {
            "streaming": ["sse", "ndjson", "bytes", "websocket"],
            "pagination": ["pagination.cursor", "pagination.offset"],
            "enum_reprs": [
                "string",
                "external",
                "internal",
                "adjacent",
                "untagged",
            ],
            "direction_split": True,
            "provides": True,
            "custom_auth": True,
            "flattened": False,
            "wide_integers": True,
            "opaque": True,
            "object_openness": ["reject", "preserve_typed", "preserve_opaque"],
            "extensions": {},
            "validation": {
                "backends": ["builtin"],
                "capabilities": dict(VALIDATION_CAPABILITIES),
            },
        },
        "option_schema": {
            "options": {
                "package": {"type": "string", "required": False},
                "validation": {"type": "object", "required": False},
            }
        },
    }
