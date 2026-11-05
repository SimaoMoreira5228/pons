import { copyRecord, encodeWire, type OpaqueContext } from "../codec.js";
import { WirePolicyError, hasOpaque, validateWire } from "../wire.js";

export function encodePathSegment(value: string): string {
  if (value === "." || value === "..") throw new WirePolicyError("path segment cannot be a dot segment");
  return encodeURIComponent(value);
}

export function preflightPath(
  values: Record<string, unknown> | undefined,
  descriptors: Array<{ name: string; required: boolean; schema: unknown }>,
  opaque: OpaqueContext,
): Record<string, unknown> {
  const output = copyRecord(values ?? {});
  for (const descriptor of descriptors) {
    const present = Object.hasOwn(output, descriptor.name);
    const value = output[descriptor.name];
    if (!present || value === undefined) {
      if (descriptor.required) {
        throw new WirePolicyError(`missing required path parameter ${descriptor.name}`);
      }
      continue;
    }
    output[descriptor.name] = encodeWire(value, descriptor.schema, opaque);
  }
  return output;
}

export function preflightQuery(value: unknown, schema: unknown, opaque: OpaqueContext): unknown {
  if (schema == null) return value;
  if (value === undefined) throw new WirePolicyError("missing request query");
  return encodeWire(value, schema, opaque);
}

export function preflightHeaders(
  values: Record<string, unknown> | undefined,
  descriptors: Array<{ name: string; required: boolean; schema: unknown }>,
  opaque: OpaqueContext,
): Record<string, unknown> {
  const output = copyRecord(values ?? {});
  for (const descriptor of descriptors) {
    const present = Object.hasOwn(output, descriptor.name);
    const value = output[descriptor.name];
    if (!present || value === undefined || value === null) {
      if (descriptor.required) {
        throw new WirePolicyError(`missing required request header ${descriptor.name}`);
      }
      delete output[descriptor.name];
      continue;
    }
    output[descriptor.name] = encodeWire(value, descriptor.schema, opaque);
  }
  return output;
}

export function preflightBody(value: unknown, schema: unknown, content: string | undefined, opaque: OpaqueContext): unknown {
  if (value === undefined || schema == null) return value;
  if (["json", "problem_json", "form", "multipart"].includes(content ?? "") || hasOpaque(schema)) {
    return encodeWire(value, schema, opaque);
  }
  validateWire(value, schema, "encode", opaque.allowUnsafeWideIntegers === true);
  return value;
}
