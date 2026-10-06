import { typeDefinitions } from "../types.js";
import {
  WirePolicyError,
  collectFields,
  collectMapValues,
  flattenedOpaqueSchema,
  hasOpaque,
  isFlattenedOpaque,
  isRecord,
  opennessValue,
  selectUntaggedVariant,
  validateWire,
} from "./wire.js";

export type OpaqueWire = "json" | "text" | "bytes" | "unknown";
export type OpaqueCodec = { encode(value: unknown, wire: OpaqueWire): unknown; decode(value: unknown, wire: OpaqueWire): unknown };
export type OpaqueCodecRegistry = Record<string, OpaqueCodec>;
export type OpaqueContext = { codecs?: OpaqueCodecRegistry | undefined; required: boolean; allowUnsafeWideIntegers?: boolean | undefined };

export class OpaqueCodecError extends Error {
  constructor(readonly codec: string, readonly phase: "encode" | "decode", readonly sourceCause: unknown) {
    super(`opaque codec failed for ${codec}`);
  }
}

function transformOpaque(
  value: any,
  schema: any,
  mode: "encode" | "decode",
  opaque: OpaqueContext,
): any {
  const hasCodec = Object.hasOwn(opaque.codecs ?? {}, schema.codec);
  const codec = hasCodec ? opaque.codecs?.[schema.codec] : undefined;
  if (!hasCodec || codec === undefined) {
    if (opaque.required) throw new OpaqueCodecError(schema.codec, mode, new Error("missing opaque codec"));
    return value;
  }
  try {
    return mode === "encode" ? codec.encode(value, schema.wire) : codec.decode(value, schema.wire);
  } catch (cause) {
    if (cause instanceof OpaqueCodecError) throw cause;
    throw new OpaqueCodecError(schema.codec, mode, cause);
  }
}

export function encodeWire(value: any, schema: any, opaque: OpaqueContext): any {
  const selectedVariant = validateWire(value, schema, "encode", opaque.allowUnsafeWideIntegers === true);
  if (value == null || schema == null) return value;
  if (schema.ref) {
    const definition = typeDefinitions[schema.ref];
    if (!definition) return value;
    if (schema.variant && definition.kind === "enum") {
      return transformVariant(value, definition, definition.variants.find((variant: any) => variant.wire === schema.variant), "encode", false, opaque);
    }
    if (definition.kind === "struct") return transformStruct(value, definition, "encode", false, opaque);
    if (definition.kind === "enum") return transformEnum(value, definition, "encode", selectedVariant, false, opaque);
    if (definition.kind === "newtype") return encodeWire(value, definition.inner, opaque);
    return value;
  }
  if (schema.kind === "opaque") return transformOpaque(value, schema, "encode", opaque);
  if (schema.kind === "bytes") return encodeBytes(value, schema.encoding);
  if (schema.kind === "nullable") return encodeWire(value, schema.inner, opaque);
  if (schema.kind === "array") return value.map((item: unknown) => encodeWire(item, schema.item, opaque));
  if (schema.kind === "tuple") return schema.items.map((item: unknown, index: number) => encodeWire(value[index], item, opaque));
  if (schema.kind === "map") {
    const output: Record<string, unknown> = Object.create(null);
    for (const [key, item] of Object.entries(value)) output[key] = encodeWire(item, schema.value, opaque);
    return output;
  }
  return value;
}

