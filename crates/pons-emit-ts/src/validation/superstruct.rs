use std::collections::{BTreeMap, BTreeSet};

use pons_ir::{DecimalWire, Document, FieldFormat, Presence, TypeRef, ValidationCoverage};

use super::backend::{
    Backend as BackendTrait, Capability, Declaration, DeclarationKind, IntCarrier, IntegerShape,
    Member, ObjectShape, Rendered, StringCarrier, StringShape,
};
use super::walk::Walk;
use super::{Capabilities, Generator as ValidationGenerator, Imports, Rendered as RenderedFile};
use crate::{EmitError, TsOptions, jsdoc, string};

const PRELUDE: &[(&str, &str)] = &[
    (
        "canonical-integer",
        r#"const CANONICAL_INTEGER = /^(0|[1-9][0-9]*)$/;
"#,
    ),
    (
        "canonical-signed-integer",
        r#"const CANONICAL_SIGNED_INTEGER = /^-?(0|[1-9][0-9]*)$/;
"#,
    ),
    (
        "decimal",
        r#"const DECIMAL = /^-?(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][+-]?\d+)?$/;
"#,
    ),
    (
        "format",
        r#"const CALENDAR_DATE = /^(\d{4})-(\d{2})-(\d{2})$/;
const CLOCK = /^(\d{2}):(\d{2}):(\d{2})(?:\.\d+)?(?:[Zz]|([+-])(\d{2}):(\d{2}))$/;
const INSTANT = /^(\d{4})-(\d{2})-(\d{2})[Tt](\d{2}):(\d{2}):(\d{2})(?:\.\d{1,3})?(?:[Zz]|([+-])(\d{2}):(\d{2}))$/;
const UUID = /^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$/;
const EMAIL = /^(?:[A-Za-z0-9_'+\-]+\.)*[A-Za-z0-9_'+\-]*[A-Za-z0-9_+-]@(?:[A-Za-z0-9][A-Za-z0-9\-]*\.)+[A-Za-z]{2,}$/;
const DURATION = /^P(?:(\d+W)|(?!.*W)(?=\d|T\d)(\d+Y)?(\d+M)?(\d+D)?(T(?=\d)(\d+H)?(\d+M)?(\d+([.,]\d+)?S)?)?)$/;
const IPV4 = /^(?:(?:25[0-5]|2[0-4][0-9]|1[0-9][0-9]|[1-9]?[0-9])\.){3}(?:25[0-5]|2[0-4][0-9]|1[0-9][0-9]|[1-9]?[0-9])$/;
const IPV6 = /^(([0-9a-fA-F]{1,4}:){7}[0-9a-fA-F]{1,4}|([0-9a-fA-F]{1,4}:){1,7}:|([0-9a-fA-F]{1,4}:){1,6}:[0-9a-fA-F]{1,4}|([0-9a-fA-F]{1,4}:){1,5}(:[0-9a-fA-F]{1,4}){1,2}|([0-9a-fA-F]{1,4}:){1,4}(:[0-9a-fA-F]{1,4}){1,3}|([0-9a-fA-F]{1,4}:){1,3}(:[0-9a-fA-F]{1,4}){1,4}|([0-9a-fA-F]{1,4}:){1,2}(:[0-9a-fA-F]{1,4}){1,5}|[0-9a-fA-F]{1,4}:((:[0-9a-fA-F]{1,4}){1,6})|:((:[0-9a-fA-F]{1,4}){1,7}|:))$/;

function daysInMonth(year: number, month: number): number {
  if (month === 2) {
    return year % 4 === 0 && (year % 100 !== 0 || year % 400 === 0) ? 29 : 28;
  }
  return [4, 6, 9, 11].includes(month) ? 30 : 31;
}

function isCalendarDay(year: string, month: string, day: string): boolean {
  const monthIndex = Number(month);
  const dayOfMonth = Number(day);
  if (monthIndex < 1 || monthIndex > 12 || dayOfMonth < 1) return false;
  return dayOfMonth <= daysInMonth(Number(year), monthIndex);
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

function matchesFormat(format: string, value: string): boolean {
  if (format === "uuid") return UUID.test(value);
  if (format === "date-time") {
    const match = INSTANT.exec(value);
    return match !== null && isCalendarDay(match[1] ?? "", match[2] ?? "", match[3] ?? "")
      && isClock(match[4] ?? "", match[5] ?? "", match[6] ?? "") && isOffset(match[8], match[9])
      && !Number.isNaN(Date.parse(value));
  }
  if (format === "date") {
    const match = CALENDAR_DATE.exec(value);
    return match !== null && isCalendarDay(match[1] ?? "", match[2] ?? "", match[3] ?? "");
  }
  if (format === "time") {
    const match = CLOCK.exec(value);
    return match !== null && isClock(match[1] ?? "", match[2] ?? "", match[3] ?? "") && isOffset(match[5], match[6]);
  }
  if (format === "duration") return DURATION.test(value);
  if (format === "uri") return isUri(value);
  if (format === "email") return EMAIL.test(value);
  if (format === "ipv4") return IPV4.test(value);
  if (format === "ipv6") return IPV6.test(value);
  return false;
}

function formatStruct(format: string): Struct<string, null> {
  return refine(string(), format, (value) => matchesFormat(format, value) || `expected a ${format}`);
}
"#,
    ),
    (
        "finite",
        r#"const finiteNumber = () => refine(number(), "finite", (value) => Number.isFinite(value) || "expected a finite number");
"#,
    ),
    (
        "binary32",
        r#"const binary32 = () => refine(number(), "binary32", (value) => Number.isFinite(Math.fround(value)) || "number is outside the f32 range");
"#,
    ),
    (
        "integer",
        r#"function exactInteger(bits: number, signed: boolean, minimum?: string, maximum?: string, multipleOf?: string) {
  const magnitude = 1n << BigInt(signed ? bits - 1 : bits);
  const low = signed ? -magnitude : 0n;
  const high = magnitude - 1n;
  return (value: unknown): true | string => {
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
    return true;
  };
}
"#,
    ),
    (
        "required",
        r#"const required = <T, S>(schema: Struct<T, S>) => refine(schema, "required", (value) => value !== undefined || "missing required value");
"#,
    ),
    (
        "one-key",
        r#"const oneKey = (key: string, payload: Struct<any, any>) => openType({ [key]: required(payload) });
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
const uniqueItems = <T, S>(item: Struct<T, S>) => refine(array(item), "unique", (values) => new Set(values.map(stableKey)).size === values.length || "array values must be unique");
"#,
    ),
];

const MISMATCH: &str = "function mismatch(schema: Struct<any, any>): (value: unknown) => ValidationIssue | null {\n  return (value) => {\n    const [error] = validate(value, schema);\n    if (error === undefined) return null;\n    const path = error.path.map((step) => (typeof step === \"number\" ? step : String(step)));\n    return { path, message: error.message };\n  };\n}\n";

const VALUE_IMPORTS: &[&str] = &[
    "any", "array", "bigint", "boolean", "date", "instance", "lazy", "literal", "nullable",
    "number", "object", "optional", "record", "refine", "string", "tuple", "union", "unknown",
    "validate",
];

pub(crate) struct Backend {
    need: BTreeSet<&'static str>,
    uses: BTreeSet<&'static str>,
}

impl Backend {
    pub(crate) fn new() -> Self {
        Backend {
            need: BTreeSet::new(),
            uses: BTreeSet::new(),
        }
    }

    fn need(&mut self, entry: &'static str) {
        self.need.insert(entry);
    }

    fn need_prelude(&mut self, entry: &'static str) {
        self.need.insert(entry);
        for name in prelude_imports(entry) {
            self.uses.insert(name);
        }
    }

    fn used(&mut self, name: &'static str) {
        self.uses.insert(name);
    }

    fn import_list(&self) -> String {
        let mut specifiers = VALUE_IMPORTS
            .iter()
            .copied()
            .filter(|name| self.uses.contains(name))
            .collect::<Vec<_>>();
        if self.uses.contains("openType") {
            specifiers.push("type as openType");
        }
        if self.uses.contains("Struct") {
            specifiers.push("type Struct");
        }
        specifiers.join(", ")
    }
}

impl BackendTrait for Backend {
    fn name(&self) -> &'static str {
        "superstruct"
    }

    fn reference(&mut self, name: &str, _variant: Option<&str>) -> Rendered {
        self.used("lazy");
        Rendered::exact(format!("lazy(() => {name}Schema)"))
    }

    fn schema(&self, name: &str) -> String {
        format!("{name}Schema")
    }

    fn object(&mut self, members: &[Member], openness: &ObjectShape) -> Rendered {
        let members = members
            .iter()
            .map(|member| {
                let schema = if member.presence == Presence::Required && member.accepts_undefined {
                    self.need_prelude("required");
                    format!("required({})", member.schema)
                } else if member.presence == Presence::Optional {
                    self.used("optional");
                    format!("optional({})", member.schema)
                } else {
                    member.schema.clone()
                };
                format!("{}: {schema}", string(&member.key))
            })
            .collect::<Vec<_>>()
            .join(", ");
        let schema = match openness {
            ObjectShape::Reject => {
                self.used("object");
                format!("object({{{members}}})")
            }
            ObjectShape::PreserveOpaque | ObjectShape::PreserveTyped(_) => {
                self.used("openType");
                format!("openType({{{members}}})")
            }
        };
        match openness {
            ObjectShape::PreserveTyped(_) => Rendered::Widened(
                schema,
                Capability::new("validation.openness.preserve_typed"),
            ),
            _ => Rendered::exact(schema),
        }
    }

    fn union(&mut self, members: &[String]) -> Rendered {
        self.used("union");
        Rendered::exact(format!("union([{}])", members.join(", ")))
    }

    fn literal(&mut self, value: &str) -> Rendered {
        self.used("literal");
        Rendered::exact(format!("literal({})", string(value)))
    }

    fn external(&mut self, wire: &str, payload: Option<&str>) -> Rendered {
        self.need_prelude("one-key");
        self.need_prelude("required");
        match payload {
            Some(payload) => Rendered::exact(format!("oneKey({}, {payload})", string(wire))),
            None => {
                self.used("union");
                self.used("literal");
                self.used("unknown");
                Rendered::exact(format!(
                    "union([literal({wire}), oneKey({wire}, unknown())])",
                    wire = string(wire)
                ))
            }
        }
    }

    fn string(&mut self, shape: &StringShape) -> Rendered {
        if shape.carrier == StringCarrier::Date {
            self.used("date");
            return Rendered::exact("date()");
        }
        self.used("string");
        let mut schema = match shape.format {
            None => "string()".to_owned(),
            Some(format) => {
                self.need_prelude("format");
                format!("formatStruct({})", string(format_name(format)))
            }
        };
        if let Some(pattern) = shape.pattern {
            self.used("refine");
            schema = format!(
                "refine({schema}, \"pattern\", (value) => new RegExp({}).test(value) || \"string does not match the contract\")",
                string(pattern)
            );
        }
        if let Some(minimum) = shape.min_length {
            self.used("refine");
            schema = format!(
                "refine({schema}, \"minLength\", (value) => [...value].length >= {minimum} || \"string is shorter than the contract\")"
            );
        }
        if let Some(maximum) = shape.max_length {
            self.used("refine");
            schema = format!(
                "refine({schema}, \"maxLength\", (value) => [...value].length <= {maximum} || \"string is longer than the contract\")"
            );
        }
        Rendered::exact(schema)
    }

    fn integer(&mut self, shape: &IntegerShape) -> Rendered {
        self.need_prelude("integer");
        let base = match shape.carrier {
            IntCarrier::Text => {
                self.used("string");
                self.used("refine");
                let canonical = if shape.signed {
                    self.need("canonical-signed-integer");
                    "CANONICAL_SIGNED_INTEGER"
                } else {
                    self.need("canonical-integer");
                    "CANONICAL_INTEGER"
                };
                format!(
                    "refine(string(), \"integer\", (value) => {canonical}.test(value) || \"expected an integer string\")"
                )
            }
            IntCarrier::Bigint => {
                self.used("bigint");
                "bigint()".to_owned()
            }
            IntCarrier::Number => {
                self.used("number");
                self.used("refine");
                "refine(number(), \"integer\", (value) => Number.isInteger(value) || \"expected an exact integer\")"
                    .to_owned()
            }
        };
        let mut arguments = vec![shape.bits.to_string(), shape.signed.to_string()];
        for value in [shape.minimum, shape.maximum, shape.multiple_of] {
            arguments.push(match value {
                Some(value) => string(&value.to_string()),
                None => String::from("undefined"),
            });
        }
        while arguments.last().map(String::as_str) == Some("undefined") {
            arguments.pop();
        }
        Rendered::exact(format!(
            "refine({base}, \"integer\", exactInteger({}))",
            arguments.join(", ")
        ))
    }

    fn float(&mut self, bits: u8) -> Rendered {
        if bits == 32 {
            self.need_prelude("binary32");
            Rendered::exact("binary32()")
        } else {
            self.need_prelude("finite");
            Rendered::exact("finiteNumber()")
        }
    }

    fn decimal(&mut self, wire: DecimalWire) -> Rendered {
        match wire {
            DecimalWire::Number => {
                self.need_prelude("finite");
                Rendered::exact("finiteNumber()")
            }
            DecimalWire::String => {
                self.need("decimal");
                self.used("string");
                self.used("refine");
                Rendered::exact(
                    "refine(string(), \"decimal\", (value) => DECIMAL.test(value) || \"expected decimal\")",
                )
            }
        }
    }

    fn bytes(&mut self) -> Rendered {
        self.used("instance");
        Rendered::exact("instance(Uint8Array)")
    }

    fn boolean(&mut self) -> Rendered {
        self.used("boolean");
        Rendered::exact("boolean()")
    }

    fn array(&mut self, item: &str, unique: bool) -> Rendered {
        if unique {
            self.need_prelude("unique");
            Rendered::exact(format!("uniqueItems({item})"))
        } else {
            self.used("array");
            Rendered::exact(format!("array({item})"))
        }
    }

    fn tuple(&mut self, items: &[String]) -> Rendered {
        self.used("tuple");
        Rendered::exact(format!("tuple([{}])", items.join(", ")))
    }

    fn map(&mut self, key: &str, value: &str) -> Rendered {
        self.used("record");
        Rendered::exact(format!("record({key}, {value})"))
    }

    fn nullable(&mut self, inner: &str) -> Rendered {
        self.used("nullable");
        Rendered::exact(format!("nullable({inner})"))
    }

    fn any(&mut self) -> Rendered {
        self.used("any");
        Rendered::exact("any()")
    }

    fn unknown(&mut self) -> Rendered {
        self.used("unknown");
        Rendered::exact("unknown()")
    }

    fn null(&mut self) -> Rendered {
        self.used("literal");
        Rendered::exact("literal(null)")
    }

    fn unit(&mut self) -> Rendered {
        self.used("literal");
        Rendered::exact("literal(null)")
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
                        "the superstruct backend cannot declare {} without {dependency}",
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
        self.backend.used("validate");
        self.backend.used("Struct");
        let specifier = crate::runtime::specifier(self.options.runtime, "\"./runtime.js\"");
        let package = self
            .options
            .validation
            .import_from
            .as_deref()
            .unwrap_or("superstruct");
        Ok(RenderedFile {
            file: "validators.ts",
            content: format!(
                "import {{ {} }} from {};\n\
                 import type {{ ValidationIssue, Validator }} from {specifier};\n\
                 export type {{ ValidationIssue, Validator }} from {specifier};\n\n{body}",
                self.backend.import_list(),
                crate::string(package)
            ),
        })
    }
}

