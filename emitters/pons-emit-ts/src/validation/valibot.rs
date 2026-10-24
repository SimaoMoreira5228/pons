use std::collections::{BTreeMap, BTreeSet};

use pons_ir::{DecimalWire, Document, FieldFormat, Presence, TypeRef, ValidationCoverage};

use super::backend::{
    Backend as BackendTrait, Capability, Declaration, DeclarationKind, IntCarrier, IntegerShape,
    Member, ObjectShape, Rendered, StringCarrier, StringShape, integral_constraint,
};
use super::walk::Walk;
use super::{Capabilities, Generator as ValidationGenerator, Imports, Rendered as RenderedFile};
use crate::render::{jsdoc, string};
use crate::{EmitError, TsOptions};

const MAP: Capability = Capability::new("validation.map");

const MISMATCH: &str = "function mismatch(schema: v.GenericSchema): (value: unknown) => ValidationIssue | null {\n  return (value) => {\n    const result = v.safeParse(schema, value);\n    if (result.success) return null;\n    const issue = result.issues[0];\n    if (issue === undefined) return { path: [], message: \"the value does not match the contract\" };\n    return { path: issue.path?.map((step) => typeof step.key === \"number\" ? step.key : String(step.key)) ?? [], message: issue.message };\n  };\n}\n";

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
        "time",
        "const TIME = /^(\\d{2}):(\\d{2}):(\\d{2})(?:\\.\\d+)?(?:[Zz]|([+-])(\\d{2}):(\\d{2}))$/;\n",
    ),
    (
        "date-time",
        "const DATE_TIME = /^(\\d{4})-(\\d{2})-(\\d{2})[Tt](\\d{2}):(\\d{2}):(\\d{2})(?:\\.\\d{1,3})?(?:[Zz]|([+-])(\\d{2}):(\\d{2}))$/;\n",
    ),
    (
        "calendar",
        r#"function isCalendarParts(yearText: string, monthText: string, dayText: string): boolean {
  const year = Number(yearText);
  const month = Number(monthText);
  const day = Number(dayText);
  if (month < 1 || month > 12 || day < 1) return false;
  const days = month === 2 ? (year % 4 === 0 && (year % 100 !== 0 || year % 400 === 0) ? 29 : 28) : [4, 6, 9, 11].includes(month) ? 30 : 31;
  return day <= days;
}
function isClockParts(hourText: string, minuteText: string, secondText: string): boolean {
  return Number(hourText) <= 23 && Number(minuteText) <= 59 && Number(secondText) <= 59;
}
function isOffsetParts(hourText: string | undefined, minuteText: string | undefined): boolean {
  return hourText === undefined || (Number(hourText) <= 23 && Number(minuteText) <= 59);
}
"#,
    ),
    (
        "date-schema",
        r#"const date = () => v.pipe(v.string(), v.check((value) => {
  const match = DATE.exec(value);
  return match !== null && isCalendarParts(match[1]!, match[2]!, match[3]!);
}, "expected a date"));
"#,
    ),
    (
        "clock-schema",
        r#"const clock = () => v.pipe(v.string(), v.check((value) => {
  const match = TIME.exec(value);
  return match !== null && isClockParts(match[1]!, match[2]!, match[3]!) && isOffsetParts(match[5], match[6]);
}, "expected a time"));
"#,
    ),
    (
        "instant-schema",
        r#"const instant = () => v.pipe(v.string(), v.check((value) => {
  const match = DATE_TIME.exec(value);
  return match !== null && isCalendarParts(match[1]!, match[2]!, match[3]!) && isClockParts(match[4]!, match[5]!, match[6]!) && isOffsetParts(match[8], match[9]) && !Number.isNaN(Date.parse(value));
}, "expected a date-time"));
"#,
    ),
    (
        "duration",
        "const DURATION = /^P(?:(\\d+W)|(?!.*W)(?=\\d|T\\d)(\\d+Y)?(\\d+M)?(\\d+D)?(T(?=\\d)(\\d+H)?(\\d+M)?(\\d+([.,]\\d+)?S)?)?)$/;\n",
    ),
    (
        "uuid",
        "const UUID = /^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$/;\n",
    ),
    (
        "email",
        "const EMAIL = /^(?:[A-Za-z0-9_'+-]+\\.)*[A-Za-z0-9_'+-]*[A-Za-z0-9_+-]@(?:[A-Za-z0-9][A-Za-z0-9-]*\\.)+[A-Za-z]{2,}$/;\n",
    ),
    (
        "ipv4",
        "const IPV4 = /^(?:(?:25[0-5]|2[0-4][0-9]|1[0-9][0-9]|[1-9]?[0-9])\\.){3}(?:25[0-5]|2[0-4][0-9]|1[0-9][0-9]|[1-9]?[0-9])$/;\n",
    ),
    (
        "ipv6",
        "const IPV6 = /^(([0-9a-fA-F]{1,4}:){7}[0-9a-fA-F]{1,4}|([0-9a-fA-F]{1,4}:){1,7}:|([0-9a-fA-F]{1,4}:){1,6}:[0-9a-fA-F]{1,4}|([0-9a-fA-F]{1,4}:){1,5}(:[0-9a-fA-F]{1,4}){1,2}|([0-9a-fA-F]{1,4}:){1,4}(:[0-9a-fA-F]{1,4}){1,3}|([0-9a-fA-F]{1,4}:){1,3}(:[0-9a-fA-F]{1,4}){1,4}|([0-9a-fA-F]{1,4}:){1,2}(:[0-9a-fA-F]{1,4}){1,5}|[0-9a-fA-F]{1,4}:((:[0-9a-fA-F]{1,4}){1,6})|:((:[0-9a-fA-F]{1,4}){1,7}|:))$/;\n",
    ),
    (
        "uri",
        r#"function isUri(value: string): boolean {
  if (/\s/.test(value)) return false;
  try {
    return new URL(value).protocol.length > 1;
  } catch {
    return false;
  }
}
const uri = () => v.pipe(v.string(), v.check(isUri, "expected a uri"));
"#,
    ),
    (
        "binary32",
        "const binary32 = () => v.pipe(v.number(), v.check((value) => Number.isFinite(Math.fround(value)), \"number is outside the f32 range\"));\n",
    ),
    (
        "integer",
        r#"function exactInteger(bits: number, signed: boolean, minimum?: string, maximum?: string, multipleOf?: string) {
  const magnitude = 1n << BigInt(signed ? bits - 1 : bits);
  const low = signed ? -magnitude : 0n;
  const high = magnitude - 1n;
  return (value: unknown): string | undefined => {
    let integer: bigint;
    try {
      integer = BigInt(value as bigint | number | string);
    } catch {
      return "expected an exact integer";
    }
    if (integer < low || integer > high) return "integer is outside its native range";
    if (minimum !== undefined && integer < BigInt(minimum)) return "integer is below its minimum";
    if (maximum !== undefined && integer > BigInt(maximum)) return "integer is above its maximum";
    if (multipleOf !== undefined && (BigInt(multipleOf) <= 0n || integer % BigInt(multipleOf) !== 0n)) return "integer does not match its multiple";
    return undefined;
  };
}
function integer(base: v.GenericSchema, bits: number, signed: boolean, minimum?: string, maximum?: string, multipleOf?: string) {
  const fault = exactInteger(bits, signed, minimum, maximum, multipleOf);
  return v.pipe(base, v.check((value) => fault(value) === undefined, "integer is outside the contract"));
}
"#,
    ),
    (
        "one-key",
        "const oneKey = (key: string, payload: v.GenericSchema) => plainRecord(v.strictObject({ [key]: v.nonOptional(payload) }));\n",
    ),
    (
        "plain-record",
        r#"function isPlainRecord(value: unknown): value is Record<string, unknown> {
  if (value === null || typeof value !== "object" || Array.isArray(value)) return false;
  const prototype = Object.getPrototypeOf(value);
  return prototype === Object.prototype || prototype === null;
}
function plainRecord<TSchema extends v.BaseSchema<unknown, unknown, v.BaseIssue<unknown>>>(schema: TSchema): TSchema {
  return {
    ...schema,
    "~run"(dataset: v.UnknownDataset, config: v.Config<v.BaseIssue<unknown>>) {
      if (!isPlainRecord(dataset.value)) {
        v._addIssue(schema, "type", dataset, config, { message: "expected a plain object" });
        return dataset;
      }
      return schema["~run"](dataset, config);
    },
  } as unknown as TSchema;
}
"#,
    ),
    (
        "unique",
        r#"function stableKey(value: unknown): string {
  if (typeof value === "bigint") return `bigint:${value}`;
  if (value instanceof Date) return `date:${value.getTime()}`;
  if (value instanceof Uint8Array) return `bytes:${Array.from(value).join(",")}`;
  if (Array.isArray(value)) return `[${value.map(stableKey).join(",")}]`;
  if (value !== null && typeof value === "object") {
    const record = value as Record<string, unknown>;
    return `{${Object.keys(record).sort().map((key) => `${JSON.stringify(key)}:${stableKey(record[key])}`).join(",")}}`;
  }
  return JSON.stringify(value);
}
const uniqueItems = (item: v.GenericSchema) => v.pipe(v.array(item), v.checkItems((_value, _index, values) => new Set(values.map(stableKey)).size === values.length, "array values must be unique"));
"#,
    ),
];

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
            backend: Backend::new(),
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
                "export const {}: Validator = {};\n",
                binding.name,
                self.backend.validator(&binding.schema)
            ));
        }
        let specifier = crate::runtime::specifier(self.options.runtime, "\"./runtime.js\"");
        let package = self
            .options
            .validation
            .import_from
            .as_deref()
            .unwrap_or("valibot");
        Ok(RenderedFile {
            file: "validators.ts",
            content: format!(
                "import * as v from {};\n\
                 import type {{ ValidationIssue, Validator }} from {specifier};\n\
                 export type {{ ValidationIssue, Validator }} from {specifier};\n\n{body}",
                string(package)
            ),
        })
    }
}

