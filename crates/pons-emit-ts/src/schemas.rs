use std::collections::{BTreeMap, BTreeSet};

use pons_ir::{
    Access, DecimalWire, Direction, EnumRepr, Field, FieldFormat, InlineType, IntegerWire,
    ObjectOpenness, Presence, TypeKind, TypeRef, VariantPayload,
};

use crate::{
    DateMode, EmitError, EmitMode, FieldCase, Int64, TsOptions, camel, jsdoc, pascal, string,
    unique_names,
};

const PRELUDE: &[(&str, &str)] = &[
    (
        "canonical-integer",
        "const CANONICAL_INTEGER = /^(0|[1-9][0-9]*)$/;\n",
    ),
    (
        "canonical-signed-integer",
        "const CANONICAL_SIGNED_INTEGER = /^-?(0|[1-9][0-9]*)$/;\n",
    ),
    (
        "decimal",
        "const DECIMAL = /^-?(?:\\d+(?:\\.\\d*)?|\\.\\d+)(?:[eE][+-]?\\d+)?$/;\n",
    ),
    (
        "date-time",
        "const DATE_TIME = /^\\d{4}-\\d{2}-\\d{2}[Tt]\\d{2}:\\d{2}:\\d{2}(?:\\.\\d{1,3})?(?:[Zz]|[+-]\\d{2}:\\d{2})$/;\n",
    ),
    (
        "instant",
        "const instant = () => z.string().regex(DATE_TIME, \"the date-time is not a full-date and a full-time\").refine((value) => !Number.isNaN(Date.parse(value)), \"date-time is not an instant a Date can hold\");\n",
    ),
    (
        "time",
        "const TIME = /^\\d{2}:\\d{2}:\\d{2}(?:\\.\\d+)?(?:[Zz]|[+-]\\d{2}:\\d{2})$/;\n",
    ),
    (
        "clock",
        "const clock = () => z.string().regex(TIME, \"the time is not a full-time\");\n",
    ),
    (
        "binary32",
        "const binary32 = () => z.number().refine((value) => Number.isFinite(Math.fround(value)), \"number is outside the f32 range\");\n",
    ),
    (
        "integer",
        "function exactInteger(bits: number, signed: boolean, minimum?: string, maximum?: string, multipleOf?: string) {\n  const magnitude = 1n << BigInt(signed ? bits - 1 : bits);\n  const low = signed ? -magnitude : 0n;\n  const high = magnitude - 1n;\n  return (value: unknown): string | undefined => {\n    let integer: bigint;\n    try {\n      integer = BigInt(value as bigint | number | string);\n    } catch {\n      return \"expected an exact integer\";\n    }\n    if (integer < low || integer > high) return \"integer is outside its native range\";\n    if (minimum !== undefined && integer < BigInt(minimum)) return \"integer is below its minimum\";\n    if (maximum !== undefined && integer > BigInt(maximum)) return \"integer is above its maximum\";\n    if (multipleOf !== undefined && (BigInt(multipleOf) <= 0n || integer % BigInt(multipleOf) !== 0n)) return \"integer does not match its multiple\";\n    return undefined;\n  };\n}\nfunction integer<T extends z.ZodType>(base: T, bits: number, signed: boolean, minimum?: string, maximum?: string, multipleOf?: string): T {\n  const fault = exactInteger(bits, signed, minimum, maximum, multipleOf);\n  return base.superRefine((value, ctx) => {\n    const reason = fault(value);\n    if (reason !== undefined) ctx.addIssue({ code: \"custom\", message: reason });\n  });\n}\n",
    ),
    (
        "required",
        "const required = <T extends z.ZodType>(schema: T): T => schema.refine((value) => value !== undefined, \"missing required value\");\n",
    ),
    (
        "one-key",
        "const oneKey = (key: string, payload: z.ZodType) => z.object({ [key]: required(payload) }).strict();\n",
    ),
    (
        "unique",
        "function stableKey(value: unknown): string {\n  if (typeof value === \"bigint\") return `bigint:${value}`;\n  if (Array.isArray(value)) return `[${value.map(stableKey).join(\",\")}]`;\n  if (value !== null && typeof value === \"object\") {\n    const record = value as Record<string, unknown>;\n    return `{${Object.keys(record).sort().map((key) => `${JSON.stringify(key)}:${stableKey(record[key])}`).join(\",\")}}`;\n  }\n  return JSON.stringify(value);\n}\nconst uniqueItems = <T extends z.ZodType>(item: T) => z.array(item).refine((values) => new Set(values.map(stableKey)).size === values.length, \"array values must be unique\");\n",
    ),
];