fn format_name(format: FieldFormat) -> &'static str {
    match format {
        FieldFormat::Uuid => "uuid",
        FieldFormat::DateTime => "date-time",
        FieldFormat::Date => "date",
        FieldFormat::Time => "time",
        FieldFormat::Duration => "duration",
        FieldFormat::Uri => "uri",
        FieldFormat::Email => "email",
        FieldFormat::Ipv4 => "ipv4",
        FieldFormat::Ipv6 => "ipv6",
    }
}

fn prelude_imports(entry: &str) -> &'static [&'static str] {
    match entry {
        "format" => &["refine", "string", "Struct"],
        "finite" | "binary32" => &["number", "refine"],
        "required" => &["refine", "Struct"],
        "one-key" => &["openType"],
        "unique" => &["array", "refine", "Struct"],
        _ => &[],
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn rendered(rendered: Rendered) -> String {
        match rendered {
            Rendered::Exact(schema) | Rendered::Widened(schema, _) => schema,
            Rendered::Unsupported(_) => panic!("superstruct supports every shape"),
        }
    }

    #[test]
    fn preserve_typed_is_the_one_openness_policy_without_an_exact_form() {
        let mut generator = Backend::new();
        let rendered = generator.object(
            &[Member {
                key: "a".into(),
                schema: "string()".into(),
                presence: Presence::Required,
                accepts_undefined: false,
            }],
            &ObjectShape::PreserveTyped("number()".into()),
        );
        match rendered {
            Rendered::Widened(schema, capability) => {
                assert_eq!(schema, "openType({\"a\": string()})");
                assert_eq!(capability.id(), "validation.openness.preserve_typed");
            }
            _ => panic!("a typed catchall has no superstruct equivalent"),
        }
    }

    #[test]
    fn an_integer_carrier_keeps_its_bounds_and_multiple() {
        let mut generator = Backend::new();
        let minimum = json!(3);
        let schema = rendered(generator.integer(&IntegerShape {
            carrier: IntCarrier::Number,
            bits: 16,
            signed: false,
            minimum: Some(&minimum),
            maximum: None,
            multiple_of: None,
        }));
        assert_eq!(
            schema,
            "refine(refine(number(), \"integer\", (value) => Number.isInteger(value) || \"expected an exact integer\"), \"integer\", exactInteger(16, false, \"3\"))"
        );
    }

    #[test]
    fn a_string_length_is_counted_in_code_points() {
        let mut generator = Backend::new();
        let schema = rendered(generator.string(&StringShape {
            format: None,
            carrier: StringCarrier::Text,
            min_length: Some(2),
            max_length: None,
            pattern: Some("^[a-z]+$"),
        }));
        assert_eq!(
            schema,
            "refine(refine(string(), \"pattern\", (value) => new RegExp(\"^[a-z]+$\").test(value) || \"string does not match the contract\"), \"minLength\", (value) => [...value].length >= 2 || \"string is shorter than the contract\")"
        );
    }
}
