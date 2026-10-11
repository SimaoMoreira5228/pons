use std::collections::{BTreeMap, BTreeSet};

use pons_ir::{DecimalWire, Document, FieldFormat, Presence, TypeRef, ValidationCoverage};

use super::backend::{
    Backend as ValidationBackend, Capability, Declaration, IntCarrier, IntegerShape, Member,
    ObjectShape, Rendered, StringCarrier, StringShape, integral_constraint,
};
use super::walk::Walk;
use super::{Capabilities, Generator as ValidationGenerator, Imports, Rendered as RenderedFile};
use crate::{EmitError, TsOptions, jsdoc, string};

const MISMATCH: &str = "function mismatch(schema: (value: unknown) => unknown): (value: unknown) => ValidationIssue | null {\n  return (value) => {\n    try {\n      const result = schema(value);\n      if (!(result instanceof ark.errors)) return null;\n      const error = result[0];\n      if (error === undefined) return { path: [], message: result.summary };\n      return { path: error.path.map((step) => typeof step === \"number\" ? step : String(step)), message: result.summary };\n    } catch (cause) {\n      return { path: [], message: cause instanceof Error ? cause.message : String(cause) };\n    }\n  };\n}\n";

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
        "duration",
        "const DURATION = /^P(?:(\\d+W)|(?!.*W)(?=\\d|T\\d)(\\d+Y)?(\\d+M)?(\\d+D)?(T(?=\\d)(\\d+H)?(\\d+M)?(\\d+([.,]\\d+)?S)?)?)$/;\n",
    ),
    (
        "uuid",
        "const UUID = /^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$/;\n",
    ),
    (
        "email",
        "const EMAIL = /^(?:[A-Za-z0-9_'+\\-]+\\.)*[A-Za-z0-9_'+\\-]*[A-Za-z0-9_+-]@(?:[A-Za-z0-9][A-Za-z0-9\\-]*\\.)+[A-Za-z]{2,}$/;\n",
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
        "calendar",
        "function isCalendarDay(year: string, month: string, day: string): boolean {\n  const value = Number(year);\n  const monthIndex = Number(month);\n  const dayOfMonth = Number(day);\n  if (monthIndex < 1 || monthIndex > 12 || dayOfMonth < 1) return false;\n  return dayOfMonth <= daysInMonth(value, monthIndex);\n}\nfunction daysInMonth(year: number, month: number): number {\n  if (month === 2) return year % 4 === 0 && (year % 100 !== 0 || year % 400 === 0) ? 29 : 28;\n  return [4, 6, 9, 11].includes(month) ? 30 : 31;\n}\nfunction isClock(hour: string, minute: string, second: string): boolean {\n  return Number(hour) <= 23 && Number(minute) <= 59 && Number(second) <= 59;\n}\nfunction isOffset(hour: string | undefined, minute: string | undefined): boolean {\n  return hour === undefined || (Number(hour) <= 23 && Number(minute) <= 59);\n}\n",
    ),
    (
        "instant",
        "const instant = () => ark(DATE_TIME).narrow((value) => {\n  const match = DATE_TIME.exec(value);\n  return match !== null && isCalendarDay(match[1]!, match[2]!, match[3]!)\n    && isClock(match[4]!, match[5]!, match[6]!) && isOffset(match[8], match[9])\n    && !Number.isNaN(Date.parse(value));\n});\n",
    ),
    (
        "day",
        "const day = () => ark(DATE).narrow((value) => {\n  const match = DATE.exec(value);\n  return match !== null && isCalendarDay(match[1]!, match[2]!, match[3]!);\n});\n",
    ),
    (
        "clock",
        "const clock = () => ark(TIME).narrow((value) => {\n  const match = TIME.exec(value);\n  return match !== null && isClock(match[1]!, match[2]!, match[3]!) && isOffset(match[5], match[6]);\n});\n",
    ),
    (
        "uri",
        "const uri = () => ark(\"string\").narrow((value) => {\n  if (/\\s/.test(value)) return false;\n  try {\n    return new URL(value).protocol.length > 1;\n  } catch {\n    return false;\n  }\n});\n",
    ),
    (
        "finite",
        "const finite = () => ark(\"number\").narrow((value) => Number.isFinite(value));\n",
    ),
    (
        "binary32",
        "const binary32 = () => ark(\"number\").narrow((value) => Number.isFinite(Math.fround(value)));\n",
    ),
    (
        "plain-record",
        "const plainRecord = ark(\"object\").narrow((value) => {\n  if (value === null || typeof value !== \"object\" || Array.isArray(value)) return false;\n  const prototype = Object.getPrototypeOf(value);\n  return prototype === Object.prototype || prototype === null;\n});\n",
    ),
    (
        "present",
        "const present = ark(\"unknown\").narrow((value) => value !== undefined);\n",
    ),
    (
        "stable-key",
        "function stableKey(value: unknown): string {\n  if (typeof value === \"bigint\") return `bigint:${value}`;\n  if (value instanceof Date) return `date:${value.getTime()}`;\n  if (value instanceof Uint8Array) return `bytes:${Array.from(value).join(\",\")}`;\n  if (Array.isArray(value)) return `[${value.map(stableKey).join(\",\")}]`;\n  if (value !== null && typeof value === \"object\") {\n    const record = value as Record<string, unknown>;\n    return `{${Object.keys(record).sort().map((key) => `${JSON.stringify(key)}:${stableKey(record[key])}`).join(\",\")}}`;\n  }\n  return JSON.stringify(value) ?? \"undefined\";\n}\n",
    ),
    (
        "unique",
        "const unique = ark(\"unknown[]\").narrow((values) => new Set(values.map(stableKey)).size === values.length);\n",
    ),
    (
        "integer",
        "function exactInteger(bits: number, signed: boolean, minimum?: string, maximum?: string, multipleOf?: string) {\n  const magnitude = 1n << BigInt(signed ? bits - 1 : bits);\n  const low = signed ? -magnitude : 0n;\n  const high = magnitude - 1n;\n  const constraint = (text: string | undefined) => text === undefined ? undefined : /^-?\\d+$/.test(text) ? BigInt(text) : null;\n  const min = constraint(minimum);\n  const max = constraint(maximum);\n  const step = constraint(multipleOf);\n  return (integer: bigint) => {\n    if (integer < low || integer > high) return false;\n    if (min === null || max === null || step === null) return false;\n    if (min !== undefined && integer < min) return false;\n    if (max !== undefined && integer > max) return false;\n    if (step !== undefined && (step <= 0n || integer % step !== 0n)) return false;\n    return true;\n  };\n}\nfunction integerText(bits: number, signed: boolean, minimum?: string, maximum?: string, multipleOf?: string) {\n  const check = exactInteger(bits, signed, minimum, maximum, multipleOf);\n  return ark(signed ? CANONICAL_SIGNED_INTEGER : CANONICAL_INTEGER).narrow((value) => check(BigInt(value)));\n}\nfunction integerBigint(bits: number, signed: boolean, minimum?: string, maximum?: string, multipleOf?: string) {\n  const check = exactInteger(bits, signed, minimum, maximum, multipleOf);\n  return ark(\"bigint\").narrow((value) => check(value));\n}\nfunction integerNumber(bits: number, signed: boolean, minimum?: string, maximum?: string, multipleOf?: string) {\n  const check = exactInteger(bits, signed, minimum, maximum, multipleOf);\n  return ark(\"number\").narrow((value) => Number.isInteger(value) && check(BigInt(value)));\n}\n",
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
        let bindings = self.walk.bindings()?;
        for binding in &bindings {
            if !self.walk.declares(&binding.schema) {
                self.backend.bind(&binding.name, &binding.schema);
            }
        }
        let mut body = self.walk.module(&mut self.backend)?;
        body.push_str(MISMATCH);
        for binding in &bindings {
            let reference = if self.walk.declares(&binding.schema) {
                &binding.schema
            } else {
                &binding.name
            };
            body.push_str(&format!(
                "export const {}: Validator = {};\n",
                binding.name,
                self.backend.validator(&format!("$.{reference}"))
            ));
        }
        let specifier = crate::runtime::specifier(self.options.runtime, "\"./runtime.js\"");
        let package = self
            .options
            .validation
            .import_from
            .as_deref()
            .unwrap_or("arktype");
        Ok(RenderedFile {
            file: "validators.ts",
            content: format!(
                "import {{ scope, type as ark }} from {};\n\
                 import type {{ ValidationIssue, Validator }} from {specifier};\n\
                 export type {{ ValidationIssue, Validator }} from {specifier};\n\n{body}",
                string(package)
            ),
        })
    }
}

