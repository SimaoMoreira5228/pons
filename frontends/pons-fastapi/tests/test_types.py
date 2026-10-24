from __future__ import annotations

from datetime import date, datetime, time, timedelta
from decimal import Decimal
from enum import Enum
from typing import Literal
from uuid import UUID

from pydantic import BaseModel, ConfigDict, Field

from pons_fastapi.pydantic_types import INPUT, OUTPUT, TypeRegistry


class Color(str, Enum):
    RED = "red"
    BLUE = "blue"


class Inner(BaseModel):
    value: int


class MappingModel(BaseModel):
    flag: bool
    count: int
    ratio: float
    name: str = Field(min_length=2, max_length=8, pattern="^[a-z]+$")
    blob: bytes
    created: datetime
    day: date
    at: time
    elapsed: timedelta
    identifier: UUID
    amount: Decimal
    color: Color
    status: Literal["open", "closed"]
    tags: list[str]
    unique: set[int]
    pair: tuple[str, int]
    scores: dict[str, int]
    inner: Inner
    maybe: str | None
    bounded: int = Field(ge=1, le=10, multiple_of=2)


class Base64Bytes(BaseModel):
    model_config = ConfigDict(ser_json_bytes="base64")

    blob: bytes


class Directional(BaseModel):
    name: str
    nickname: str | None = None


class OutputOnly(BaseModel):
    name: str
    nickname: str | None = None


class IntLiteral(BaseModel):
    state: Literal[3]


def lower(annotation, direction):
    registry = TypeRegistry()
    ref, diagnostics = registry.lower(annotation, direction)
    return registry, ref, diagnostics


def fields_of(registry: TypeRegistry, model: type[BaseModel]) -> dict[str, dict]:
    definition = registry.definitions()[f"{model.__module__}.{model.__qualname__}"]
    return {field["wire"]: field for field in definition["fields"]}


def test_scalars_and_containers_map_to_ir() -> None:
    registry, ref, _ = lower(MappingModel, INPUT)
    assert ref == {"ref": f"{MappingModel.__module__}.{MappingModel.__qualname__}"}
    fields = fields_of(registry, MappingModel)

    assert fields["flag"]["ty"] == {"kind": "bool"}
    assert fields["count"]["ty"]["kind"] == "int"
    assert fields["count"]["ty"]["bits"] == 64
    assert fields["count"]["ty"]["wire"] == "number"
    assert fields["ratio"]["ty"] == {"kind": "float", "bits": 64}
    assert fields["name"]["ty"]["min_length"] == 2
    assert fields["name"]["ty"]["max_length"] == 8
    assert fields["name"]["ty"]["pattern"] == "^[a-z]+$"
    assert fields["created"]["ty"] == {"kind": "string", "format": "date-time"}
    assert fields["day"]["ty"] == {"kind": "string", "format": "date"}
    assert fields["at"]["ty"] == {"kind": "string", "format": "time"}
    assert fields["elapsed"]["ty"] == {"kind": "string", "format": "duration"}
    assert fields["identifier"]["ty"] == {"kind": "string", "format": "uuid"}
    assert fields["amount"]["ty"] == {"kind": "decimal", "wire": "string"}
    assert fields["color"]["ty"] == {"ref": f"{Color.__module__}.{Color.__qualname__}"}
    assert fields["status"]["ty"]["ref"].endswith("status::literal")
    assert fields["tags"]["ty"]["kind"] == "array"
    assert fields["unique"]["ty"]["unique"] is True
    assert fields["pair"]["ty"]["kind"] == "tuple"
    assert len(fields["pair"]["ty"]["items"]) == 2
    assert fields["scores"]["ty"]["kind"] == "map"
    assert fields["scores"]["ty"]["key"] == {"kind": "string"}
    assert fields["inner"]["ty"] == {"ref": f"{Inner.__module__}.{Inner.__qualname__}"}
    assert fields["maybe"]["ty"]["kind"] == "nullable"
    assert fields["bounded"]["ty"]["minimum"] == 1
    assert fields["bounded"]["ty"]["maximum"] == 10
    assert fields["bounded"]["ty"]["multiple_of"] == 2


def test_bytes_honors_the_pydantic_serializer() -> None:
    registry, _, _ = lower(Base64Bytes, INPUT)
    fields = fields_of(registry, Base64Bytes)
    assert fields["blob"]["ty"] == {"kind": "bytes", "encoding": "base64url"}


def test_default_bytes_records_utf8_as_a_string() -> None:
    registry, _, _ = lower(MappingModel, INPUT)
    fields = fields_of(registry, MappingModel)
    assert fields["blob"]["ty"] == {"kind": "string"}
    codes = [entry["code"] for entry in fields["blob"]["diagnostics"]]
    assert "fastapi.bytes_utf8" in codes


def test_enum_is_emitted_once_for_both_directions() -> None:
    registry, _, _ = lower(MappingModel, INPUT)
    definition = registry.definitions()[f"{Color.__module__}.{Color.__qualname__}"]
    assert definition["direction"] == "input"
    assert definition["repr"] == {"kind": "string"}
    assert [variant["wire"] for variant in definition["variants"]] == ["red", "blue"]


def test_direction_split_follows_presence() -> None:
    registry = TypeRegistry()
    registry.collect(Directional, INPUT)
    registry.collect(Directional, OUTPUT)
    registry.analyze()
    registry.build()
    definitions = registry.definitions()
    base = f"{Directional.__module__}.{Directional.__qualname__}"
    input_definition = definitions[f"{base}#input"]
    output_definition = definitions[f"{base}#output"]

    assert input_definition["direction"] == "input"
    assert output_definition["direction"] == "output"
    assert input_definition["origin"] == base
    assert output_definition["origin"] == base
    input_fields = {field["wire"]: field for field in input_definition["fields"]}
    output_fields = {field["wire"]: field for field in output_definition["fields"]}
    assert input_fields["nickname"]["presence"] == "optional"
    assert input_fields["nickname"]["default"] is None
    assert output_fields["nickname"]["presence"] == "required"


def test_output_only_model_is_required_and_unsplit() -> None:
    registry, _, _ = lower(OutputOnly, OUTPUT)
    definitions = registry.definitions()
    base = f"{OutputOnly.__module__}.{OutputOnly.__qualname__}"
    assert base in definitions
    assert definitions[base]["direction"] == "output"
    fields = {field["wire"]: field for field in definitions[base]["fields"]}
    assert fields["nickname"]["presence"] == "required"
    assert "default" not in fields["nickname"]


def test_single_integer_literal_becomes_a_bounded_integer() -> None:
    registry, _, _ = lower(IntLiteral, INPUT)
    fields = fields_of(registry, IntLiteral)
    assert fields["state"]["ty"]["minimum"] == 3
    assert fields["state"]["ty"]["maximum"] == 3


def test_unsupported_union_is_any_with_a_diagnostic() -> None:
    registry, _, diagnostics = lower(int | str, INPUT)
    assert registry.definitions() == {}
    assert any(entry["code"] == "fastapi.unsupported_union" for entry in diagnostics)