struct Backend {
    need: BTreeSet<&'static str>,
}

impl Backend {
    fn new() -> Self {
        Backend {
            need: BTreeSet::new(),
        }
    }

    fn need(&mut self, ids: &[&'static str]) {
        self.need.extend(ids.iter().copied());
    }

    fn string_schema(&mut self, shape: &StringShape) -> String {
        if shape.carrier == StringCarrier::Date {
            return "v.date()".to_owned();
        }
        let base = match shape.format {
            None => "v.string()".to_owned(),
            Some(FieldFormat::DateTime) => {
                self.need(&["date-time", "calendar", "instant-schema"]);
                "instant()".into()
            }
            Some(FieldFormat::Date) => {
                self.need(&["date", "calendar", "date-schema"]);
                "date()".into()
            }
            Some(FieldFormat::Time) => {
                self.need(&["time", "calendar", "clock-schema"]);
                "clock()".into()
            }
            Some(FieldFormat::Duration) => {
                self.need(&["duration"]);
                "v.pipe(v.string(), v.regex(DURATION, \"expected a duration\"))".into()
            }
            Some(FieldFormat::Uuid) => {
                self.need(&["uuid"]);
                "v.pipe(v.string(), v.regex(UUID, \"expected a uuid\"))".into()
            }
            Some(FieldFormat::Uri) => {
                self.need(&["uri"]);
                "uri()".into()
            }
            Some(FieldFormat::Email) => {
                self.need(&["email"]);
                "v.pipe(v.string(), v.regex(EMAIL, \"expected an email\"))".into()
            }
            Some(FieldFormat::Ipv4) => {
                self.need(&["ipv4"]);
                "v.pipe(v.string(), v.regex(IPV4, \"expected an ipv4 address\"))".into()
            }
            Some(FieldFormat::Ipv6) => {
                self.need(&["ipv6"]);
                "v.pipe(v.string(), v.regex(IPV6, \"expected an ipv6 address\"))".into()
            }
        };
        let mut actions = Vec::new();
        if let Some(pattern) = shape.pattern {
            actions.push(format!("v.regex(new RegExp({}))", string(pattern)));
        }
        if let Some(minimum) = shape.min_length {
            actions.push(format!(
                "v.check((value) => [...value].length >= {minimum}, \"string is shorter than the contract\")"
            ));
        }
        if let Some(maximum) = shape.max_length {
            actions.push(format!(
                "v.check((value) => [...value].length <= {maximum}, \"string is longer than the contract\")"
            ));
        }
        if actions.is_empty() {
            base
        } else {
            format!("v.pipe({base}, {})", actions.join(", "))
        }
    }

    fn integer_schema(&mut self, shape: &IntegerShape) -> Result<String, EmitError> {
        self.need(&["integer"]);
        let base = match shape.carrier {
            IntCarrier::Text => {
                self.need(&[if shape.signed {
                    "canonical-signed-integer"
                } else {
                    "canonical-integer"
                }]);
                format!(
                    "v.pipe(v.string(), v.regex({}))",
                    if shape.signed {
                        "CANONICAL_SIGNED_INTEGER"
                    } else {
                        "CANONICAL_INTEGER"
                    }
                )
            }
            IntCarrier::Bigint => "v.bigint()".into(),
            IntCarrier::Number => "v.pipe(v.number(), v.integer())".into(),
        };
        let mut arguments = vec![base, shape.bits.to_string(), shape.signed.to_string()];
        for value in [shape.minimum, shape.maximum, shape.multiple_of] {
            arguments.push(match value {
                Some(value) => string(&integral_constraint(value)?),
                None => String::from("undefined"),
            });
        }
        while arguments.last().map(String::as_str) == Some("undefined") {
            arguments.pop();
        }
        Ok(format!("integer({})", arguments.join(", ")))
    }
}

impl BackendTrait for Backend {
    fn name(&self) -> &'static str {
        "valibot"
    }

