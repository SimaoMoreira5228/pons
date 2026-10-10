use std::collections::{BTreeMap, BTreeSet};

use pons_ir::{
    DecimalWire, Document, EnumRepr, FieldFormat, Presence, TypeKind, TypeRef, ValidationCoverage,
};

use super::backend::{
    Backend as BackendTrait, Capability, Declaration, DeclarationKind, IntCarrier, IntegerShape,
    Member, ObjectShape, Rendered, StringCarrier, StringShape,
};
use super::walk::{Walk, declaration_names};
use super::{Capabilities, Generator as ValidationGenerator, Imports, Rendered as RenderedFile};
use crate::{EmitError, TsOptions, jsdoc, string};

const EXTERNAL: Capability = Capability::new("validation.enum.external");
const INTERNAL: Capability = Capability::new("validation.enum.internal");
const ADJACENT: Capability = Capability::new("validation.enum.adjacent");
const UNTAGGED: Capability = Capability::new("validation.enum.untagged");
const PRESERVE_TYPED: Capability = Capability::new("validation.openness.preserve_typed");
const MAP: Capability = Capability::new("validation.map");

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
    ("date", "const DATE = /^(\\d{4})-(\\d{2})-(\\d{2})$/;\n"),
    (
        "calendar",
        "function isCalendarDay(value: string): boolean {\n  const match = DATE.exec(value);\n  if (match === null) return false;\n  const year = Number(match[1]);\n  const month = Number(match[2]);\n  const day = Number(match[3]);\n  const date = new Date(Date.UTC(year, month - 1, day));\n  return date.getUTCFullYear() === year && date.getUTCMonth() === month - 1 && date.getUTCDate() === day;\n}\nconst day = () => yup.string().matches(DATE, \"the date is not a full-date\").test(\"calendar\", \"the date is not a day the calendar has\", (value) => value === undefined || isCalendarDay(value as string));\n",
    ),
    (
        "date-time",
        "const DATE_TIME = /^\\d{4}-\\d{2}-\\d{2}[Tt]\\d{2}:\\d{2}:\\d{2}(?:\\.\\d{1,3})?(?:[Zz]|[+-]\\d{2}:\\d{2})$/;\n",
    ),
    (
        "instant",
        "const instant = () => yup.string().matches(DATE_TIME, \"the date-time is not a full-date and a full-time\").test(\"instant\", \"date-time is not an instant a Date can hold\", (value) => value === undefined || !Number.isNaN(Date.parse(value as string)));\n",
    ),
    (
        "time",
        "const TIME = /^\\d{2}:\\d{2}:\\d{2}(?:\\.\\d+)?(?:[Zz]|[+-]\\d{2}:\\d{2})$/;\n",
    ),
    (
        "clock",
        "const clock = () => yup.string().matches(TIME, \"the time is not a full-time\");\n",
    ),
    (
        "duration",
        "const DURATION = /^-?P(?=\\d|T\\d)(?:\\d+Y)?(?:\\d+M)?(?:\\d+W)?(?:\\d+D)?(?:T(?=\\d)(?:\\d+H)?(?:\\d+M)?(?:\\d+(?:\\.\\d+)?S)?)?$/;\n",
    ),
    (
        "uuid",
        "const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;\n",
    ),
    ("email", "const EMAIL = /^[^\\s@]+@[^\\s@]+\\.[^\\s@]+$/;\n"),
    (
        "ipv4",
        "const IPV4 = /^(?:(?:25[0-5]|2[0-4]\\d|1\\d\\d|[1-9]?\\d)\\.){3}(?:25[0-5]|2[0-4]\\d|1\\d\\d|[1-9]?\\d)$/;\n",
    ),
    (
        "ipv6",
        r#"const IPV6 = /^(?:(?:[0-9a-f]{1,4}:){7}[0-9a-f]{1,4}|(?:[0-9a-f]{1,4}:){1,7}:|(?:[0-9a-f]{1,4}:){1,6}:[0-9a-f]{1,4}|(?:[0-9a-f]{1,4}:){1,5}(?::[0-9a-f]{1,4}){1,2}|(?:[0-9a-f]{1,4}:){1,4}(?::[0-9a-f]{1,4}){1,3}|(?:[0-9a-f]{1,4}:){1,3}(?::[0-9a-f]{1,4}){1,4}|(?:[0-9a-f]{1,4}:){1,2}(?::[0-9a-f]{1,4}){1,5}|[0-9a-f]{1,4}:(?::[0-9a-f]{1,4}){1,6}|:(?:(?::[0-9a-f]{1,4}){1,7}|:))$/i;
"#,
    ),
    (
        "uri",
        "const isUri = (value: string): boolean => {\n  try {\n    new URL(value);\n    return true;\n  } catch {\n    return false;\n  }\n};\n",
    ),
    (
        "schema-of",
        "function schemaOf(schema: yup.ISchema, value: unknown): yup.Schema {\n  let current = schema;\n  while (current.type === \"lazy\") current = (current as yup.Lazy).resolve({ value });\n  return current as yup.Schema;\n}\n",
    ),
    (
        "present",
        "const present = (schema: yup.ISchema) => yup.lazy((value) => value === undefined ? yup.mixed().defined() : schemaOf(schema, value));\n",
    ),
    (
        "optional",
        "const optional = (schema: yup.ISchema) => yup.lazy((value) => value === undefined ? yup.mixed() : schemaOf(schema, value));\n",
    ),
    (
        "nullable",
        "const nullable = (schema: yup.ISchema) => yup.lazy((value) => value === null ? yup.mixed().nullable() : schemaOf(schema, value));\n",
    ),
    (
        "integer",
        "function exactInteger(bits: number, signed: boolean, minimum?: string, maximum?: string, multipleOf?: string) {\n  const magnitude = 1n << BigInt(signed ? bits - 1 : bits);\n  const low = signed ? -magnitude : 0n;\n  const high = magnitude - 1n;\n  return (value: unknown): string | undefined => {\n    let integer: bigint;\n    try {\n      integer = BigInt(value as bigint | number | string);\n    } catch {\n      return \"expected an exact integer\";\n    }\n    if (integer < low || integer > high) return \"integer is outside its native range\";\n    if (minimum !== undefined && integer < BigInt(minimum)) return \"integer is below its minimum\";\n    if (maximum !== undefined && integer > BigInt(maximum)) return \"integer is above its maximum\";\n    if (multipleOf !== undefined && (BigInt(multipleOf) <= 0n || integer % BigInt(multipleOf) !== 0n)) return \"integer does not match its multiple\";\n    return undefined;\n  };\n}\nfunction integer(base: yup.Schema, bits: number, signed: boolean, minimum?: string, maximum?: string, multipleOf?: string): yup.Schema {\n  const fault = exactInteger(bits, signed, minimum, maximum, multipleOf);\n  return base.test(\"integer\", \"expected an exact integer\", (value) => fault(value) === undefined);\n}\n",
    ),
    (
        "unique",
        "function stableKey(value: unknown): string {\n  if (typeof value === \"bigint\") return `bigint:${value}`;\n  if (Array.isArray(value)) return `[${value.map(stableKey).join(\",\")}]`;\n  if (value !== null && typeof value === \"object\") {\n    const record = value as Record<string, unknown>;\n    return `{${Object.keys(record).sort().map((key) => `${JSON.stringify(key)}:${stableKey(record[key])}`).join(\",\")}}`;\n  }\n  return JSON.stringify(value);\n}\nconst uniqueItems = (item: yup.ISchema) => yup.array().of(present(item)).test(\"unique\", \"array values must be unique\", (values) => values === undefined || new Set((values as unknown[]).map(stableKey)).size === (values as unknown[]).length);\n",
    ),
];