const MISMATCH: &str = "export function mismatch(schema: z.ZodType): (value: unknown) => string | undefined {\n  return (value) => {\n    const result = schema.safeParse(value);\n    if (result.success) return undefined;\n    const issue = result.error.issues[0];\n    if (issue === undefined) return \"the value does not match the contract\";\n    const path = issue.path.map((step) => String(step)).join(\".\");\n    return path === \"\" ? issue.message : `${path}: ${issue.message}`;\n  };\n}\n";

struct Schemas<'a> {
    document: &'a pons_ir::Document,
    names: &'a BTreeMap<String, String>,
    options: &'a TsOptions,
    declarations: BTreeMap<(String, Option<String>), String>,
    need: BTreeSet<&'static str>,
    used: BTreeSet<String>,
    imports: BTreeSet<String>,
    bare: bool,
}

pub(crate) struct Generator<'a>(Schemas<'a>);

impl<'a> Generator<'a> {
    pub(crate) fn new(
        document: &'a pons_ir::Document,
        names: &'a BTreeMap<String, String>,
        options: &'a TsOptions,
    ) -> Self {
        Generator(Schemas {
            document,
            names,
            options,
            declarations: declaration_names(document, names),
            need: BTreeSet::new(),
            used: BTreeSet::new(),
            imports: BTreeSet::new(),
            bare: false,
        })
    }

    pub(crate) fn validate(&mut self, reference: &TypeRef) -> Result<String, EmitError> {
        self.0.used.clear();
        self.0.bare = true;
        let expression = self.0.reference(reference);
        self.0.bare = false;
        self.0.imports.extend(std::mem::take(&mut self.0.used));
        expression
    }

    pub(crate) fn take_imports(&mut self) -> BTreeSet<String> {
        std::mem::take(&mut self.0.imports)
    }

    pub(crate) fn render(&mut self) -> Result<String, EmitError> {
        let mut body = String::new();
        for (id, definition) in &self.0.document.types {
            let comment = jsdoc(definition.doc.as_ref());
            match &definition.definition {
                TypeKind::Enum { variants, .. } => {
                    for variant in variants {
                        let wire = variant.wire.clone();
                        let declaration = self.0.declaration(id, Some(&wire))?;
                        let doc = jsdoc(variant.doc.as_ref());
                        if !doc.is_empty() {
                            body.push_str(&doc);
                            body.push('\n');
                        }
                        body.push_str(&format!(
                            "export const {declaration}Schema = {};\n",
                            self.0.variant(id, definition, variant)?
                        ));
                    }
                    if !comment.is_empty() {
                        body.push_str(&comment);
                        body.push('\n');
                    }
                    let members = variants
                        .iter()
                        .map(|variant| {
                            self.0
                                .declaration(id, Some(&variant.wire))
                                .map(|declaration| format!("{declaration}Schema"))
                        })
                        .collect::<Result<Vec<_>, _>>()?
                        .join(", ");
                    body.push_str(&format!(
                        "export const {}Schema = z.union([{members}]);\n\n",
                        self.0.declaration(id, None)?
                    ));
                }
                _ => {
                    if !comment.is_empty() {
                        body.push_str(&comment);
                        body.push('\n');
                    }
                    body.push_str(&format!(
                        "export const {}Schema = {};\n\n",
                        self.0.declaration(id, None)?,
                        self.0.definition(id, definition)?
                    ))
                }
            }
        }
        body.push_str(MISMATCH);
        let mut output = String::from("import { z } from \"zod\";\n\n");
        for (id, code) in PRELUDE {
            if self.0.need.contains(id) {
                output.push_str(code);
                output.push('\n');
            }
        }
        output.push_str(&body);
        Ok(output)
    }
}