struct Backend {
    need: BTreeSet<&'static str>,
    aliases: BTreeMap<String, String>,
}

impl Backend {
    fn new() -> Self {
        Backend {
            need: BTreeSet::new(),
            aliases: BTreeMap::new(),
        }
    }

    fn need(&mut self, id: &'static str) {
        self.need.insert(id);
    }

    fn bind(&mut self, name: &str, schema: &str) {
        self.aliases.insert(name.to_owned(), schema.to_owned());
    }

    fn object_expression(
        &mut self,
        members: &[Member],
        index: Option<&str>,
        reject: bool,
    ) -> String {
        let mut entries = Vec::new();
        if reject {
            entries.push(format!("{}: {}", string("+"), string("reject")));
        }
        for member in members {
            let schema = if member.presence == Presence::Optional {
                format!("[{}, \"|\", {}]", member.schema, definition("undefined"))
            } else if member.accepts_undefined {
                self.need("present");
                format!("[{}, \"&\", present]", member.schema)
            } else {
                member.schema.clone()
            };
            let key = match member.presence {
                Presence::Optional => format!("{}?", member.key),
                Presence::Required => member.key.clone(),
            };
            entries.push(format!("{}: {schema}", string(&key)));
        }
        if let Some(index) = index {
            entries.push(format!("{}: {index}", string("[string]")));
        }
        self.need("plain-record");
        format!("[{{ {} }}, \"&\", plainRecord]", entries.join(", "))
    }