    fn reference(&mut self, name: &str, _variant: Option<&str>) -> Rendered {
        Rendered::exact(format!("v.lazy(() => {name}Schema)"))
    }

    fn schema(&self, name: &str) -> String {
        format!("{name}Schema")
    }

    fn object(&mut self, members: &[Member], openness: &ObjectShape) -> Rendered {
        self.need(&["plain-record"]);
        let members = members
            .iter()
            .map(|member| {
                let mut schema = member.schema.clone();
                if member.presence == Presence::Required && member.accepts_undefined {
                    schema = format!("v.nonOptional({schema})");
                } else if member.presence == Presence::Optional {
                    schema = format!("v.optional({schema})");
                }
                format!("{}: {schema}", string(&member.key))
            })
            .collect::<Vec<_>>()
            .join(", ");
        let schema = match openness {
            ObjectShape::Reject => format!("v.strictObject({{{members}}})"),
            ObjectShape::PreserveOpaque => format!("v.looseObject({{{members}}})"),
            ObjectShape::PreserveTyped(value) => {
                format!("v.objectWithRest({{{members}}}, {value})")
            }
        };
        Rendered::exact(format!("plainRecord({schema})"))
    }

    fn tagged_union(&mut self, tag: &str, members: &[String]) -> Rendered {
        Rendered::exact(format!(
            "v.variant({}, [{}])",
            string(tag),
            members.join(", ")
        ))
    }