pub(crate) fn declaration_names(
    document: &pons_ir::Document,
    names: &BTreeMap<String, String>,
) -> BTreeMap<(String, Option<String>), String> {
    let mut keys = Vec::new();
    let mut bases = Vec::new();
    for (id, definition) in &document.types {
        let name = &names[id];
        if let TypeKind::Enum { variants, .. } = &definition.definition {
            for variant in variants {
                keys.push((id.clone(), Some(variant.wire.clone())));
                bases.push(format!("{name}{}", pascal(&variant.ident)));
            }
        }
        keys.push((id.clone(), None));
        bases.push(name.clone());
    }
    keys.into_iter().zip(unique_names(bases, &[])).collect()
}

impl<'a> Schemas<'a> {
    fn declaration(&self, id: &str, variant: Option<&str>) -> Result<String, EmitError> {
        let name = &self.names[id];
        match variant {
            None => Ok(name.clone()),
            Some(wire) => {
                let key = (id.to_owned(), Some(wire.to_owned()));
                self.declarations.get(&key).cloned().ok_or_else(|| {
                    EmitError(format!("unknown enum variant {wire} of the type {id}"))
                })
            }
        }
    }

    fn lazy(&mut self, id: &str, variant: Option<&str>) -> Result<String, EmitError> {
        let name = format!("{}Schema", self.declaration(id, variant)?);
        self.used.insert(name.clone());
        Ok(if self.bare {
            name
        } else {
            format!("z.lazy(() => {name})")
        })
    }

    fn reference(&mut self, reference: &TypeRef) -> Result<String, EmitError> {
        if let TypeRef::Named(named) = reference {
            return self.lazy(&named.id, named.variant.as_deref());
        }
        self.inline(reference)
    }

    fn inline(&mut self, reference: &TypeRef) -> Result<String, EmitError> {
        let TypeRef::Inline(inline) = reference else {
            unreachable!("a reference that is not inline is named")
        };
        Ok(match inline {
            InlineType::Bool => "z.boolean()".into(),
            InlineType::Null => "z.null()".into(),
            InlineType::Any => "z.any()".into(),
            InlineType::Opaque { .. } => "z.unknown()".into(),
            InlineType::Nullable { inner } => {
                let inner = self.reference(inner)?;
                format!("{inner}.nullable()")
            }
            InlineType::Int {
                bits,
                signed,
                wire,
                minimum,
                maximum,
                multiple_of,
            } => {
                self.need.insert("integer");
                let base = match self.int_carrier(*wire, *bits) {
                    Carrier::Text => {
                        self.need.insert(if *signed {
                            "canonical-signed-integer"
                        } else {
                            "canonical-integer"
                        });
                        format!(
                            "z.string().regex({})",
                            if *signed {
                                "CANONICAL_SIGNED_INTEGER"
                            } else {
                                "CANONICAL_INTEGER"
                            }
                        )
                    }
                    Carrier::Bigint => "z.bigint()".into(),
                    Carrier::Number if *bits <= 53 => "z.int()".into(),
                    Carrier::Number => "z.number().int()".into(),
                };
                let mut arguments = vec![base, bits.to_string(), signed.to_string()];
                for value in [minimum, maximum, multiple_of] {
                    arguments.push(match value {
                        Some(value) => string(&value.to_string()),
                        None => String::from("undefined"),
                    });
                }
                while arguments.last().map(String::as_str) == Some("undefined") {
                    arguments.pop();
                }
                format!("integer({})", arguments.join(", "))
            }
            InlineType::Float { bits } => {
                if *bits == 32 {
                    self.need.insert("binary32");
                    "binary32()".into()
                } else {
                    "z.number()".into()
                }
            }
            InlineType::Decimal { wire } => match wire {
                DecimalWire::Number => "z.number()".into(),
                DecimalWire::String => {
                    self.need.insert("decimal");
                    "z.string().regex(DECIMAL)".into()
                }
            },
            InlineType::String {
                format,
                min_length,
                max_length,
                pattern,
            } => self.string(*format, *min_length, *max_length, pattern.as_deref()),
            InlineType::Bytes { .. } => "z.instanceof(Uint8Array)".into(),
            InlineType::Array { item, unique } => {
                let item = self.reference(item)?;
                match unique {
                    Some(true) => {
                        self.need.insert("unique");
                        format!("uniqueItems({item})")
                    }
                    _ => format!("z.array({item})"),
                }
            }
            InlineType::Map { key, value } => {
                let key = self.reference(key)?;
                let value = self.reference(value)?;
                format!("z.record({key}, {value})")
            }
            InlineType::Tuple { items } => {
                let items = self.items(items)?.join(", ");
                format!("z.tuple([{items}])")
            }
        })
    }