const MISMATCH: &str = r#"function parsePath(path: string | undefined): (string | number)[] {
  if (path === undefined || path === "") return [];
  const steps: (string | number)[] = [];
  for (const part of path.match(/[^.[\]]+|\[(?:\d+|'[^']*'|"[^"]*")\]/g) ?? []) {
    if (part.startsWith("[")) {
      const inner = part.slice(1, -1);
      steps.push(/^\d+$/.test(inner) ? Number(inner) : inner.slice(1, -1));
    } else {
      steps.push(part);
    }
  }
  return steps;
}
function mismatch(schema: yup.ISchema): Validator {
  return (value) => {
    if (value === undefined) return { path: [], message: "a JSON value is required" };
    try {
      schema.validateSync(value, { strict: true, abortEarly: true, stripUnknown: false, recursive: true });
      return null;
    } catch (failure) {
      if (failure instanceof yup.ValidationError) {
        const issue = failure.inner[0] ?? failure;
        return { path: parsePath(issue.path), message: issue.message };
      }
      return { path: [], message: failure instanceof Error ? failure.message : String(failure) };
    }
  };
}
"#;

struct VariantMeta {
    repr: EnumRepr,
    wire: String,
}

pub(super) struct Generator<'a> {
    options: &'a TsOptions,
    walk: Walk<'a>,
    backend: Backend,
    imports: BTreeSet<String>,
}

