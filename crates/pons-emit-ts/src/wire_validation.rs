pub(super) fn render() -> &'static str {
    r#"function validateWire(value: any, schema: any, mode: "encode" | "decode", allowUnsafeWideIntegers = false): any {
  if (schema == null) return;
  if (value === undefined) throw new WirePolicyError("missing value");
  if (schema.ref) {
     const definition = typeDefinitions[schema.ref];
     if (!definition) return;
     if ((mode === "encode" && definition.direction === "output") || (mode === "decode" && definition.direction === "input")) {
       throw new WirePolicyError(`type ${schema.ref} cannot be used in this direction`);
     }
     if (schema.variant && definition.kind === "enum") {
      const variant = definition.variants.find((entry: any) => entry.wire === schema.variant);
      if (!variant) throw new WirePolicyError(`unknown enum variant ${schema.variant}`);
       return validateEnum(value, definition, variant, mode, allowUnsafeWideIntegers);
    } else if (definition.kind === "struct") {
       return validateStruct(value, definition, mode, allowUnsafeWideIntegers);
    } else if (definition.kind === "enum") {
       return validateEnum(value, definition, undefined, mode, allowUnsafeWideIntegers);
    } else if (definition.kind === "newtype") {
       return validateWire(value, definition.inner, mode, allowUnsafeWideIntegers);
    }
    return;
  }
  if (schema.kind === "any" || schema.kind === "opaque") return;
  if (schema.kind === "null") {
    if (value !== null) throw new WirePolicyError("expected null");
    return;
  }
  if (schema.kind === "nullable") {
     if (value !== null) validateWire(value, schema.inner, mode, allowUnsafeWideIntegers);
    return;
  }
  if (schema.kind === "bool") {
    if (typeof value !== "boolean") throw new WirePolicyError("expected boolean");
    return;
  }
  if (schema.kind === "int") {
    validateInteger(value, schema, mode, allowUnsafeWideIntegers);
    return;
  }
  if (schema.kind === "float") {
    const numeric = typeof value === "bigint" && mode === "decode" ? Number(value) : value;
    if (typeof numeric !== "number" || !Number.isFinite(numeric)) {
      throw new WirePolicyError("expected finite number");
    }
    if (schema.bits === 32 && !Number.isFinite(Math.fround(numeric))) {
      throw new WirePolicyError("number is outside the f32 range");
    }
    return;
  }
  if (schema.kind === "decimal") {
    if (schema.wire === "string" && (typeof value !== "string" || !/^-?(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][+-]?\d+)?$/.test(value))) {
      throw new WirePolicyError("expected decimal");
    }
    if (schema.wire === "number") {
      const numeric = typeof value === "bigint" && mode === "decode" ? Number(value) : value;
      if (typeof numeric !== "number" || !Number.isFinite(numeric)) {
        throw new WirePolicyError("expected decimal");
      }
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
    if (schema.pattern !== undefined && !new RegExp(schema.pattern).test(value)) {
      throw new WirePolicyError("string does not match the contract");
    }
    return;
  }
  if (schema.kind === "bytes") {
    if (mode === "encode") {
      if (!(value instanceof Uint8Array || Array.isArray(value))) {
        throw new WirePolicyError("expected byte array");
      }
      if (Array.isArray(value) && value.some((item) => !Number.isInteger(item) || item < 0 || item > 255)) {
        throw new WirePolicyError("byte values must be between 0 and 255");
      }
    } else if (schema.encoding === "array") {
      if (!Array.isArray(value) || value.some((item) => !Number.isInteger(item) || item < 0 || item > 255)) {
        throw new WirePolicyError("expected byte array");
      }
    } else if (typeof value !== "string") {
      throw new WirePolicyError("expected encoded bytes");
    } else if (schema.encoding === "hex" && !/^(?:[0-9a-fA-F]{2})*$/.test(value)) {
      throw new WirePolicyError("invalid hexadecimal bytes");
    } else if (schema.encoding === "base64" && !/^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/.test(value)) {
      throw new WirePolicyError("invalid base64 bytes");
    } else if (schema.encoding === "base64url" && (!/^[A-Za-z0-9_-]*$/.test(value) || value.length % 4 === 1)) {
      throw new WirePolicyError("invalid base64url bytes");
    }
    return;
  }
  if (schema.kind === "array") {
    if (!Array.isArray(value)) throw new WirePolicyError("expected array");
     for (const item of value) validateWire(item, schema.item, mode, allowUnsafeWideIntegers);
    if (schema.unique === true) {
      const values = new Set(value.map(stableValue));
      if (values.size !== value.length) throw new WirePolicyError("array values must be unique");
    }
    return;
  }
  if (schema.kind === "map") {
    if (!isRecord(value)) throw new WirePolicyError("expected object");
    for (const [key, item] of Object.entries(value)) {
       validateWire(key, schema.key, mode, allowUnsafeWideIntegers);
       validateWire(item, schema.value, mode, allowUnsafeWideIntegers);
    }
    return;
  }
  if (schema.kind === "tuple") {
    if (!Array.isArray(value) || value.length !== schema.items.length) {
      throw new WirePolicyError("expected tuple");
    }
     schema.items.forEach((item: any, index: number) => validateWire(value[index], item, mode, allowUnsafeWideIntegers));
    return;
  }
}