    fn preserve_typed_exact(&self, members: &[Member], extra: &str) -> bool {
        if extra == definition("unknown") {
            return true;
        }
        members.iter().all(|member| {
            member.presence == Presence::Required
                && !member.accepts_undefined
                && member.schema == extra
        })
    }
}

impl ValidationBackend for Backend {
    fn name(&self) -> &'static str {
        "arktype"
    }

    fn reference(&mut self, name: &str, _variant: Option<&str>) -> Rendered {
        Rendered::exact(definition(name))
    }

    fn schema(&self, name: &str) -> String {
        name.to_owned()
    }

    fn object(&mut self, members: &[Member], openness: &ObjectShape) -> Rendered {
        match openness {
            ObjectShape::Reject => Rendered::exact(self.object_expression(members, None, true)),
            ObjectShape::PreserveOpaque => {
                Rendered::exact(self.object_expression(members, None, false))
            }
            ObjectShape::PreserveTyped(extra) => {
                if self.preserve_typed_exact(members, extra) {
                    Rendered::exact(self.object_expression(members, Some(extra), false))
                } else {
                    Rendered::Widened(
                        self.object_expression(members, None, false),
                        Capability::new("validation.openness.preserve_typed"),
                    )
                }
            }
        }
    }

    fn union(&mut self, members: &[String]) -> Rendered {
        Rendered::exact(definition(&members.join(" | ")))
    }

    fn literal(&mut self, value: &str) -> Rendered {
        Rendered::exact(literal(value))
    }

    fn external(&mut self, wire: &str, payload: Option<&str>) -> Rendered {
        let key = string(wire);
        let reject = format!("{}: {}", string("+"), string("reject"));
        match payload {
            Some(payload) => Rendered::exact(format!("{{ {key}: {payload}, {reject} }}")),
            None => Rendered::exact(format!(
                "[{}, \"|\", {{ {key}: {}, {reject} }}]",
                literal(wire),
                definition("unknown")
            )),
        }
    }

    fn string(&mut self, shape: &StringShape) -> Rendered {
        if shape.carrier == StringCarrier::Date {
            return Rendered::exact(
                "ark(\"Date\").narrow((value) => Number.isFinite(value.getTime()))",
            );
        }
        let mut conditions = Vec::new();
        let base = match shape.format {
            None => "ark(\"string\")".to_owned(),
            Some(FieldFormat::DateTime) => {
                self.need("date-time");
                self.need("calendar");
                self.need("instant");
                "instant()".to_owned()
            }
            Some(FieldFormat::Date) => {
                self.need("date");
                self.need("calendar");
                self.need("day");
                "day()".to_owned()
            }
            Some(FieldFormat::Time) => {
                self.need("time");
                self.need("calendar");
                self.need("clock");
                "clock()".to_owned()
            }
            Some(FieldFormat::Duration) => {
                self.need("duration");
                "ark(DURATION)".to_owned()
            }
            Some(FieldFormat::Uuid) => {
                self.need("uuid");
                "ark(UUID)".to_owned()
            }
            Some(FieldFormat::Uri) => {
                self.need("uri");
                "uri()".to_owned()
            }
            Some(FieldFormat::Email) => {
                self.need("email");
                "ark(EMAIL)".to_owned()
            }
            Some(FieldFormat::Ipv4) => {
                self.need("ipv4");
                "ark(IPV4)".to_owned()
            }
            Some(FieldFormat::Ipv6) => {
                self.need("ipv6");
                "ark(IPV6)".to_owned()
            }
        };
        if shape.format.is_none()
            && shape.pattern.is_none()
            && shape.min_length.is_none()
            && shape.max_length.is_none()
        {
            return Rendered::exact(definition("string"));
        }
        if let Some(pattern) = shape.pattern {
            conditions.push(format!("new RegExp({}).test(value)", string(pattern)));
        }
        if let Some(minimum) = shape.min_length {
            conditions.push(format!("[...value].length >= {minimum}"));
        }
        if let Some(maximum) = shape.max_length {
            conditions.push(format!("[...value].length <= {maximum}"));
        }
        if conditions.is_empty() {
            Rendered::exact(base)
        } else {
            Rendered::exact(format!(
                "{base}.narrow((value) => {})",
                conditions.join(" && ")
            ))
        }
    }

    fn integer(&mut self, shape: &IntegerShape) -> Result<Rendered, EmitError> {
        match shape.carrier {
            IntCarrier::Number if shape.bits <= 32 => Ok(Rendered::exact(native_integer(shape)?)),
            IntCarrier::Text => {
                self.need("integer");
                self.need("canonical-integer");
                self.need("canonical-signed-integer");
                Ok(Rendered::exact(integer_call("integerText", shape)?))
            }
            IntCarrier::Bigint => {
                self.need("integer");
                Ok(Rendered::exact(integer_call("integerBigint", shape)?))
            }
            IntCarrier::Number => {
                self.need("integer");
                Ok(Rendered::exact(integer_call("integerNumber", shape)?))
            }
        }
    }

    fn float(&mut self, bits: u8) -> Rendered {
        if bits == 32 {
            self.need("binary32");
            Rendered::exact("binary32()")
        } else {
            self.need("finite");
            Rendered::exact("finite()")
        }
    }

    fn decimal(&mut self, wire: DecimalWire) -> Rendered {
        match wire {
            DecimalWire::Number => {
                self.need("finite");
                Rendered::exact("finite()")
            }
            DecimalWire::String => {
                self.need("decimal");
                Rendered::exact("ark(DECIMAL)")
            }
        }
    }

    fn bytes(&mut self) -> Rendered {
        Rendered::exact("ark.instanceOf(Uint8Array)")
    }

    fn boolean(&mut self) -> Rendered {
        Rendered::exact(definition("boolean"))
    }

    fn array(&mut self, item: &str, unique: bool) -> Rendered {
        if unique {
            self.need("stable-key");
            self.need("unique");
            Rendered::exact(format!("[[{item}, {}], \"&\", unique]", string("[]")))
        } else {
            Rendered::exact(format!("[{item}, {}]", string("[]")))
        }
    }

    fn tuple(&mut self, items: &[String]) -> Rendered {
        Rendered::exact(format!("[{}]", items.join(", ")))
    }

    fn map(&mut self, key: &str, value: &str) -> Rendered {
        let expression = self.object_expression(
            &[Member {
                key: "[string]".into(),
                schema: value.to_owned(),
                presence: Presence::Required,
                accepts_undefined: false,
            }],
            None,
            false,
        );
        if key == definition("string") {
            Rendered::exact(expression)
        } else {
            Rendered::Widened(expression, Capability::new("validation.map"))
        }
    }

    fn nullable(&mut self, inner: &str) -> Rendered {
        Rendered::exact(format!("[{inner}, \"|\", {}]", definition("null")))
    }

    fn any(&mut self) -> Rendered {
        Rendered::exact(definition("unknown"))
    }

    fn unknown(&mut self) -> Rendered {
        Rendered::exact(definition("unknown"))
    }

    fn null(&mut self) -> Rendered {
        Rendered::exact(definition("null"))
    }

    fn unit(&mut self) -> Rendered {
        Rendered::exact(definition("null"))
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
                        "the arktype backend cannot declare {} without {dependency}",
                        declaration.name
                    )));
                }
            }
        }
        let mut output = String::new();
        for (id, code) in PRELUDE {
            if self.need.contains(id) {
                output.push_str(code);
            }
        }
        if output.ends_with('\n') {
            output.push('\n');
        }
        let mut entries = String::new();
        for declaration in declarations {
            let comment = jsdoc(declaration.doc);
            if !comment.is_empty() {
                entries.push_str(&comment);
                entries.push('\n');
            }
            entries.push_str(&format!(
                "  {}: {},\n",
                declaration.name, declaration.schema
            ));
        }
        for (name, schema) in &self.aliases {
            entries.push_str(&format!("  {name}: {schema},\n"));
        }
        output.push_str(&format!("const $ = scope({{\n{entries}}}).export();\n"));
        Ok(output)
    }
}