impl<'a> Generator<'a> {
    pub(super) fn new(
        document: &'a Document,
        names: &'a BTreeMap<String, String>,
        options: &'a TsOptions,
    ) -> Self {
        Generator {
            options,
            walk: Walk::new(document, names, options, &[]),
            backend: Backend::new(document, names),
            imports: BTreeSet::new(),
        }
    }
}

impl ValidationGenerator for Generator<'_> {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            module: "validators",
        }
    }

    fn coverage(&mut self, _document: &Document) -> Result<ValidationCoverage, EmitError> {
        self.walk.declarations(&mut self.backend)?;
        Ok(self.walk.coverage())
    }

    fn has_bindings(&self) -> bool {
        self.walk.has_bindings()
    }

    fn validate(&mut self, reference: &TypeRef, subject: &str) -> Result<String, EmitError> {
        let name = self.walk.validate(&mut self.backend, reference, subject)?;
        self.imports.insert(name.clone());
        Ok(name)
    }

    fn take_imports(&mut self) -> Imports {
        Imports {
            names: std::mem::take(&mut self.imports),
        }
    }

    fn render(&mut self) -> Result<RenderedFile, EmitError> {
        self.walk.declarations(&mut self.backend)?;
        let mut body = self.walk.module(&mut self.backend)?;
        body.push_str(MISMATCH);
        for binding in self.walk.bindings()? {
            body.push_str(&format!(
                "export const {}: Validator = mismatch({});\n",
                binding.name, binding.schema
            ));
        }
        let specifier = crate::runtime::specifier(self.options.runtime, "\"./runtime.js\"");
        let package = self
            .options
            .validation
            .import_from
            .as_deref()
            .unwrap_or("yup");
        Ok(RenderedFile {
            file: "validators.ts",
            content: format!(
                "import * as yup from {};\n\
                 import type {{ ValidationIssue, Validator }} from {specifier};\n\
                 export type {{ ValidationIssue, Validator }} from {specifier};\n\n{body}",
                string(package)
            ),
        })
    }
}

pub(super) struct Backend {
    variants: BTreeMap<String, VariantMeta>,
    need: BTreeSet<&'static str>,
}

impl Backend {
    fn new(document: &Document, names: &BTreeMap<String, String>) -> Self {
        let declarations = declaration_names(document, names);
        let mut variants = BTreeMap::new();
        for (id, definition) in &document.types {
            let TypeKind::Enum {
                repr,
                variants: choices,
            } = &definition.definition
            else {
                continue;
            };
            for variant in choices {
                let key = (id.clone(), Some(variant.wire.clone()));
                if let Some(name) = declarations.get(&key) {
                    variants.insert(
                        format!("{name}Schema"),
                        VariantMeta {
                            repr: repr.clone(),
                            wire: variant.wire.clone(),
                        },
                    );
                }
            }
        }
        Backend {
            variants,
            need: BTreeSet::new(),
        }
    }

    fn require(&mut self, id: &'static str) {
        if !self.need.insert(id) {
            return;
        }
        match id {
            "calendar" => self.require("date"),
            "instant" => self.require("date-time"),
            "clock" => self.require("time"),
            "present" | "optional" | "nullable" => self.require("schema-of"),
            "unique" => self.require("present"),
            _ => {}
        }
    }
}

fn nullable(inner: &str) -> String {
    format!("nullable({inner})")
}

fn present(inner: &str) -> String {
    format!("present({inner})")
}

fn optional(inner: &str) -> String {
    format!("optional({inner})")
}

