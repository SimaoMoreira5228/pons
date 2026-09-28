pub(super) fn render() -> &'static str {
    r#"function validateWire(value: any, schema: any, mode: "encode" | "decode"): any {
  if (schema == null) return;
  if (value === undefined) throw new WirePolicyError("missing value");
  if (schema.ref) {
    const definition = typeDefinitions[schema.ref];
    if (!definition) return;
    if (schema.variant && definition.kind === "enum") {
      const variant = definition.variants.find((entry: any) => entry.wire === schema.variant);
      if (!variant) throw new WirePolicyError(`unknown enum variant ${schema.variant}`);
      return validateEnum(value, definition, variant, mode);
    } else if (definition.kind === "struct") {
      return validateStruct(value, definition, mode);
    } else if (definition.kind === "enum") {
      return validateEnum(value, definition, undefined, mode);
    } else if (definition.kind === "newtype") {
      return validateWire(value, definition.inner, mode);
    }
    return;
  }
  if (schema.kind === "any") return;
  if (schema.kind === "null") {
    if (value !== null) throw new WirePolicyError("expected null");
    return;
  }
  if (schema.kind === "nullable") {
    if (value !== null) validateWire(value, schema.inner, mode);
    return;
  }
  if (schema.kind === "bool") {
    if (typeof value !== "boolean") throw new WirePolicyError("expected boolean");
    return;
  }
  if (schema.kind === "int") {
    validateInteger(value, schema);
    return;
  }
  if (schema.kind === "float") {
    if (typeof value !== "number" || !Number.isFinite(value)) {
      throw new WirePolicyError("expected finite number");
    }
    return;
  }
  if (schema.kind === "decimal") {
    if (schema.wire === "string" ? typeof value !== "string" : typeof value !== "number") {
      throw new WirePolicyError("expected decimal");
    }
    return;
  }
  if (schema.kind === "string") {
    if (typeof value !== "string") throw new WirePolicyError("expected string");
    if (schema.min_length !== undefined && value.length < schema.min_length) {
      throw new WirePolicyError("string is shorter than the contract");
    }
    if (schema.max_length !== undefined && value.length > schema.max_length) {
      throw new WirePolicyError("string is longer than the contract");
    }
    return;
  }
  if (schema.kind === "bytes") {
    if (mode === "encode") {
      if (!(value instanceof Uint8Array || Array.isArray(value))) {
        throw new WirePolicyError("expected byte array");
      }
    } else if (schema.encoding === "array") {
      if (!Array.isArray(value)) throw new WirePolicyError("expected byte array");
    } else if (typeof value !== "string") {
      throw new WirePolicyError("expected encoded bytes");
    }
    return;
  }
  if (schema.kind === "array") {
    if (!Array.isArray(value)) throw new WirePolicyError("expected array");
    for (const item of value) validateWire(item, schema.item, mode);
    return;
  }
  if (schema.kind === "map") {
    if (!isRecord(value)) throw new WirePolicyError("expected object");
    for (const item of Object.values(value)) validateWire(item, schema.value, mode);
    return;
  }
  if (schema.kind === "tuple") {
    if (!Array.isArray(value) || value.length !== schema.items.length) {
      throw new WirePolicyError("expected tuple");
    }
    schema.items.forEach((item: any, index: number) => validateWire(value[index], item, mode));
    return;
  }
}

function validateInteger(value: any, schema: any): void {
  let integer: bigint;
  if (schema.wire === "string") {
    if (typeof value !== "string" || !/^-?\d+$/.test(value)) {
      throw new WirePolicyError("expected integer string");
    }
    integer = BigInt(value);
  } else if (typeof value === "bigint") {
    integer = value;
  } else if (typeof value === "number" && Number.isSafeInteger(value)) {
    integer = BigInt(value);
  } else {
    throw new WirePolicyError("expected an exact integer");
  }
  const bits = BigInt(schema.bits);
  const magnitude = 1n << (schema.signed ? bits - 1n : bits);
  const minimum = schema.signed ? -magnitude : 0n;
  const maximum = magnitude - 1n;
  if (integer < minimum || integer > maximum) {
    throw new WirePolicyError("integer is outside its native range");
  }
  if (schema.minimum !== undefined && integer < integerConstraint(schema.minimum)) {
    throw new WirePolicyError("integer is below its minimum");
  }
  if (schema.maximum !== undefined && integer > integerConstraint(schema.maximum)) {
    throw new WirePolicyError("integer is above its maximum");
  }
  if (schema.multiple_of !== undefined) {
    const divisor = integerConstraint(schema.multiple_of);
    if (divisor <= 0n || integer % divisor !== 0n) {
      throw new WirePolicyError("integer does not match its multiple");
    }
  }
}