export function decodeWire(value: any, schema: any, preserveWideIntegers = false, opaque: OpaqueContext = { required: false }): any {
  const selectedVariant = validateWire(value, schema, "decode", opaque.allowUnsafeWideIntegers === true);
  if (schema?.kind === "float" && typeof value === "bigint") return Number(value);
  if (schema?.kind === "decimal" && schema.wire === "number" && typeof value === "bigint") return Number(value);
  if (value == null || schema == null) return value;
  if (schema.kind === "int") {
    if (schema.bits <= 53 && typeof value === "bigint") return Number(value);
    if (schema.wire === "number" && schema.bits > 53 && preserveWideIntegers && typeof value === "number") return BigInt(value);
  }
  if (schema.ref) {
    const definition = typeDefinitions[schema.ref];
    if (!definition) return value;
    if (schema.variant && definition.kind === "enum") {
      return transformVariant(value, definition, definition.variants.find((variant: any) => variant.wire === schema.variant), "decode", preserveWideIntegers, opaque);
    }
    if (definition.kind === "struct") return transformStruct(value, definition, "decode", preserveWideIntegers, opaque);
    if (definition.kind === "enum") return transformEnum(value, definition, "decode", selectedVariant, preserveWideIntegers, opaque);
    if (definition.kind === "newtype") return decodeWire(value, definition.inner, preserveWideIntegers, opaque);
    return value;
  }
  if (schema.kind === "opaque") return transformOpaque(value, schema, "decode", opaque);
  if (schema.kind === "bytes") return decodeBytes(value, schema.encoding);
  if (schema.kind === "nullable") return decodeWire(value, schema.inner, preserveWideIntegers, opaque);
  if (schema.kind === "array") return value.map((item: unknown) => decodeWire(item, schema.item, preserveWideIntegers, opaque));
  if (schema.kind === "tuple") return value.map((item: unknown, index: number) => decodeWire(value[index], item, preserveWideIntegers, opaque));
  if (schema.kind === "map") {
    const output: Record<string, unknown> = Object.create(null);
    for (const [key, item] of Object.entries(value)) output[key] = decodeWire(item, schema.value, preserveWideIntegers, opaque);
    return output;
  }
  return value;
}

function transformStruct(value: any, definition: any, mode: "encode" | "decode", preserveWideIntegers = false, opaque: OpaqueContext = { required: false }): any {
  const transform = (item: unknown, itemSchema: any) => mode === "encode"
    ? encodeWire(item, itemSchema, opaque)
    : decodeWire(item, itemSchema, preserveWideIntegers, opaque);
  let current = value;
  for (const item of definition.flattened ?? []) {
    const carrier = flattenedOpaqueSchema(item);
    if (carrier !== undefined) {
      current = transform(current, carrier);
      break;
    }
  }
  if (!isRecord(current)) return current;
  const fields = collectFields(definition, mode);
  const declaredFields = collectFields(definition);
  const flattenedOpaque = (definition.flattened ?? []).some((item: any) => isFlattenedOpaque(item));
  const flattenedMaps = collectMapValues(definition.flattened ?? []);
  const known = new Set(declaredFields.map((field: any) => field.wire));
  const openness = definition.openness ?? "preserve_opaque";
  const policy = typeof openness === "string" ? openness : Object.keys(openness)[0];
  const output: Record<string, unknown> = Object.create(null);
  for (const key of Object.keys(current)) {
    if (!known.has(key)) output[key] = current[key];
  }
  for (const field of fields) {
    if (Object.hasOwn(current, field.wire) && current[field.wire] !== undefined) {
      output[field.wire] = !flattenedOpaque || hasOpaque(field.ty)
        ? transform(current[field.wire], field.ty)
        : current[field.wire];
    }
  }
  const additional = policy === "preserve_typed" ? opennessValue(openness.preserve_typed.value) : flattenedMaps[0];
  if (additional) {
    for (const key of Object.keys(output)) {
      if (!known.has(key) && output[key] !== undefined) output[key] = transform(output[key], additional);
    }
  }
  return output;
}

function transformEnum(value: any, definition: any, mode: "encode" | "decode", selectedVariant?: any, preserveWideIntegers = false, opaque: OpaqueContext = { required: false }): any {
  const repr = definition.repr;
  let variant: any;
  let payload: unknown;
  if (repr.kind === "string") return value;
  if (repr.kind === "external") {
    if (typeof value === "string") return value;
    const wire = Object.keys(value)[0] ?? "";
    variant = definition.variants.find((entry: any) => entry.wire === wire);
    payload = value[wire];
  } else if (repr.kind === "internal") {
    const tag = Object.hasOwn(value, repr.tag) ? value[repr.tag] : undefined;
    variant = definition.variants.find((entry: any) => entry.wire === tag);
    payload = value;
  } else if (repr.kind === "adjacent") {
    const tag = Object.hasOwn(value, repr.tag) ? value[repr.tag] : undefined;
    variant = definition.variants.find((entry: any) => entry.wire === tag);
    payload = Object.hasOwn(value, repr.content) ? value[repr.content] : undefined;
  } else {
    variant = selectedVariant ?? selectUntaggedVariant(value, definition, undefined, mode, opaque.allowUnsafeWideIntegers === true);
    payload = value;
  }
  return transformVariant(value, definition, variant, mode, preserveWideIntegers, opaque, payload);
}