    fn items(&mut self, items: &[TypeRef]) -> Result<Vec<String>, EmitError> {
        items
            .iter()
            .map(|item| self.reference(item))
            .collect::<Result<Vec<_>, _>>()
    }

    fn int_carrier(&self, wire: IntegerWire, bits: u16) -> Carrier {
        if wire == IntegerWire::String {
            return Carrier::Text;
        }
        if bits > 53 {
            return match self.options.int64 {
                Int64::String => Carrier::Text,
                Int64::Bigint if self.options.mode == EmitMode::Strict => Carrier::Bigint,
                Int64::Bigint | Int64::Number => Carrier::Number,
            };
        }
        Carrier::Number
    }

    fn string(
        &mut self,
        format: Option<FieldFormat>,
        min_length: Option<u64>,
        max_length: Option<u64>,
        pattern: Option<&str>,
    ) -> String {
        if format == Some(FieldFormat::DateTime) && self.options.date_mode == DateMode::Date {
            return "z.date()".into();
        }
        let mut schema = match format {
            None => "z.string()".to_owned(),
            Some(FieldFormat::DateTime) => {
                self.need.insert("date-time");
                self.need.insert("instant");
                "instant()".into()
            }
            Some(FieldFormat::Date) => "z.iso.date()".into(),
            Some(FieldFormat::Time) => {
                self.need.insert("time");
                self.need.insert("clock");
                "clock()".into()
            }
            Some(FieldFormat::Duration) => "z.iso.duration()".into(),
            Some(FieldFormat::Uuid) => "z.uuid()".into(),
            Some(FieldFormat::Uri) => "z.url()".into(),
            Some(FieldFormat::Email) => "z.email()".into(),
            Some(FieldFormat::Ipv4) => "z.ipv4()".into(),
            Some(FieldFormat::Ipv6) => "z.ipv6()".into(),
        };
        if let Some(pattern) = pattern {
            schema.push_str(&format!(".regex(new RegExp({}))", string(pattern)));
        }
        if let Some(minimum) = min_length {
            schema.push_str(&format!(
                ".refine((value) => [...value].length >= {minimum}, \"string is shorter than the contract\")"
            ));
        }
        if let Some(maximum) = max_length {
            schema.push_str(&format!(
                ".refine((value) => [...value].length <= {maximum}, \"string is longer than the contract\")"
            ));
        }
        schema
    }

    fn definition(&mut self, id: &str, definition: &pons_ir::TypeDef) -> Result<String, EmitError> {
        match &definition.definition {
            TypeKind::Struct {
                fields,
                flattened,
                openness,
            } => self.structure(fields, flattened, openness, definition.direction),
            TypeKind::Enum { .. } => self.lazy(id, None),
            TypeKind::Newtype { inner } => self.reference(inner),
        }
    }