function integerConstraint(value: any): bigint {
  if (typeof value === "bigint") return value;
  if (typeof value === "string" && /^-?\d+$/.test(value)) return BigInt(value);
  if (typeof value === "number" && Number.isSafeInteger(value)) return BigInt(value);
  throw new WirePolicyError("invalid integer constraint");
}

function validateStruct(value: any, definition: any, mode: "encode" | "decode"): void {
  if (!isRecord(value)) throw new WirePolicyError("expected object");
  for (const field of collectFields(definition)) {
    const present = Object.hasOwn(value, field.wire) && value[field.wire] !== undefined;
    if (!present) {
      if (field.presence === "required") {
        throw new WirePolicyError(`missing required field ${field.wire}`);
      }
      continue;
    }
    validateWire(value[field.wire], field.ty, mode);
  }
}

function validateEnum(value: any, definition: any, expected: any, mode: "encode" | "decode"): any {
  const repr = definition.repr;
  const variantFor = (wire: any) => definition.variants.find((entry: any) => entry.wire === wire);
  const matchesExpected = (variant: any) => expected === undefined || variant.wire === expected.wire;
  if (repr.kind === "string") {
    const variant = typeof value === "string" ? variantFor(value) : undefined;
    if (!variant || variant.payload.kind !== "unit" || !matchesExpected(variant)) {
      throw new WirePolicyError(`invalid enum variant ${String(value)}`);
    }
    return variant;
  }
  if (repr.kind === "untagged") {
    return selectUntaggedVariant(value, definition, expected, mode);
  }
  if (repr.kind === "external") {
    if (typeof value === "string") {
      const variant = variantFor(value);
      if (!variant || variant.payload.kind !== "unit" || !matchesExpected(variant)) {
        throw new WirePolicyError(`invalid enum variant ${value}`);
      }
      return variant;
    }
    if (!isRecord(value)) throw new WirePolicyError("expected enum object");
    const keys = Object.keys(value);
    if (keys.length !== 1) throw new WirePolicyError("expected one enum variant");
    const variant = variantFor(keys[0]);
    if (!variant || !matchesExpected(variant)) {
      throw new WirePolicyError(`invalid enum variant ${keys[0]}`);
    }
    validateVariantPayload(value[keys[0]], variant, mode, "external");
    return variant;
  }
  if (!isRecord(value)) throw new WirePolicyError("expected enum object");
  const variant = variantFor(value[repr.tag]);
  if (!variant || !matchesExpected(variant)) {
    throw new WirePolicyError(`invalid enum variant ${String(value[repr.tag])}`);
  }
  if (variant.payload.kind !== "unit") {
    const payload = repr.kind === "adjacent" ? value[repr.content] : value;
    validateVariantPayload(payload, variant, mode, repr.kind);
  }
  return variant;
}

function selectUntaggedVariant(value: any, definition: any, expected: any, mode: "encode" | "decode"): any {
  const variants = expected === undefined ? definition.variants : [expected];
  for (const variant of variants) {
    try {
      validateVariantPayload(value, variant, mode, "untagged");
      return variant;
    } catch {
    }
  }
  throw new WirePolicyError("value does not match an enum variant");
}

function validateVariantPayload(value: any, variant: any, mode: "encode" | "decode", representation: string): void {
  const payload = variant.payload;
  if (payload.kind === "unit") {
    if (representation === "untagged" && value !== null) throw new WirePolicyError("expected null enum payload");
    return;
  }
  if (payload.kind === "newtype") {
    validateWire(value, payload.ty, mode);
    return;
  }
  if (payload.kind === "tuple") {
    if (!Array.isArray(value) || value.length !== payload.items.length) {
      throw new WirePolicyError("expected enum tuple");
    }
    payload.items.forEach((item: any, index: number) => validateWire(value[index], item, mode));
    return;
  }
  validateStruct(value, { kind: "struct", fields: payload.fields, flattened: [], openness: "preserve_opaque" }, mode);
}

function isRecord(value: any): boolean {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}
"#
}
