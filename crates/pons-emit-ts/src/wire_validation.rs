pub(super) fn render() -> &'static str {
    r#"function validateWire(value: any, schema: any): void {
  if (schema == null) return;
  if (value === undefined) throw new WirePolicyError("missing value");
  if (schema.ref) {
    const definition = typeDefinitions[schema.ref];
    if (!definition) return;
    if (schema.variant && definition.kind === "enum") {
      const variant = definition.variants.find((entry: any) => entry.wire === schema.variant);
      if (!variant) throw new WirePolicyError(`unknown enum variant ${schema.variant}`);
      validateEnum(value, definition, variant);
    } else if (definition.kind === "struct") {
      validateStruct(value, definition);
    } else if (definition.kind === "enum") {
      validateEnum(value, definition);
    } else if (definition.kind === "newtype") {
      validateWire(value, definition.inner);
    }
    return;
  }
  if (schema.kind === "any") return;
  if (schema.kind === "null") {
    if (value !== null) throw new WirePolicyError("expected null");
    return;
  }
  if (schema.kind === "nullable") {
    if (value !== null) validateWire(value, schema.inner);
    return;
  }
  if (schema.kind === "bool") {
    if (typeof value !== "boolean") throw new WirePolicyError("expected boolean");
    return;
  }
  if (schema.kind === "int") {
    if (schema.wire === "string") {
      if (typeof value !== "string" || !/^-?\d+$/.test(value)) {
        throw new WirePolicyError("expected integer string");
      }
    } else if (typeof value !== "number" && typeof value !== "bigint") {
      throw new WirePolicyError("expected integer");
    } else if (typeof value === "number" && !Number.isInteger(value)) {
      throw new WirePolicyError("expected integer");
    }
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
    if (schema.encoding === "array") {
      if (!(value instanceof Uint8Array || Array.isArray(value))) {
        throw new WirePolicyError("expected byte array");
      }
    } else if (typeof value !== "string") {
      throw new WirePolicyError("expected encoded bytes");
    }
    return;
  }
  if (schema.kind === "array") {
    if (!Array.isArray(value)) throw new WirePolicyError("expected array");
    for (const item of value) validateWire(item, schema.item);
    return;
  }
  if (schema.kind === "map") {
    if (!isRecord(value)) throw new WirePolicyError("expected object");
    for (const item of Object.values(value)) validateWire(item, schema.value);
    return;
  }
  if (schema.kind === "tuple") {
    if (!Array.isArray(value) || value.length !== schema.items.length) {
      throw new WirePolicyError("expected tuple");
    }
    schema.items.forEach((item: any, index: number) => validateWire(value[index], item));
    return;
  }
}

function validateStruct(value: any, definition: any): void {
  if (!isRecord(value)) throw new WirePolicyError("expected object");
  for (const field of collectFields(definition)) {
    const present = Object.hasOwn(value, field.wire) && value[field.wire] !== undefined;
    if (!present) {
      if (field.presence === "required") {
        throw new WirePolicyError(`missing required field ${field.wire}`);
      }
      continue;
    }
    validateWire(value[field.wire], field.ty);
  }
}

function validateEnum(value: any, definition: any, expected?: any): void {
  const repr = definition.repr;
  const variantFor = (wire: any) => definition.variants.find((entry: any) => entry.wire === wire);
  const matchesExpected = (variant: any) => expected === undefined || variant.wire === expected.wire;
  if (repr.kind === "string") {
    const variant = typeof value === "string" ? variantFor(value) : undefined;
    if (!variant || variant.payload.kind !== "unit" || !matchesExpected(variant)) {
      throw new WirePolicyError(`invalid enum variant ${String(value)}`);
    }
    return;
  }
  if (repr.kind === "untagged") {
    const variants = expected === undefined ? definition.variants : [expected];
    for (const variant of variants) {
      try {
        validateVariantPayload(value, variant, "untagged");
        return;
      } catch {
      }
    }
    throw new WirePolicyError("value does not match an enum variant");
  }
  if (repr.kind === "external") {
    if (typeof value === "string") {
      const variant = variantFor(value);
      if (!variant || variant.payload.kind !== "unit" || !matchesExpected(variant)) {
        throw new WirePolicyError(`invalid enum variant ${value}`);
      }
      return;
    }
    if (!isRecord(value)) throw new WirePolicyError("expected enum object");
    const keys = Object.keys(value);
    if (keys.length !== 1) throw new WirePolicyError("expected one enum variant");
    const variant = variantFor(keys[0]);
    if (!variant || !matchesExpected(variant)) {
      throw new WirePolicyError(`invalid enum variant ${keys[0]}`);
    }
    validateVariantPayload(value[keys[0]], variant, "external");
    return;
  }
  if (!isRecord(value)) throw new WirePolicyError("expected enum object");
  const variant = variantFor(value[repr.tag]);
  if (!variant || !matchesExpected(variant)) {
    throw new WirePolicyError(`invalid enum variant ${String(value[repr.tag])}`);
  }
  if (variant.payload.kind !== "unit") {
    const payload = repr.kind === "adjacent" ? value[repr.content] : value;
    validateVariantPayload(payload, variant, repr.kind);
  }
}

function validateVariantPayload(value: any, variant: any, mode: string): void {
  const payload = variant.payload;
  if (payload.kind === "unit") {
    if (mode === "untagged" && value !== null) throw new WirePolicyError("expected null enum payload");
    return;
  }
  if (payload.kind === "newtype") {
    validateWire(value, payload.ty);
    return;
  }
  if (payload.kind === "tuple") {
    if (!Array.isArray(value) || value.length !== payload.items.length) {
      throw new WirePolicyError("expected enum tuple");
    }
    payload.items.forEach((item: any, index: number) => validateWire(value[index], item));
    return;
  }
  validateStruct(value, { kind: "struct", fields: payload.fields, flattened: [], openness: "preserve_opaque" });
}

function isRecord(value: any): boolean {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}
"#
}