    fn union(&mut self, members: &[String]) -> Rendered {
        Rendered::exact(format!("v.union([{}])", members.join(", ")))
    }

    fn literal(&mut self, value: &str) -> Rendered {
        Rendered::exact(format!("v.literal({})", string(value)))
    }

    fn external(&mut self, wire: &str, payload: Option<&str>) -> Rendered {
        self.need(&["one-key", "plain-record"]);
        Rendered::exact(match payload {
            Some(payload) => format!("oneKey({}, {payload})", string(wire)),
            None => format!(
                "v.union([v.literal({wire}), oneKey({wire}, v.unknown())])",
                wire = string(wire)
            ),
        })
    }

    fn string(&mut self, shape: &StringShape) -> Rendered {
        Rendered::exact(self.string_schema(shape))
    }

    fn integer(&mut self, shape: &IntegerShape) -> Result<Rendered, EmitError> {
        Ok(Rendered::exact(self.integer_schema(shape)?))
    }

    fn float(&mut self, bits: u8) -> Rendered {
        if bits == 32 {
            self.need(&["binary32"]);
            Rendered::exact("binary32()")
        } else {
            Rendered::exact(
                "v.pipe(v.number(), v.check((value) => Number.isFinite(value), \"number is not finite\"))",
            )
        }
    }