function transformVariant(value: any, definition: any, variant: any, mode: "encode" | "decode", preserveWideIntegers = false, opaque: OpaqueContext = { required: false }, payloadValue: any = value): any {
  if (!variant) return value;
  if (variant.payload.kind === "unit") return isRecord(value) ? copyRecord(value) : value;
  const payload = variant.payload;
  const transform = (item: unknown, itemSchema: any) => mode === "encode"
    ? encodeWire(item, itemSchema, opaque)
    : decodeWire(item, itemSchema, preserveWideIntegers, opaque);
  const transformPayload = (current: any): any => {
    if (payload.kind === "newtype") return transform(current, payload.ty);
    if (payload.kind === "tuple") return payload.items.map((item: unknown, index: number) => transform(current[index], item));
    if (payload.kind === "struct") return transformStruct(current, { kind: "struct", direction: definition.direction, fields: payload.fields }, mode, preserveWideIntegers, opaque);
    return current;
  };
  const repr = definition.repr;
  if (repr.kind === "internal" && isRecord(payloadValue)) {
    const { [repr.tag]: _tag, ...payloadObject } = payloadValue as any;
    const transformed = transformPayload(payloadObject);
    const output: Record<string, unknown> = isRecord(transformed) ? copyRecord(transformed) : Object.create(null);
    output[repr.tag] = variant.wire;
    return output;
  }
  if (repr.kind === "external") {
    const wire = variant.wire;
    if (typeof value === "string") return value;
    const output: Record<string, unknown> = Object.create(null);
    output[wire] = transformPayload(payloadValue);
    return output;
  }
  if (repr.kind === "adjacent") {
    const output: Record<string, unknown> = isRecord(value) ? copyRecord(value) : Object.create(null);
    output[repr.content] = transformPayload(payloadValue);
    return output;
  }
  return transformPayload(payloadValue);
}

function encodeBytes(value: Uint8Array | number[], encoding: string): string | number[] {
  const bytes = value instanceof Uint8Array ? value : new Uint8Array(value);
  if (encoding === "array") return Array.from(bytes);
  if (encoding === "hex") return Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("");
  const binary = Array.from(bytes, (byte) => String.fromCharCode(byte)).join("");
  const base64 = btoa(binary);
  return encoding === "base64url" ? base64.replaceAll("+", "-").replaceAll("/", "_").replace(/=+$/, "") : base64;
}

function decodeBytes(value: string | number[], encoding: string): Uint8Array {
  if (encoding === "array") return new Uint8Array(value as number[]);
  if (encoding === "hex") return new Uint8Array((value as string).match(/.{1,2}/g)?.map((byte) => parseInt(byte, 16)) ?? []);
  let base64 = value as string;
  if (encoding === "base64url") {
    base64 = base64.replaceAll("-", "+").replaceAll("_", "/");
    base64 += "=".repeat((4 - base64.length % 4) % 4);
  } else if (base64.length % 4 !== 0) {
    throw new WirePolicyError("invalid base64 bytes");
  }
  return Uint8Array.from(atob(base64), (byte) => byte.charCodeAt(0));
}

export function copyRecord(value: Record<string, unknown>): Record<string, unknown> {
  const output: Record<string, unknown> = Object.create(null);
  for (const [key, item] of Object.entries(value)) output[key] = item;
  return output;
}

function exactIntegerToken(token: string): bigint | undefined {
  const match = /^(-?)(\d+)(?:\.(\d+))?(?:[eE]([+-]?\d+))?$/.exec(token);
  if (match === null) return undefined;
  const fraction = match[3] ?? "";
  const exponent = Number(match[4] ?? "0");
  if (!Number.isInteger(exponent) || Math.abs(exponent) > 10_000) return undefined;
  const coefficient = BigInt(match[2] + fraction);
  const scale = fraction.length - exponent;
  if (scale <= 0) {
    const magnitude = coefficient * (10n ** BigInt(-scale));
    return match[1] === "-" ? -magnitude : magnitude;
  }
  const divisor = 10n ** BigInt(scale);
  if (coefficient % divisor !== 0n) return undefined;
  const magnitude = coefficient / divisor;
  return match[1] === "-" ? -magnitude : magnitude;
}