fn definition(text: &str) -> String {
    string(text)
}

fn literal(value: &str) -> String {
    string(&format!(
        "'{}'",
        value.replace('\\', "\\\\").replace('\'', "\\'")
    ))
}

fn integer_call(name: &str, shape: &IntegerShape) -> Result<String, EmitError> {
    let mut arguments = vec![shape.bits.to_string(), shape.signed.to_string()];
    for value in [shape.minimum, shape.maximum, shape.multiple_of] {
        arguments.push(match value {
            Some(value) => string(&integral_constraint(value)?),
            None => String::from("undefined"),
        });
    }
    while arguments.last().map(String::as_str) == Some("undefined") {
        arguments.pop();
    }
    Ok(format!("{name}({})", arguments.join(", ")))
}

fn native_integer(shape: &IntegerShape) -> Result<String, EmitError> {
    let magnitude = 1i128
        << (if shape.signed {
            shape.bits - 1
        } else {
            shape.bits
        });
    let low = if shape.signed { -magnitude } else { 0 };
    let high = magnitude - 1;
    let mut bounds = vec![
        format!("number.integer >= {low}"),
        format!("number.integer <= {high}"),
    ];
    for (operator, value) in [
        (">=", shape.minimum),
        ("<=", shape.maximum),
        ("%", shape.multiple_of),
    ] {
        if let Some(value) = value {
            bounds.push(format!(
                "number.integer {operator} {}",
                integral_constraint(value)?
            ));
        }
    }
    let members = bounds
        .iter()
        .map(|bound| definition(bound))
        .collect::<Vec<_>>();
    let mut expression = members[0].clone();
    for member in members.iter().skip(1) {
        expression = format!("[{expression}, \"&\", {member}]");
    }
    Ok(expression)
}