    fn decimal(&mut self, wire: DecimalWire) -> Rendered {
        match wire {
            DecimalWire::Number => Rendered::exact(
                "v.pipe(v.number(), v.check((value) => Number.isFinite(value), \"number is not finite\"))",
            ),
            DecimalWire::String => {
                self.need(&["decimal"]);
                Rendered::exact("v.pipe(v.string(), v.regex(DECIMAL, \"expected a decimal\"))")
            }
        }
    }

    fn bytes(&mut self) -> Rendered {
        Rendered::exact("v.instance(Uint8Array)")
    }

    fn boolean(&mut self) -> Rendered {
        Rendered::exact("v.boolean()")
    }

    fn array(&mut self, item: &str, unique: bool) -> Rendered {
        if unique {
            self.need(&["unique"]);
            Rendered::exact(format!("uniqueItems({item})"))
        } else {
            Rendered::exact(format!("v.array({item})"))
        }
    }

    fn tuple(&mut self, items: &[String]) -> Rendered {
        Rendered::exact(format!("v.strictTuple([{}])", items.join(", ")))
    }

    fn map(&mut self, key: &str, value: &str) -> Rendered {
        self.need(&["plain-record"]);
        Rendered::Widened(format!("plainRecord(v.record({key}, {value}))"), MAP)
    }

    fn nullable(&mut self, inner: &str) -> Rendered {
        Rendered::exact(format!("v.nullable({inner})"))
    }

    fn any(&mut self) -> Rendered {
        Rendered::exact("v.any()")
    }

    fn unknown(&mut self) -> Rendered {
        Rendered::exact("v.unknown()")
    }

    fn null(&mut self) -> Rendered {
        Rendered::exact("v.null()")
    }

    fn unit(&mut self) -> Rendered {
        Rendered::exact("v.null()")
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
                        "the valibot backend cannot declare {} without {dependency}",
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

#[cfg(test)]
mod tests {
    use pons_ir::FieldFormat;

    use super::*;

    fn rendered(rendered: Rendered) -> String {
        match rendered {
            Rendered::Exact(schema) => schema,
            Rendered::Widened(schema, _) => schema,
            Rendered::Unsupported(_) => panic!("valibot supports every shape"),
        }
    }

    #[test]
    fn a_string_shape_keeps_the_regex_and_the_code_point_bounds_in_order() {
        let mut backend = Backend::new();
        let schema = rendered(backend.string(&StringShape {
            format: None,
            carrier: StringCarrier::Text,
            min_length: Some(2),
            max_length: Some(4),
            pattern: Some("^[a-z]+$"),
        }));
        assert_eq!(
            schema,
            "v.pipe(v.string(), v.regex(new RegExp(\"^[a-z]+$\")), v.check((value) => [...value].length >= 2, \"string is shorter than the contract\"), v.check((value) => [...value].length <= 4, \"string is longer than the contract\"))"
        );
    }

    #[test]
    fn an_instant_becomes_a_date_only_under_the_date_carrier() {
        let mut backend = Backend::new();
        let text = rendered(backend.string(&StringShape {
            format: Some(FieldFormat::DateTime),
            carrier: StringCarrier::Text,
            min_length: None,
            max_length: None,
            pattern: None,
        }));
        assert_eq!(text, "instant()");
        let date = rendered(backend.string(&StringShape {
            format: Some(FieldFormat::DateTime),
            carrier: StringCarrier::Date,
            min_length: None,
            max_length: None,
            pattern: None,
        }));
        assert_eq!(date, "v.date()");
    }

    #[test]
    fn trailing_undefined_constraints_are_dropped_from_the_integer_call() {
        let mut backend = Backend::new();
        let minimum = serde_json::json!(3);
        let schema = rendered(
            backend
                .integer(&IntegerShape {
                    carrier: IntCarrier::Number,
                    bits: 16,
                    signed: false,
                    minimum: Some(&minimum),
                    maximum: None,
                    multiple_of: None,
                })
                .unwrap(),
        );
        assert_eq!(
            schema,
            "integer(v.pipe(v.number(), v.integer()), 16, false, \"3\")"
        );
    }
}