    fn variant(
        &mut self,
        id: &str,
        definition: &pons_ir::TypeDef,
        variant: &pons_ir::Variant,
    ) -> Result<String, EmitError> {
        let TypeKind::Enum { repr, .. } = &definition.definition else {
            unreachable!("a variant belongs to an enum")
        };
        let direction = definition.direction;
        let wire = string(&variant.wire);
        let payload = &variant.payload;
        Ok(match repr {
            EnumRepr::String => match payload {
                VariantPayload::Unit => format!("z.literal({wire})"),
                _ => {
                    return Err(EmitError(format!(
                        "the type {id} spells the variant {} as a string but gives it a payload, which a zod string cannot hold",
                        variant.wire
                    )));
                }
            },
            EnumRepr::External => {
                self.need.insert("one-key");
                match payload {
                    VariantPayload::Unit => {
                        format!("z.union([z.literal({wire}), oneKey({wire}, z.unknown())])")
                    }
                    _ => {
                        let payload = self.payload(payload, direction)?;
                        format!("oneKey({wire}, {payload})")
                    }
                }
            }
            EnumRepr::Internal { tag } => {
                let tag = string(tag);
                let mut members = vec![format!("{tag}: z.literal({wire})")];
                match payload {
                    VariantPayload::Unit => {}
                    VariantPayload::Struct { fields } => {
                        members.extend(self.members(fields, direction)?)
                    }
                    VariantPayload::Newtype { ty } => {
                        match self.inlined(ty) {
                            Some(definition) => {
                                let members_of_definition =
                                    self.members_of(definition)?;
                                members.extend(members_of_definition)
                            }
                            None => {
                                return Err(EmitError(
                                    "TypeScript cannot represent internally tagged newtype or tuple payloads"
                                        .into(),
                                ))
                            }
                        }
                    }
                    VariantPayload::Tuple { .. } => {
                        return Err(EmitError(
                            "TypeScript cannot represent internally tagged newtype or tuple payloads"
                                .into(),
                        ))
                    }
                }
                format!("z.object({{{}}})", members.join(", "))
            }
            EnumRepr::Adjacent { tag, content } => {
                let tag = string(tag);
                let content = string(content);
                match payload {
                    VariantPayload::Unit => format!("z.object({{{tag}: z.literal({wire})}})"),
                    _ => {
                        let payload = self.payload(payload, direction)?;
                        format!("z.object({{{tag}: z.literal({wire}), {content}: {payload}}})")
                    }
                }
            }
            EnumRepr::Untagged => self.payload(payload, direction)?,
        })
    }

    fn payload(
        &mut self,
        payload: &VariantPayload,
        direction: Direction,
    ) -> Result<String, EmitError> {
        Ok(match payload {
            VariantPayload::Unit => "z.null()".into(),
            VariantPayload::Newtype { ty } => self.reference(ty)?,
            VariantPayload::Tuple { items } => {
                let items = self.items(items)?.join(", ");
                format!("z.tuple([{items}])")
            }
            VariantPayload::Struct { fields } => {
                let members = self.members(fields, direction)?;
                format!("z.object({{{}}})", members.join(", "))
            }
        })
    }

    fn structure(
        &mut self,
        fields: &[Field],
        flattened: &[TypeRef],
        openness: &ObjectOpenness,
        direction: Direction,
    ) -> Result<String, EmitError> {
        let flattened = self.flattened(flattened);
        if flattened
            .iter()
            .any(|member| matches!(member, Flattened::Opaque))
        {
            return Ok("z.unknown()".into());
        }
        let mut members = self.members(fields, direction)?;
        for member in flattened {
            let Flattened::Struct(definition) = member else {
                continue;
            };
            members.extend(self.members_of(definition)?);
        }
        let mut schema = format!("z.object({{{}}})", members.join(", "));
        match openness {
            ObjectOpenness::Reject => schema.push_str(".strict()"),
            ObjectOpenness::PreserveTyped { value } => {
                let value = openness_value(value, self.document);
                let value = self.reference(value)?;
                schema.push_str(&format!(".catchall({value})"));
            }
            ObjectOpenness::PreserveOpaque => {}
        }
        Ok(schema)
    }