function stableValue(value: any): string {
  if (typeof value === "bigint") return `bigint:${value}`;
  if (Array.isArray(value)) return `[${value.map(stableValue).join(",")}]`;
  if (value !== null && typeof value === "object") {
    return `{${Object.keys(value).sort().map((key) => `${JSON.stringify(key)}:${stableValue(value[key])}`).join(",")}}`;
  }
  return JSON.stringify(value);
}

function validateInteger(value: any, schema: any, mode: "encode" | "decode", allowUnsafeWideIntegers: boolean): void {
  let integer: bigint;
  if (schema.wire === "string") {
    if (typeof value !== "string" || !/^-?\d+$/.test(value)) {
      throw new WirePolicyError("expected integer string");
    }
    integer = BigInt(value);
  } else if (typeof value === "bigint") {
    integer = value;
  } else if (
    typeof value === "number" && (
      Number.isSafeInteger(value) || (
        allowUnsafeWideIntegers && schema.wire === "number" && schema.bits > 53 && Number.isInteger(value)
      )
    )
  ) {
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

function validateStruct(value: any, definition: any, mode: "encode" | "decode", allowUnsafeWideIntegers: boolean): void {
  if (!isRecord(value)) throw new WirePolicyError("expected object");
  const flattenedOpaque = (definition.flattened ?? []).some((item: any) => isFlattenedOpaque(item));
  for (const field of collectFields(definition, mode)) {
    if (flattenedOpaque && !hasOpaque(field.ty)) continue;
    const present = Object.hasOwn(value, field.wire) && value[field.wire] !== undefined;
    if (!present) {
      if (field.presence === "required") {
        throw new WirePolicyError(`missing required field ${field.wire}`);
      }
      continue;
    }
    validateWire(value[field.wire], field.ty, mode, allowUnsafeWideIntegers);
  }
}

function validateEnum(value: any, definition: any, expected: any, mode: "encode" | "decode", allowUnsafeWideIntegers: boolean): any {
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
    return selectUntaggedVariant(value, definition, expected, mode, allowUnsafeWideIntegers);
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
     validateVariantPayload(value[keys[0]], variant, mode, "external", definition.direction, allowUnsafeWideIntegers);
    return variant;
  }
  if (!isRecord(value)) throw new WirePolicyError("expected enum object");
   const tag = Object.hasOwn(value, repr.tag) ? value[repr.tag] : undefined;
   const variant = variantFor(tag);
   if (!variant || !matchesExpected(variant)) {
     throw new WirePolicyError(`invalid enum variant ${String(tag)}`);
   }
   if (repr.kind === "internal" && variant.payload.kind === "tuple") {
     throw new WirePolicyError("internal enum payload must be an object");
   }
    if (repr.kind === "internal" && variant.payload.kind === "newtype") {
      let payloadType = variant.payload.ty.ref ? typeDefinitions[variant.payload.ty.ref] : undefined;
      while (payloadType?.kind === "newtype") {
        payloadType = payloadType.inner.ref ? typeDefinitions[payloadType.inner.ref] : undefined;
      }
      if (payloadType?.kind !== "struct") throw new WirePolicyError("internal enum payload must be an object");
    }
   if (variant.payload.kind !== "unit") {
     const payload = repr.kind === "adjacent"
       ? (Object.hasOwn(value, repr.content) ? value[repr.content] : undefined)
       : value;
     validateVariantPayload(payload, variant, mode, repr.kind, definition.direction, allowUnsafeWideIntegers);
  }
  return variant;
}

function selectUntaggedVariant(value: any, definition: any, expected: any, mode: "encode" | "decode", allowUnsafeWideIntegers: boolean): any {
  const variants = expected === undefined ? definition.variants : [expected];
  for (const variant of variants) {
    try {
       validateVariantPayload(value, variant, mode, "untagged", definition.direction, allowUnsafeWideIntegers);
      return variant;
    } catch {
    }
  }
  throw new WirePolicyError("value does not match an enum variant");
}

function validateVariantPayload(value: any, variant: any, mode: "encode" | "decode", representation: string, direction: string, allowUnsafeWideIntegers: boolean): void {
  const payload = variant.payload;
  if (payload.kind === "unit") {
    if (representation === "untagged" && value !== null) throw new WirePolicyError("expected null enum payload");
    return;
  }
  if (payload.kind === "newtype") {
     validateWire(value, payload.ty, mode, allowUnsafeWideIntegers);
    return;
  }
  if (payload.kind === "tuple") {
    if (!Array.isArray(value) || value.length !== payload.items.length) {
      throw new WirePolicyError("expected enum tuple");
    }
     payload.items.forEach((item: any, index: number) => validateWire(value[index], item, mode, allowUnsafeWideIntegers));
    return;
  }
   validateStruct(value, { kind: "struct", direction, fields: payload.fields, flattened: [], openness: "preserve_opaque" }, mode, allowUnsafeWideIntegers);
}

function isRecord(value: any): boolean {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}
"#
}