export function parseJson(text: string, preserveWideIntegers: boolean): unknown {
  if (!preserveWideIntegers) return JSON.parse(text);
  const strings: string[] = [];
  const pending: unknown[] = [JSON.parse(text)];
  while (pending.length > 0) {
    const value = pending.pop();
    if (typeof value === "string") {
      strings.push(value);
    } else if (Array.isArray(value)) {
      for (const item of value) pending.push(item);
    } else if (value !== null && typeof value === "object") {
      for (const [key, item] of Object.entries(value)) {
        strings.push(key);
        pending.push(item);
      }
    }
  }
  let marker = "\u0000pons-bigint:";
  while (strings.some((value) => value.startsWith(marker))) marker += "\u0000";
  const at = (position: number): string => text[position] ?? "";
  let output = "";
  let index = 0;
  let inString = false;
  while (index < text.length) {
    const character = at(index);
    if (inString) {
      output += character;
      if (character === "\\") {
        index += 1;
        if (index < text.length) output += at(index);
      } else if (character === "\"") {
        inString = false;
      }
      index += 1;
      continue;
    }
    if (character === "\"") {
      inString = true;
      output += character;
      index += 1;
      continue;
    }
    if (character === "-" || (character >= "0" && character <= "9")) {
      const start = index;
      if (character === "-") index += 1;
      while (index < text.length && at(index) >= "0" && at(index) <= "9") index += 1;
      if (at(index) === ".") {
        index += 1;
        while (index < text.length && at(index) >= "0" && at(index) <= "9") index += 1;
      }
      if (at(index) === "e" || at(index) === "E") {
        index += 1;
        if (at(index) === "+" || at(index) === "-") index += 1;
        while (index < text.length && at(index) >= "0" && at(index) <= "9") index += 1;
      }
      const token = text.slice(start, index);
      const exactInteger = exactIntegerToken(token);
      if (exactInteger !== undefined && !Number.isSafeInteger(Number(token))) {
        output += JSON.stringify(marker + exactInteger.toString());
      } else {
        output += token;
      }
      continue;
    }
    output += character;
    index += 1;
  }
  return JSON.parse(output, (_key: string, value: unknown) => {
    if (typeof value !== "string" || !value.startsWith(marker)) return value;
    const token = value.slice(marker.length);
    return /^-?\d+$/.test(token) ? BigInt(token) : value;
  });
}

export function stringifyCarrier(value: unknown): string {
  if (typeof value === "string") return value;
  if (typeof value === "bigint") return value.toString();
  return stringifyJson(value);
}

export function stringifyJson(value: unknown): string {
  if (typeof value === "bigint") return value.toString();
  if (value === null || typeof value !== "object") return JSON.stringify(value) ?? "null";
  if (Array.isArray(value)) return `[${value.map((item) => stringifyJson(item)).join(",")}]`;
  const prototype = Object.getPrototypeOf(value);
  if (prototype !== Object.prototype && prototype !== null) return JSON.stringify(value) ?? "null";
  return `{${Object.entries(value as Record<string, unknown>)
    .filter(([, item]) => item !== undefined)
    .map(([key, item]) => `${JSON.stringify(key)}:${stringifyJson(item)}`)
    .join(",")}}`;
}

export function parseIntegerText(value: string): string {
  const normalized = value.trim();
  if (!/^-?\d+$/.test(normalized)) throw new Error("invalid integer");
  return normalized;
}

export function parseIntegerNumber(value: string, bits: number, preserveWideIntegers: boolean): number | bigint {
  const integer = BigInt(parseIntegerText(value));
  if (bits > 53 && preserveWideIntegers) return integer;
  return Number(integer);
}

export function parseNumber(value: string): number {
  const normalized = value.trim();
  if (!/^[+-]?(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][+-]?\d+)?$/.test(normalized)) {
    throw new Error("invalid number");
  }
  const parsed = Number(normalized);
  if (!Number.isFinite(parsed)) throw new Error("invalid number");
  return parsed;
}
