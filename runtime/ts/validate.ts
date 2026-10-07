import { typeDefinitions } from "../types.js";
import {
  collectFields,
  collectMapValues,
  definitionApplies,
  fieldOutKey,
  flattenedOpaqueSchema,
  isRecord,
  opennessValue,
} from "./wire.js";

export type ValidationIssue = {
  readonly path: readonly (string | number)[];
  readonly message: string;
};

export type Validator = (value: unknown) => ValidationIssue | null;

const CANONICAL_INTEGER = /^(0|[1-9][0-9]*)$/;
const CANONICAL_SIGNED_INTEGER = /^-?(0|[1-9][0-9]*)$/;
const DECIMAL = /^-?(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][+-]?\d+)?$/;
const DATE = /^(\d{4})-(\d{2})-(\d{2})$/;
const TIME = /^(\d{2}):(\d{2}):(\d{2})(?:\.\d+)?(?:[Zz]|([+-])(\d{2}):(\d{2}))$/;
const DATE_TIME = /^(\d{4})-(\d{2})-(\d{2})[Tt](\d{2}):(\d{2}):(\d{2})(?:\.\d{1,3})?(?:[Zz]|([+-])(\d{2}):(\d{2}))$/;
const UUID = /^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$/;
const EMAIL = /^(?:[A-Za-z0-9_'+\-]+\.)*[A-Za-z0-9_'+\-]*[A-Za-z0-9_+-]@(?:[A-Za-z0-9][A-Za-z0-9\-]*\.)+[A-Za-z]{2,}$/;
const DURATION = /^P(?:(\d+W)|(?!.*W)(?=\d|T\d)(\d+Y)?(\d+M)?(\d+D)?(T(?=\d)(\d+H)?(\d+M)?(\d+([.,]\d+)?S)?)?)$/;
const IPV4 = /^(?:(?:25[0-5]|2[0-4][0-9]|1[0-9][0-9]|[1-9]?[0-9])\.){3}(?:25[0-5]|2[0-4][0-9]|1[0-9][0-9]|[1-9]?[0-9])$/;
const IPV6 = /^(([0-9a-fA-F]{1,4}:){7}[0-9a-fA-F]{1,4}|([0-9a-fA-F]{1,4}:){1,7}:|([0-9a-fA-F]{1,4}:){1,6}:[0-9a-fA-F]{1,4}|([0-9a-fA-F]{1,4}:){1,5}(:[0-9a-fA-F]{1,4}){1,2}|([0-9a-fA-F]{1,4}:){1,4}(:[0-9a-fA-F]{1,4}){1,3}|([0-9a-fA-F]{1,4}:){1,3}(:[0-9a-fA-F]{1,4}){1,4}|([0-9a-fA-F]{1,4}:){1,2}(:[0-9a-fA-F]{1,4}){1,5}|[0-9a-fA-F]{1,4}:((:[0-9a-fA-F]{1,4}){1,6})|:((:[0-9a-fA-F]{1,4}){1,7}|:))$/;

export function validateContract(value: unknown, schema: any): ValidationIssue | null {
  return validate(value, schema, []);
}

function validate(value: any, schema: any, path: readonly (string | number)[]): ValidationIssue | null {
  if (schema == null) return null;
  if (value === undefined) return issue(path, "missing value");
  if (schema.ref) {
    const definition = typeDefinitions[schema.ref];
    if (!definition) return issue(path, `unknown type ${schema.ref}`);
    if (!definitionApplies(definition, "decode")) {
      return issue(path, `type ${schema.ref} cannot be used in this direction`);
    }
    if (schema.variant && definition.kind === "enum") {
      const variant = definition.variants.find((entry: any) => entry.wire === schema.variant);
      if (!variant) return issue(path, `unknown enum variant ${schema.variant}`);
      return validateEnum(value, definition, variant, path);
    }
    if (definition.kind === "struct") return validateStruct(value, definition, path);
    if (definition.kind === "enum") return validateEnum(value, definition, undefined, path);
    if (definition.kind === "newtype") return validate(value, definition.inner, path);
    return issue(path, `unknown type kind ${String(definition.kind)}`);
  }
  if (schema.kind === "any" || schema.kind === "opaque") return null;
  if (schema.kind === "null") return value === null ? null : issue(path, "expected null");
  if (schema.kind === "nullable") return value === null ? null : validate(value, schema.inner, path);
  if (schema.kind === "bool") return typeof value === "boolean" ? null : issue(path, "expected boolean");
  if (schema.kind === "int") return validateInteger(value, schema, path);
  if (schema.kind === "float") return validateFloat(value, schema, path);
  if (schema.kind === "decimal") return validateDecimal(value, schema, path);
  if (schema.kind === "string") return validateString(value, schema, path);
  if (schema.kind === "bytes") return value instanceof Uint8Array ? null : issue(path, "expected bytes");
  if (schema.kind === "array") return validateArray(value, schema, path);
  if (schema.kind === "map") return validateMap(value, schema, path);
  if (schema.kind === "tuple") return validateTuple(value, schema, path);
  return issue(path, `unknown type kind ${String(schema.kind)}`);
}

function issue(path: readonly (string | number)[], message: string): ValidationIssue {
  return { path, message };
}

function validateInteger(value: any, schema: any, path: readonly (string | number)[]): ValidationIssue | null {
  const canonical = schema.signed ? CANONICAL_SIGNED_INTEGER : CANONICAL_INTEGER;
  let integer: bigint;
  if (schema.wire === "string" || schema.carrier === "string") {
    if (typeof value !== "string" || !canonical.test(value)) return issue(path, "expected integer string");
    integer = BigInt(value);
  } else if (typeof value === "bigint") {
    integer = value;
  } else if (typeof value === "number" && Number.isInteger(value)) {
    integer = BigInt(value);
  } else {
    return issue(path, "expected an exact integer");
  }
  const bits = BigInt(schema.bits);
  const magnitude = 1n << (schema.signed ? bits - 1n : bits);
  const minimum = schema.signed ? -magnitude : 0n;
  const maximum = magnitude - 1n;
  if (integer < minimum || integer > maximum) return issue(path, "integer is outside its native range");
  if (schema.minimum !== undefined) {
    const bound = integerConstraint(schema.minimum);
    if (bound === undefined) return issue(path, "invalid integer constraint");
    if (integer < bound) return issue(path, "integer is below its minimum");
  }
  if (schema.maximum !== undefined) {
    const bound = integerConstraint(schema.maximum);
    if (bound === undefined) return issue(path, "invalid integer constraint");
    if (integer > bound) return issue(path, "integer is above its maximum");
  }
  if (schema.multiple_of !== undefined) {
    const divisor = integerConstraint(schema.multiple_of);
    if (divisor === undefined || divisor <= 0n) return issue(path, "invalid integer constraint");
    if (integer % divisor !== 0n) return issue(path, "integer does not match its multiple");
  }
  return null;
}

function integerConstraint(value: any): bigint | undefined {
  if (typeof value === "bigint") return value;
  if (typeof value === "string" && /^-?\d+$/.test(value)) return BigInt(value);
  if (typeof value === "number" && Number.isSafeInteger(value)) return BigInt(value);
  return undefined;
}

function validateFloat(value: any, schema: any, path: readonly (string | number)[]): ValidationIssue | null {
  if (typeof value !== "number" || !Number.isFinite(value)) return issue(path, "expected finite number");
  if (schema.bits === 32 && !Number.isFinite(Math.fround(value))) {
    return issue(path, "number is outside the f32 range");
  }
  return null;
}

function validateDecimal(value: any, schema: any, path: readonly (string | number)[]): ValidationIssue | null {
  if (schema.wire === "string") {
    return typeof value === "string" && DECIMAL.test(value) ? null : issue(path, "expected decimal");
  }
  return typeof value === "number" && Number.isFinite(value) ? null : issue(path, "expected decimal");
}

function validateString(value: any, schema: any, path: readonly (string | number)[]): ValidationIssue | null {
  if (schema.carrier === "date") {
    return value instanceof Date && Number.isFinite(value.getTime()) ? null : issue(path, "expected a Date");
  }
  if (typeof value !== "string") {
    return issue(path, "expected string");
  }
  if (schema.format !== undefined && !matchesFormat(schema.format, value)) {
    return issue(path, formatMessage(schema.format));
  }
  const length = [...value].length;
  if (schema.min_length !== undefined && length < schema.min_length) {
    return issue(path, "string is shorter than the contract");
  }
  if (schema.max_length !== undefined && length > schema.max_length) {
    return issue(path, "string is longer than the contract");
  }
  if (schema.pattern !== undefined && !matchesPattern(schema.pattern, value)) {
    return issue(path, "string does not match the contract");
  }
  return null;
}

function matchesFormat(format: string, value: string): boolean {
  if (format === "uuid") return UUID.test(value);
  if (format === "date-time") {
    const match = DATE_TIME.exec(value);
    return match !== null && isCalendarDay(match[1]!, match[2]!, match[3]!)
      && isClock(match[4]!, match[5]!, match[6]!) && isOffset(match[8], match[9])
      && !Number.isNaN(Date.parse(value));
  }
  if (format === "date") {
    const match = DATE.exec(value);
    return match !== null && isCalendarDay(match[1]!, match[2]!, match[3]!);
  }
  if (format === "time") {
    const match = TIME.exec(value);
    return match !== null && isClock(match[1]!, match[2]!, match[3]!) && isOffset(match[5], match[6]);
  }
  if (format === "duration") return DURATION.test(value);
  if (format === "uri") return isUri(value);
  if (format === "email") return EMAIL.test(value);
  if (format === "ipv4") return IPV4.test(value);
  if (format === "ipv6") return IPV6.test(value);
  return false;
}

function formatMessage(format: string): string {
  if (format === "uuid") return "expected a uuid";
  if (format === "date-time") return "expected a date-time";
  if (format === "date") return "expected a date";
  if (format === "time") return "expected a time";
  if (format === "duration") return "expected a duration";
  if (format === "uri") return "expected a uri";
  if (format === "email") return "expected an email";
  if (format === "ipv4") return "expected an ipv4 address";
  if (format === "ipv6") return "expected an ipv6 address";
  return "unknown string format";
}

function isCalendarDay(year: string, month: string, day: string): boolean {
  const value = Number(year);
  const monthIndex = Number(month);
  const dayOfMonth = Number(day);
  if (monthIndex < 1 || monthIndex > 12 || dayOfMonth < 1) return false;
  return dayOfMonth <= daysInMonth(value, monthIndex);
}

function daysInMonth(year: number, month: number): number {
  if (month === 2) {
    return year % 4 === 0 && (year % 100 !== 0 || year % 400 === 0) ? 29 : 28;
  }
  return [4, 6, 9, 11].includes(month) ? 30 : 31;
}

function isClock(hour: string, minute: string, second: string): boolean {
  return Number(hour) <= 23 && Number(minute) <= 59 && Number(second) <= 59;
}

function isOffset(hour: string | undefined, minute: string | undefined): boolean {
  return hour === undefined || (Number(hour) <= 23 && Number(minute) <= 59);
}

function isUri(value: string): boolean {
  if (/\s/.test(value)) return false;
  try {
    return new URL(value).protocol.length > 1;
  } catch {
    return false;
  }
}

function matchesPattern(pattern: string, value: string): boolean {
  try {
    return new RegExp(pattern).test(value);
  } catch {
    return false;
  }
}

function validateArray(value: any, schema: any, path: readonly (string | number)[]): ValidationIssue | null {
  if (!Array.isArray(value)) return issue(path, "expected array");
  for (let index = 0; index < value.length; index += 1) {
    const result = validate(value[index], schema.item, [...path, index]);
    if (result !== null) return result;
  }
  if (schema.unique === true && new Set(value.map(stableValue)).size !== value.length) {
    return issue(path, "array values must be unique");
  }
  return null;
}

function stableValue(value: any): string {
  if (typeof value === "bigint") return `bigint:${value}`;
  if (value instanceof Date) return `date:${value.getTime()}`;
  if (value instanceof Uint8Array) return `bytes:${Array.from(value).join(",")}`;
  if (Array.isArray(value)) return `[${value.map(stableValue).join(",")}]`;
  if (value !== null && typeof value === "object") {
    return `{${Object.keys(value).sort().map((key) => `${JSON.stringify(key)}:${stableValue(value[key])}`).join(",")}}`;
  }
  return JSON.stringify(value);
}

function isPlainRecord(value: any): boolean {
  if (value === null || typeof value !== "object" || Array.isArray(value)) return false;
  const prototype = Object.getPrototypeOf(value);
  return prototype === Object.prototype || prototype === null;
}

function validateMap(value: any, schema: any, path: readonly (string | number)[]): ValidationIssue | null {
  if (!isPlainRecord(value)) return issue(path, "expected object");
  for (const [key, item] of Object.entries(value)) {
    const keyIssue = validate(key, schema.key, path);
    if (keyIssue !== null) return keyIssue;
    const result = validate(item, schema.value, [...path, key]);
    if (result !== null) return result;
  }
  return null;
}

function validateTuple(value: any, schema: any, path: readonly (string | number)[]): ValidationIssue | null {
  if (!Array.isArray(value) || value.length !== schema.items.length) return issue(path, "expected tuple");
  for (let index = 0; index < schema.items.length; index += 1) {
    const result = validate(value[index], schema.items[index], [...path, index]);
    if (result !== null) return result;
  }
  return null;
}

function validateStruct(value: any, definition: any, path: readonly (string | number)[]): ValidationIssue | null {
  const flattened = definition.flattened ?? [];
  const opaqueFlattened = flattened.some((item: any) => flattenedOpaqueSchema(item) !== undefined);
  if (!isPlainRecord(value)) return issue(path, "expected object");
  let declared: any[];
  try {
    declared = collectFields(definition, "decode");
  } catch (cause) {
    return issue(path, cause instanceof Error ? cause.message : "the struct cannot be decoded");
  }
  const known = new Set(declared.map((field: any) => fieldOutKey(field, "decode")));
  const unknown = Object.keys(value).filter((key) => !known.has(key));
  const openness = definition.openness ?? "preserve_opaque";
  const policy = typeof openness === "string" ? openness : "preserve_typed";
  if (unknown.length > 0 && !opaqueFlattened) {
    if (policy === "reject") return issue([...path, unknown[0]!], "unknown property");
    if (policy === "preserve_typed") {
      const extra = opennessValue(openness.preserve_typed.value);
      for (const key of unknown) {
        const result = validate(value[key], extra, [...path, key]);
        if (result !== null) return result;
      }
    } else {
      const extra = collectMapValues(flattened)[0];
      if (extra !== undefined) {
        for (const key of unknown) {
          const result = validate(value[key], extra, [...path, key]);
          if (result !== null) return result;
        }
      }
    }
  }
  for (const field of declared) {
    const key = fieldOutKey(field, "decode");
    const present = Object.hasOwn(value, key) && value[key] !== undefined;
    if (!present) {
      if (field.presence === "required") return issue([...path, key], "missing required field");
      continue;
    }
    const result = validate(value[key], field.ty, [...path, key]);
    if (result !== null) return result;
  }
  return null;
}

function validateEnum(value: any, definition: any, expected: any, path: readonly (string | number)[]): ValidationIssue | null {
  const repr = definition.repr;
  const variantFor = (wire: any) => definition.variants.find((entry: any) => entry.wire === wire);
  const matches = (variant: any) => expected === undefined || variant.wire === expected.wire;
  if (repr.kind === "string") {
    const variant = typeof value === "string" ? variantFor(value) : undefined;
    if (!variant || variant.payload.kind !== "unit" || !matches(variant)) {
      return issue(path, `invalid enum variant ${String(value)}`);
    }
    return null;
  }
  if (repr.kind === "untagged") {
    for (const variant of expected === undefined ? definition.variants : [expected]) {
      const result = validateVariantPayload(value, variant, "untagged", definition.direction, path);
      if (result === null) return null;
    }
    return issue(path, "value does not match an enum variant");
  }
  if (repr.kind === "external") {
    if (typeof value === "string") {
      const variant = variantFor(value);
      if (!variant || variant.payload.kind !== "unit" || !matches(variant)) {
        return issue(path, `invalid enum variant ${value}`);
      }
      return null;
    }
    if (!isRecord(value)) return issue(path, "expected enum object");
    const keys = Object.keys(value);
    if (keys.length !== 1) return issue(path, "expected one enum variant");
    const wire = keys[0]!;
    const variant = variantFor(wire);
    if (!variant || !matches(variant)) return issue(path, `invalid enum variant ${wire}`);
    return validateVariantPayload(value[wire], variant, "external", definition.direction, [...path, wire]);
  }
  if (!isRecord(value)) return issue(path, "expected an enum object");
  const tag = Object.hasOwn(value, repr.tag) ? value[repr.tag] : undefined;
  const variant = variantFor(tag);
  if (!variant || !matches(variant)) return issue(path, `invalid enum variant ${String(tag)}`);
  if (variant.payload.kind === "unit") return null;
  if (repr.kind === "adjacent") {
    if (!Object.hasOwn(value, repr.content)) return issue([...path, repr.content], "missing enum payload");
    return validateVariantPayload(value[repr.content], variant, "adjacent", definition.direction, [...path, repr.content]);
  }
  if (variant.payload.kind === "tuple") return issue(path, "internal enum payload must be an object");
  const payload = Object.create(null);
  for (const [key, item] of Object.entries(value)) {
    if (key !== repr.tag) payload[key] = item;
  }
  return validateVariantPayload(payload, variant, "internal", definition.direction, path);
}

function validateVariantPayload(
  value: any,
  variant: any,
  representation: string,
  direction: string,
  path: readonly (string | number)[],
): ValidationIssue | null {
  const payload = variant.payload;
  if (payload.kind === "unit") {
    return representation === "untagged" && value !== null ? issue(path, "expected null enum payload") : null;
  }
  if (payload.kind === "newtype") return validate(value, payload.ty, path);
  if (payload.kind === "tuple") {
    if (!Array.isArray(value) || value.length !== payload.items.length) {
      return issue(path, "expected enum tuple");
    }
    for (let index = 0; index < payload.items.length; index += 1) {
      const result = validate(value[index], payload.items[index], [...path, index]);
      if (result !== null) return result;
    }
    return null;
  }
  return validateStruct(value, {
    kind: "struct",
    direction,
    fields: payload.fields,
    flattened: [],
    openness: "preserve_opaque",
  }, path);
}