    fn members(
        &mut self,
        fields: &[Field],
        direction: Direction,
    ) -> Result<Vec<String>, EmitError> {
        let mut members = Vec::new();
        for field in fields {
            if !field_applies(field, direction) {
                continue;
            }
            let key = string(&match self.options.field_case {
                FieldCase::Wire => field.wire.clone(),
                FieldCase::Camel => camel(&field.ident),
            });
            let mut member = self.reference(&field.ty)?;
            if field.presence == Presence::Required && self.accepts_undefined(&field.ty) {
                self.need.insert("required");
                member = format!("required({member})");
            }
            if field.presence == Presence::Optional {
                member.push_str(".optional()");
            }
            members.push(format!("{key}: {member}"));
        }
        Ok(members)
    }

    fn members_of(&mut self, definition: &pons_ir::TypeDef) -> Result<Vec<String>, EmitError> {
        let TypeKind::Struct { fields, .. } = &definition.definition else {
            return Ok(Vec::new());
        };
        let fields = fields.clone();
        let direction = definition.direction;
        self.members(&fields, direction)
    }

    fn flattened(&self, members: &[TypeRef]) -> Vec<Flattened<'a>> {
        let mut found = Vec::new();
        let mut seen = BTreeSet::new();
        for member in members {
            let key = match member {
                TypeRef::Named(named) => named.id.clone(),
                TypeRef::Inline(_) => String::new(),
            };
            if seen.insert(key) {
                found.push(self.flattened_member(member));
            }
        }
        found
    }

    fn inlined(&self, reference: &TypeRef) -> Option<&'a pons_ir::TypeDef> {
        match self.flattened_member(reference) {
            Flattened::Struct(definition) => Some(definition),
            _ => None,
        }
    }

    fn flattened_member(&self, reference: &TypeRef) -> Flattened<'a> {
        let mut current = reference;
        let mut seen = BTreeSet::new();
        loop {
            match current {
                TypeRef::Inline(InlineType::Opaque { .. }) => return Flattened::Opaque,
                TypeRef::Inline(_) => return Flattened::Other,
                TypeRef::Named(named) => {
                    if named.variant.is_some() || !seen.insert(named.id.clone()) {
                        return Flattened::Other;
                    }
                    let Some(definition) = self.document.types.get(&named.id) else {
                        return Flattened::Other;
                    };
                    match &definition.definition {
                        TypeKind::Newtype { inner } => current = inner,
                        TypeKind::Struct { .. } => return Flattened::Struct(definition),
                        TypeKind::Enum { .. } => return Flattened::Other,
                    }
                }
            }
        }
    }

    fn accepts_undefined(&self, reference: &TypeRef) -> bool {
        match reference {
            TypeRef::Inline(InlineType::Any | InlineType::Opaque { .. }) => true,
            TypeRef::Inline(InlineType::Nullable { inner }) => self.accepts_undefined(inner),
            TypeRef::Inline(_) => false,
            TypeRef::Named(named) => {
                let Some(definition) = self.document.types.get(&named.id) else {
                    return false;
                };
                match &definition.definition {
                    TypeKind::Newtype { inner } => self.accepts_undefined(inner),
                    _ => false,
                }
            }
        }
    }
}

enum Flattened<'a> {
    Struct(&'a pons_ir::TypeDef),
    Opaque,
    Other,
}

enum Carrier {
    Text,
    Bigint,
    Number,
}

fn field_applies(field: &Field, direction: Direction) -> bool {
    if matches!(
        (direction, field.access),
        (Direction::Input, Some(Access::ReadOnly)) | (Direction::Output, Some(Access::WriteOnly))
    ) {
        return false;
    }
    direction == Direction::Input || field.access != Some(Access::WriteOnly)
}

fn openness_value<'a>(ty: &'a TypeRef, document: &'a pons_ir::Document) -> &'a TypeRef {
    let mut current = ty;
    let mut visited = BTreeSet::new();
    loop {
        match current {
            TypeRef::Inline(InlineType::Map { value, .. }) => return value,
            TypeRef::Named(named) if visited.insert(named.id.clone()) => {
                let Some(TypeKind::Newtype { inner }) = document
                    .types
                    .get(&named.id)
                    .map(|definition| &definition.definition)
                else {
                    return current;
                };
                current = inner;
            }
            _ => return current,
        }
    }
}
