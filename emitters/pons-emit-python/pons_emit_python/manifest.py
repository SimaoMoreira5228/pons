"""The manifest the Python emitter prints for `describe`."""

from __future__ import annotations

PROTOCOL_VERSION = 3
IR_VERSION = "0.3"
TARGET = "python"
EMITTER_VERSION = "0.1.0"


def build_manifest() -> dict:
    return {
        "name": TARGET,
        "version": EMITTER_VERSION,
        "protocol": {"min": "3.0", "max": "3.0"},
        "ir_version": {"min": "0.3", "max": "0.3"},
        "capabilities": {
            "streaming": [],
            "pagination": ["pagination.cursor", "pagination.offset"],
            "enum_reprs": ["string", "external", "internal", "adjacent"],
            "direction_split": True,
            "provides": True,
            "custom_auth": False,
            "flattened": False,
            "wide_integers": True,
            "opaque": False,
            "object_openness": ["reject", "preserve_typed", "preserve_opaque"],
            "extensions": {},
            "validation": {"backends": [], "capabilities": {}},
        },
        "option_schema": {
            "options": {"package": {"type": "string", "required": False}}
        },
    }