impl BackendTrait for Backend {
    fn name(&self) -> &'static str {
        "yup"
    }

    fn reference(&mut self, name: &str, _variant: Option<&str>) -> Rendered {
        Rendered::exact(format!("yup.lazy(() => {name}Schema)"))
    }

    fn schema(&self, name: &str) -> String {
        format!("{name}Schema")
    }

    fn object(&mut self, members: &[Member], openness: &ObjectShape) -> Rendered {
        let mut parts = Vec::with_capacity(members.len());
        for member in members {
            let (helper, schema) = match member.presence {
                Presence::Required => ("present", present(&member.schema)),
                Presence::Optional => ("optional", optional(&member.schema)),
            };
            self.require(helper);
            parts.push(format!("{}: {schema}", string(&member.key)));
        }
        let mut schema = format!("yup.object({{{}}})", parts.join(", "));
        match openness {
            ObjectShape::Reject => schema.push_str(".noUnknown(true)"),
            ObjectShape::PreserveOpaque => {}
            ObjectShape::PreserveTyped(_) => {
                return Rendered::Widened(schema, PRESERVE_TYPED);
            }
        }
        Rendered::exact(schema)
    }

    fn union(&mut self, members: &[String]) -> Rendered {
        let Some(meta) = members.first().and_then(|name| self.variants.get(name)) else {
            return Rendered::Widened("yup.mixed()".to_owned(), UNTAGGED);
        };
        match &meta.repr {
            EnumRepr::String => {
                let values = members
                    .iter()
                    .filter_map(|name| self.variants.get(name))
                    .map(|meta| string(&meta.wire))
                    .collect::<Vec<_>>()
                    .join(", ");
                Rendered::exact(format!("yup.string().oneOf([{values}])"))
            }
            EnumRepr::External => Rendered::Widened("yup.mixed()".to_owned(), EXTERNAL),
            EnumRepr::Internal { tag } => Rendered::Widened(
                format!("yup.object({{{}: yup.string()}})", string(tag)),
                INTERNAL,
            ),
            EnumRepr::Adjacent { tag, .. } => Rendered::Widened(
                format!("yup.object({{{}: yup.string()}})", string(tag)),
                ADJACENT,
            ),
            EnumRepr::Untagged => Rendered::Widened("yup.mixed()".to_owned(), UNTAGGED),
        }
    }

    fn literal(&mut self, value: &str) -> Rendered {
        Rendered::exact(format!("yup.string().oneOf([{}])", string(value)))
    }

    fn external(&mut self, wire: &str, payload: Option<&str>) -> Rendered {
        match payload {
            Some(payload) => {
                self.require("present");
                Rendered::exact(format!(
                    "yup.object({{{}: {}}}).noUnknown(true)",
                    string(wire),
                    present(payload)
                ))
            }
            None => Rendered::Widened("yup.mixed()".to_owned(), EXTERNAL),
        }
    }

    fn string(&mut self, shape: &StringShape) -> Rendered {
        if shape.carrier == StringCarrier::Date {
            return Rendered::exact("yup.date()");
        }
        let mut schema = match shape.format {
            None => "yup.string()".to_owned(),
            Some(FieldFormat::DateTime) => {
                self.require("instant");
                "instant()".into()
            }
            Some(FieldFormat::Date) => {
                self.require("calendar");
                "day()".into()
            }
            Some(FieldFormat::Time) => {
                self.require("clock");
                "clock()".into()
            }
            Some(FieldFormat::Duration) => {
                self.require("duration");
                "yup.string().matches(DURATION, \"the duration is not an ISO 8601 duration\")"
                    .into()
            }
            Some(FieldFormat::Uuid) => {
                self.require("uuid");
                "yup.string().matches(UUID, \"the value is not a UUID\")".into()
            }
            Some(FieldFormat::Uri) => {
                self.require("uri");
                "yup.string().test(\"uri\", \"the value is not a URI\", (value) => value === undefined || isUri(value as string))".into()
            }
            Some(FieldFormat::Email) => {
                self.require("email");
                "yup.string().matches(EMAIL, \"the value is not an email address\")".into()
            }
            Some(FieldFormat::Ipv4) => {
                self.require("ipv4");
                "yup.string().matches(IPV4, \"the value is not an IPv4 address\")".into()
            }
            Some(FieldFormat::Ipv6) => {
                self.require("ipv6");
                "yup.string().matches(IPV6, \"the value is not an IPv6 address\")".into()
            }
        };
        if let Some(pattern) = shape.pattern {
            schema.push_str(&format!(
                ".matches(new RegExp({}), \"string does not match its pattern\")",
                string(pattern)
            ));
        }
        if let Some(minimum) = shape.min_length {
            schema.push_str(&format!(
                ".test(\"min-length\", \"string is shorter than the contract\", (value) => value === undefined || [...(value as string)].length >= {minimum})"
            ));
        }
        if let Some(maximum) = shape.max_length {
            schema.push_str(&format!(
                ".test(\"max-length\", \"string is longer than the contract\", (value) => value === undefined || [...(value as string)].length <= {maximum})"
            ));
        }
        Rendered::exact(schema)
    }

    fn integer(&mut self, shape: &IntegerShape) -> Rendered {
        self.require("integer");
        let base = match shape.carrier {
            IntCarrier::Text => {
                self.require(if shape.signed {
                    "canonical-signed-integer"
                } else {
                    "canonical-integer"
                });
                format!(
                    "yup.string().matches({})",
                    if shape.signed {
                        "CANONICAL_SIGNED_INTEGER"
                    } else {
                        "CANONICAL_INTEGER"
                    }
                )
            }
            IntCarrier::Bigint => {
                "yup.mixed().test(\"bigint\", \"expected a bigint\", (value) => typeof value === \"bigint\")"
                    .into()
            }
            IntCarrier::Number => "yup.number().integer()".into(),
        };
        let mut arguments = vec![base, shape.bits.to_string(), shape.signed.to_string()];
        for value in [shape.minimum, shape.maximum, shape.multiple_of] {
            arguments.push(match value {
                Some(value) => string(&value.to_string()),
                None => String::from("undefined"),
            });
        }
        while arguments.last().map(String::as_str) == Some("undefined") {
            arguments.pop();
        }
        Rendered::exact(format!("integer({})", arguments.join(", ")))
    }

    fn float(&mut self, bits: u8) -> Rendered {
        if bits == 32 {
            Rendered::exact(
                "yup.number().test(\"f32\", \"number is outside the f32 range\", (value) => value === undefined || Number.isFinite(Math.fround(value as number)))",
            )
        } else {
            Rendered::exact(
                "yup.number().test(\"finite\", \"number is not finite\", (value) => value === undefined || Number.isFinite(value as number))",
            )
        }
    }

    fn decimal(&mut self, wire: DecimalWire) -> Rendered {
        match wire {
            DecimalWire::Number => Rendered::exact(
                "yup.number().test(\"finite\", \"number is not finite\", (value) => value === undefined || Number.isFinite(value as number))",
            ),
            DecimalWire::String => {
                self.require("decimal");
                Rendered::exact("yup.string().matches(DECIMAL, \"the value is not a decimal\")")
            }
        }
    }

    fn bytes(&mut self) -> Rendered {
        Rendered::exact(
            "yup.mixed().test(\"bytes\", \"expected bytes\", (value) => value instanceof Uint8Array)",
        )
    }

    fn boolean(&mut self) -> Rendered {
        Rendered::exact("yup.boolean()")
    }

    fn array(&mut self, item: &str, unique: bool) -> Rendered {
        self.require("present");
        if unique {
            self.require("unique");
            Rendered::exact(format!("uniqueItems({item})"))
        } else {
            Rendered::exact(format!("yup.array().of({})", present(item)))
        }
    }

    fn tuple(&mut self, items: &[String]) -> Rendered {
        let items = items
            .iter()
            .map(|item| {
                self.require("present");
                present(item)
            })
            .collect::<Vec<_>>()
            .join(", ");
        Rendered::exact(format!("yup.tuple([{items}])"))
    }

    fn map(&mut self, _key: &str, _value: &str) -> Rendered {
        Rendered::Widened("yup.object()".into(), MAP)
    }

    fn nullable(&mut self, inner: &str) -> Rendered {
        self.require("nullable");
        Rendered::exact(nullable(inner))
    }

    fn any(&mut self) -> Rendered {
        Rendered::exact("yup.mixed()")
    }

    fn unknown(&mut self) -> Rendered {
        Rendered::exact("yup.mixed()")
    }

    fn null(&mut self) -> Rendered {
        Rendered::exact("yup.mixed().nullable().oneOf([null]).defined()")
    }

    fn unit(&mut self) -> Rendered {
        Rendered::exact("yup.mixed().nullable().oneOf([null]).defined()")
    }

    fn validator(&mut self, schema: &str) -> String {
        format!("mismatch({schema})")
    }

    fn module(&mut self, declarations: &[Declaration<'_>]) -> Result<String, EmitError> {
        let names = declarations
            .iter()
            .map(|declaration| declaration.name)
            .collect::<BTreeSet<_>>();
        for declaration in declarations {
            for dependency in declaration.dependencies {
                if !names.contains(dependency.as_str()) {
                    return Err(EmitError(format!(
                        "the yup backend cannot declare {} without {dependency}",
                        declaration.name
                    )));
                }
            }
        }
        let mut output = String::new();
        for (id, code) in PRELUDE {
            if self.need.contains(id) {
                output.push_str(code);
                output.push('\n');
            }
        }
        for declaration in declarations {
            let comment = jsdoc(declaration.doc);
            if !comment.is_empty() {
                output.push_str(&comment);
                output.push('\n');
            }
            output.push_str(&format!(
                "const {}Schema = {};\n",
                declaration.name, declaration.schema
            ));
            if declaration.kind == DeclarationKind::Type {
                output.push('\n');
            }
        }
        Ok(output)
    }
}
