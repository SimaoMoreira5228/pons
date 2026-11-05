import { typeDefinitions } from "../../types.js";
import {
  OpaqueCodecFailure,
  decodeWire,
  parseIntegerNumber,
  parseIntegerText,
  parseJson,
  parseNumber,
  type OpaqueContext,
} from "../codec.js";
import { definitionApplies, WirePolicyError } from "../wire.js";
import { ResponseHeaderDecodeError, contractMessage } from "./failures.js";
import type { HeaderDescriptor } from "./spec.js";

function mediaTypeBase(value: string): string {
  return (value.split(";", 1)[0] ?? "").trim().toLowerCase();
}

export function responseMediaTypeMatches(actual: string, expected: string): boolean {
  return mediaTypeBase(actual) === mediaTypeBase(expected);
}

function normalizeMediaParameter(part: string): [string, string] {
  const [key, ...value] = part.split("=");
  return [(key ?? "").trim().toLowerCase(), value.join("=").trim().toLowerCase()];
}

export function mediaTypeMatches(actual: string, expected: string): boolean {
  if (mediaTypeBase(actual) !== mediaTypeBase(expected)) return false;
  const actualParameters = new Map(actual.split(";").slice(1).map(normalizeMediaParameter));
  return expected.split(";").slice(1).every((part) => {
    const [key, value] = normalizeMediaParameter(part);
    return actualParameters.get(key) === value;
  });
}

export function withSuccessStatus(
  value: unknown,
  response: Response,
  schema: { status: number | string; headers?: HeaderDescriptor[] } | undefined,
  preserveWideIntegers: boolean,
  opaque: OpaqueContext,
): unknown {
  const wrapped = withResponseHeaders(value, response, schema, preserveWideIntegers, opaque);
  return schema?.status === "unknown" || schema?.status === "default"
    ? { value: wrapped, status: response.status }
    : wrapped;
}

function withResponseHeaders(
  value: unknown,
  response: Response,
  schema: { headers?: HeaderDescriptor[] } | undefined,
  preserveWideIntegers: boolean,
  opaque: OpaqueContext,
): unknown {
  if (!schema?.headers) return value;
  return { value, headers: responseHeaders(response, schema.headers, preserveWideIntegers, opaque) };
}

export function responseHeaders(
  response: Response,
  headers: HeaderDescriptor[],
  preserveWideIntegers: boolean,
  opaque: OpaqueContext,
): Record<string, unknown> {
  const values: Record<string, unknown> = Object.create(null);
  for (const header of headers) {
    const headerValue = response.headers.get(header.name);
    if (headerValue === null) continue;
    const decoded = decodeHeaderValue(headerValue, header.schema, preserveWideIntegers, opaque);
    if (header.validate) {
      let violation: unknown;
      try {
        violation = header.validate(decoded);
      } catch (cause) {
        throw new ResponseHeaderDecodeError("the response does not match the contract: the validator threw", cause);
      }
      if (violation !== null) throw new ResponseHeaderDecodeError(contractMessage(violation));
    }
    values[header.name] = decoded;
  }
  return values;
}

function decodeHeaderValue(value: string, schema: any, preserveWideIntegers: boolean, opaque: OpaqueContext): any {
  try {
    if (schema.ref) {
      const definition = typeDefinitions[schema.ref];
      if (definition && !definitionApplies(definition, "decode")) {
        throw new WirePolicyError("response header type cannot be used in this direction");
      }
      if (definition?.kind === "newtype") return decodeHeaderValue(value, definition.inner, preserveWideIntegers, opaque);
      if (definition?.kind === "bytes") return decodeHeaderValue(value, definition, preserveWideIntegers, opaque);
      if (definition?.kind === "enum") {
        const parsed = definition.repr.kind === "string" || (definition.repr.kind === "external" && !value.trim().startsWith("{"))
          ? value
          : parseJson(value, preserveWideIntegers);
        return decodeWire(parsed, schema, preserveWideIntegers, opaque);
      }
      return decodeWire(parseJson(value, preserveWideIntegers), schema, preserveWideIntegers, opaque);
    }
    if (schema.kind === "opaque") return decodeWire(value, schema, preserveWideIntegers, opaque);
    if (schema.kind === "bytes") {
      const parsed = schema.encoding === "array" ? parseJson(value, false) : value;
      return decodeWire(parsed, schema, preserveWideIntegers, opaque);
    }
    if (schema.kind === "int") {
      const parsed = schema.wire === "string"
        ? parseIntegerText(value)
        : parseIntegerNumber(value, schema.bits, preserveWideIntegers);
      return decodeWire(parsed, schema, preserveWideIntegers, opaque);
    }
    if (schema.kind === "float") return decodeWire(parseNumber(value), schema, preserveWideIntegers, opaque);
    if (schema.kind === "decimal") {
      return schema.wire === "number"
        ? decodeWire(parseNumber(value), schema, preserveWideIntegers, opaque)
        : decodeWire(value, schema, preserveWideIntegers, opaque);
    }
    if (schema.kind === "bool") {
      if (value === "true") return true;
      if (value === "false") return false;
      throw new Error("invalid boolean");
    }
    if (schema.kind === "nullable") return decodeHeaderValue(value, schema.inner, preserveWideIntegers, opaque);
    if (schema.kind === "array" || schema.kind === "map" || schema.kind === "tuple") {
      return decodeWire(parseJson(value, preserveWideIntegers), schema, preserveWideIntegers, opaque);
    }
    if (schema.kind === "string") return decodeWire(value, schema, preserveWideIntegers, opaque);
    return value;
  } catch (cause) {
    if (cause instanceof ResponseHeaderDecodeError || cause instanceof OpaqueCodecFailure) throw cause;
    throw new ResponseHeaderDecodeError(`invalid response header value ${JSON.stringify(value)}`);
  }
}
