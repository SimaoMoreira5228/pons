use std::collections::BTreeSet;

use pons_ir::{Document, TypeRef, ValidationCoverage};

use super::backend::Backend as _;
use super::walk::Walk;
use super::{Capabilities, Generator as ValidationGenerator, Imports, Rendered as RenderedFile};
use crate::render::string;
use crate::{EmitError, TsOptions};

const REFERENCE: char = '\u{1}';

const CANONICAL_INTEGER: &str = "^(0|[1-9][0-9]*)$";
const CANONICAL_SIGNED_INTEGER: &str = "^-?(0|[1-9][0-9]*)$";
const DECIMAL: &str = r"^-?(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][+-]?\d+)?$";

const PRELUDE: &[(&str, &str)] = &[
    (
        "integer",
        "function exactInteger(bits: number, signed: boolean, minimum?: string, maximum?: string, multipleOf?: string) {\n  const magnitude = 1n << BigInt(signed ? bits - 1 : bits);\n  const low = signed ? -magnitude : 0n;\n  const high = magnitude - 1n;\n  return (value: unknown): string | undefined => {\n    let integer: bigint;\n    try {\n      integer = BigInt(value as bigint | number | string);\n    } catch {\n      return \"expected an exact integer\";\n    }\n    if (integer < low || integer > high) return \"integer is outside its native range\";\n    if (minimum !== undefined && integer < BigInt(minimum)) return \"integer is below its minimum\";\n    if (maximum !== undefined && integer > BigInt(maximum)) return \"integer is above its maximum\";\n    if (multipleOf !== undefined && (BigInt(multipleOf) <= 0n || integer % BigInt(multipleOf) !== 0n)) return \"integer does not match its multiple\";\n    return undefined;\n  };\n}\nfunction integer(base: Type.TSchema, bits: number, signed: boolean, minimum?: string, maximum?: string, multipleOf?: string): Type.TSchema {\n  const fault = exactInteger(bits, signed, minimum, maximum, multipleOf);\n  return Type.Refine(base, (value) => fault(value) === undefined, (value) => fault(value) ?? \"expected an exact integer\");\n}\n",
    ),
    (
        "formats",
        r#"const DATE = /^(\d{4})-(\d{2})-(\d{2})$/;
const TIME = /^(\d{2}):(\d{2}):(\d{2})(?:\.\d+)?(?:[Zz]|([+-])(\d{2}):(\d{2}))$/;
const DATE_TIME = /^(\d{4})-(\d{2})-(\d{2})[Tt](\d{2}):(\d{2}):(\d{2})(?:\.\d{1,3})?(?:[Zz]|([+-])(\d{2}):(\d{2}))$/;
const UUID = /^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$/;
const EMAIL = /^(?:[A-Za-z0-9_'+\-]+\.)*[A-Za-z0-9_'+\-]*[A-Za-z0-9_+-]@(?:[A-Za-z0-9][A-Za-z0-9\-]*\.)+[A-Za-z]{2,}$/;
const DURATION = /^P(?:(\d+W)|(?!.*W)(?=\d|T\d)(\d+Y)?(\d+M)?(\d+D)?(T(?=\d)(\d+H)?(\d+M)?(\d+([.,]\d+)?S)?)?)$/;
const IPV4 = /^(?:(?:25[0-5]|2[0-4][0-9]|1[0-9][0-9]|[1-9]?[0-9])\.){3}(?:25[0-5]|2[0-4][0-9]|1[0-9][0-9]|[1-9]?[0-9])$/;
const IPV6 = /^(([0-9a-fA-F]{1,4}:){7}[0-9a-fA-F]{1,4}|([0-9a-fA-F]{1,4}:){1,7}:|([0-9a-fA-F]{1,4}:){1,6}:[0-9a-fA-F]{1,4}|([0-9a-fA-F]{1,4}:){1,5}(:[0-9a-fA-F]{1,4}){1,2}|([0-9a-fA-F]{1,4}:){1,4}(:[0-9a-fA-F]{1,4}){1,3}|([0-9a-fA-F]{1,4}:){1,3}(:[0-9a-fA-F]{1,4}){1,4}|([0-9a-fA-F]{1,4}:){1,2}(:[0-9a-fA-F]{1,4}){1,5}|[0-9a-fA-F]{1,4}:((:[0-9a-fA-F]{1,4}){1,6})|:((:[0-9a-fA-F]{1,4}){1,7}|:))$/;
function isCalendarDay(year: string, month: string, day: string): boolean {
  const value = Number(year);
  const monthIndex = Number(month);
  const dayOfMonth = Number(day);
  if (monthIndex < 1 || monthIndex > 12 || dayOfMonth < 1) return false;
  return dayOfMonth <= daysInMonth(value, monthIndex);
}
function daysInMonth(year: number, month: number): number {
  if (month === 2) return year % 4 === 0 && (year % 100 !== 0 || year % 400 === 0) ? 29 : 28;
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
function format(schema: Type.TSchema, name: string): Type.TSchema {
  return Type.Refine(schema, (value) => matchesFormat(name, value as string), () => formatMessage(name));
}
"#,
    ),
    (
        "bounded",
        "function bounded(schema: Type.TSchema, minimum?: number, maximum?: number): Type.TSchema {\n  let result = schema;\n  if (minimum !== undefined) result = Type.Refine(result, (value) => [...(value as string)].length >= minimum, () => \"string is shorter than the contract\");\n  if (maximum !== undefined) result = Type.Refine(result, (value) => [...(value as string)].length <= maximum, () => \"string is longer than the contract\");\n  return result;\n}\n",
    ),
    (
        "finite",
        "function finite(schema: Type.TSchema, message: string): Type.TSchema {\n  return Type.Refine(schema, (value) => Number.isFinite(value), () => message);\n}\n",
    ),
    (
        "binary32",
        "function binary32(schema: Type.TSchema = Type.Number()): Type.TSchema {\n  return Type.Refine(schema, (value) => Number.isFinite(value) && Number.isFinite(Math.fround(value)), () => \"number is outside the f32 range\");\n}\n",
    ),
    (
        "present",
        "function present(schema: Type.TSchema): Type.TSchema {\n  return Type.Refine(schema, (value) => value !== undefined, () => \"missing value\");\n}\n",
    ),
    (
        "required",
        "function required(schema: Type.TSchema): Type.TSchema {\n  return Type.Refine(schema, (value) => value !== undefined, () => \"missing required value\");\n}\n",
    ),
    (
        "one-key",
        "function oneKey(key: string, payload: Type.TSchema): Type.TSchema {\n  return Type.Object({ [key]: payload }, { additionalProperties: false });\n}\n",
    ),
    (
        "bytes",
        "function bytes(): Type.TSchema {\n  return Type.Refine(Type.Unsafe<Uint8Array>({ type: \"object\" }), (value) => value instanceof Uint8Array, () => \"expected bytes\");\n}\n",
    ),
    (
        "date",
        "function dateCarrier(): Type.TSchema {\n  return Type.Refine(Type.Unsafe<Date>({ type: \"object\" }), (value) => value instanceof Date && Number.isFinite(value.getTime()), () => \"expected a Date\");\n}\n",
    ),
];

const MISMATCH: &str = "function pointerPath(path: string): (string | number)[] {\n  if (path === \"\") return [];\n  return path.slice(1).split(\"/\").map((segment) => {\n    const key = segment.replace(/~1/g, \"/\").replace(/~0/g, \"~\");\n    return /^(0|[1-9][0-9]*)$/.test(key) ? Number(key) : key;\n  });\n}\nfunction mismatch(schema: Type.TSchema): Validator {\n  return (value) => {\n    try {\n      if (Value.Check(schema, value)) return null;\n      const first = Value.Errors(schema, value)[0];\n      return { path: pointerPath(first?.instancePath ?? \"\"), message: first?.message ?? \"the value does not match the contract\" };\n    } catch (cause) {\n      return { path: [], message: cause instanceof Error ? cause.message : String(cause) };\n    }\n  };\n}\n";

pub(super) struct Generator<'a> {
    options: &'a TsOptions,
    walk: Walk<'a>,
    schemas: schema::Generator,
    imports: BTreeSet<String>,
}

impl<'a> Generator<'a> {
    pub(super) fn new(
        document: &'a Document,
        names: &'a std::collections::BTreeMap<String, String>,
        options: &'a TsOptions,
    ) -> Self {
        Generator {
            options,
            walk: Walk::new(document, names, options, &[]),
            schemas: schema::Generator::new(),
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
        self.walk.declarations(&mut self.schemas)?;
        Ok(self.walk.coverage())
    }

    fn has_bindings(&self) -> bool {
        self.walk.has_bindings()
    }

    fn validate(&mut self, reference: &TypeRef, subject: &str) -> Result<String, EmitError> {
        let name = self.walk.validate(&mut self.schemas, reference, subject)?;
        self.imports.insert(name.clone());
        Ok(name)
    }

    fn take_imports(&mut self) -> Imports {
        Imports {
            names: std::mem::take(&mut self.imports),
        }
    }

    fn render(&mut self) -> Result<RenderedFile, EmitError> {
        self.walk.declarations(&mut self.schemas)?;
        let mut body = self.schemas.prelude();
        body.push_str(&self.walk.module(&mut self.schemas)?);
        if !body.is_empty() && !body.ends_with("\n\n") {
            body.push('\n');
        }
        body.push_str(MISMATCH);
        body.push('\n');
        for binding in self.walk.bindings()? {
            body.push_str(&format!(
                "export const {}: Validator = {};\n",
                binding.name,
                self.schemas.validator(&binding.schema)
            ));
        }
        let package = self
            .options
            .validation
            .import_from
            .as_deref()
            .unwrap_or("typebox");
        let root = package_root(package)?;
        let value = format!("{root}/value");
        let specifier = crate::runtime::specifier(self.options.runtime, "\"./runtime.js\"");
        Ok(RenderedFile {
            file: "validators.ts",
            content: format!(
                "import Type from {};\n\
                 import Value from {};\n\
                 import type {{ Validator }} from {specifier};\n\
                 export type {{ ValidationIssue, Validator }} from {specifier};\n\n{body}",
                string(root),
                string(&value)
            ),
        })
    }
}

fn package_root(specifier: &str) -> Result<&str, EmitError> {
    let mut segments = specifier.split('/');
    let first = segments.next().unwrap_or_default();
    let root = if first.starts_with('@') {
        let name = segments.next().unwrap_or_default();
        if name.is_empty() || segments.next().is_some() {
            return Err(package_root_error(specifier));
        }
        &specifier[..first.len() + 1 + name.len()]
    } else {
        if first.is_empty() || segments.next().is_some() {
            return Err(package_root_error(specifier));
        }
        first
    };
    if root.is_empty()
        || root.contains('\\')
        || root.chars().any(char::is_whitespace)
        || root.starts_with('.')
        || root.starts_with('/')
    {
        return Err(package_root_error(specifier));
    }
    Ok(root)
}

fn package_root_error(specifier: &str) -> EmitError {
    EmitError(format!(
        "the typebox validation backend needs a package root such as typebox, not the subpath {specifier}; give the package root and the backend derives its typebox/value subpath"
    ))
}

mod schema {
    use std::collections::BTreeSet;

    use pons_ir::{DecimalWire, FieldFormat, Presence};

    use super::super::backend::{
        Backend, Capability, Declaration, FloatShape, IntCarrier, IntegerShape, Member,
        ObjectShape, Rendered, StringCarrier, StringShape, float_constraint, integral_constraint,
    };
    use super::{CANONICAL_INTEGER, CANONICAL_SIGNED_INTEGER, DECIMAL, PRELUDE, REFERENCE};
    use crate::EmitError;
    use crate::render::{jsdoc, string};

    const MAP: Capability = Capability::new("validation.map");

    pub(super) struct Generator {
        need: BTreeSet<&'static str>,
    }

    impl Generator {
        pub(super) fn new() -> Self {
            Generator {
                need: BTreeSet::new(),
            }
        }

        pub(super) fn prelude(&self) -> String {
            let mut output = String::new();
            for (id, code) in PRELUDE {
                if self.need.contains(id) {
                    output.push_str(code);
                }
            }
            output
        }

        fn one_key(&mut self, key: &str, payload: &str) -> String {
            self.need.insert("one-key");
            format!("oneKey({}, {payload})", string(key))
        }
    }

    impl Backend for Generator {
        fn name(&self) -> &'static str {
            "typebox"
        }

        fn reference(&mut self, name: &str, _variant: Option<&str>) -> Rendered {
            Rendered::exact(marker(name))
        }

        fn schema(&self, name: &str) -> String {
            marker(name)
        }

        fn object(&mut self, members: &[Member], openness: &ObjectShape) -> Rendered {
            let properties = members
                .iter()
                .map(|member| {
                    let mut schema = member.schema.clone();
                    if member.presence == Presence::Required && member.accepts_undefined {
                        self.need.insert("required");
                        schema = format!("required({schema})");
                    } else if member.presence == Presence::Optional {
                        schema = format!("Type.Optional({schema})");
                    }
                    format!("{}: {schema}", string(&member.key))
                })
                .collect::<Vec<_>>()
                .join(", ");
            let schema = match openness {
                ObjectShape::Reject => {
                    format!("Type.Object({{{properties}}}, {{ additionalProperties: false }})")
                }
                ObjectShape::PreserveOpaque => format!("Type.Object({{{properties}}})"),
                ObjectShape::PreserveTyped(value) => {
                    format!("Type.Object({{{properties}}}, {{ additionalProperties: {value} }})")
                }
            };
            Rendered::exact(schema)
        }

        fn union(&mut self, members: &[String]) -> Rendered {
            Rendered::exact(format!("Type.Union([{}])", members.join(", ")))
        }

        fn literal(&mut self, value: &str) -> Rendered {
            Rendered::exact(format!("Type.Literal({})", string(value)))
        }

        fn external(&mut self, wire: &str, payload: Option<&str>) -> Rendered {
            match payload {
                Some(payload) => {
                    let schema = self.one_key(wire, payload);
                    Rendered::exact(schema)
                }
                None => {
                    let object = self.one_key(wire, "Type.Unknown()");
                    Rendered::exact(format!(
                        "Type.Union([Type.Literal({}), {object}])",
                        string(wire)
                    ))
                }
            }
        }

        fn string(&mut self, shape: &StringShape) -> Rendered {
            if shape.carrier == StringCarrier::Date {
                self.need.insert("date");
                return Rendered::exact("dateCarrier()");
            }
            let mut schema = match shape.pattern {
                Some(pattern) => format!("Type.String({{ pattern: {} }})", string(pattern)),
                None => String::from("Type.String()"),
            };
            if let Some(format) = shape.format {
                self.need.insert("formats");
                schema = format!("format({schema}, {})", string(format_name(format)));
            }
            if shape.min_length.is_some() || shape.max_length.is_some() {
                self.need.insert("bounded");
                let mut arguments = vec![schema];
                arguments.push(
                    shape
                        .min_length
                        .map_or_else(|| String::from("undefined"), |value| value.to_string()),
                );
                if let Some(maximum) = shape.max_length {
                    arguments.push(maximum.to_string());
                }
                while arguments.last().map(String::as_str) == Some("undefined") {
                    arguments.pop();
                }
                schema = format!("bounded({})", arguments.join(", "));
            }
            Rendered::exact(schema)
        }

        fn integer(&mut self, shape: &IntegerShape) -> Result<Rendered, EmitError> {
            self.need.insert("integer");
            let base = match shape.carrier {
                IntCarrier::Text => format!(
                    "Type.String({{ pattern: {} }})",
                    string(if shape.signed {
                        CANONICAL_SIGNED_INTEGER
                    } else {
                        CANONICAL_INTEGER
                    })
                ),
                IntCarrier::Bigint => String::from("Type.BigInt()"),
                IntCarrier::Number => String::from("Type.Integer()"),
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
            Ok(Rendered::exact(format!(
                "integer({})",
                arguments.join(", ")
            )))
        }

        fn float(&mut self, shape: &FloatShape) -> Result<Rendered, EmitError> {
            let mut options = Vec::new();
            for (key, value) in [
                ("minimum", shape.minimum),
                ("maximum", shape.maximum),
                ("exclusiveMinimum", shape.exclusive_minimum),
                ("exclusiveMaximum", shape.exclusive_maximum),
                ("multipleOf", shape.multiple_of),
            ] {
                if let Some(value) = value {
                    options.push(format!("{key}: {}", float_constraint(value)?));
                }
            }
            let schema = if options.is_empty() {
                "Type.Number()".to_owned()
            } else {
                format!("Type.Number({{ {} }})", options.join(", "))
            };
            if shape.bits == 32 {
                self.need.insert("binary32");
                if options.is_empty() {
                    Ok(Rendered::exact("binary32()"))
                } else {
                    Ok(Rendered::exact(format!("binary32({schema})")))
                }
            } else {
                self.need.insert("finite");
                Ok(Rendered::exact(format!(
                    "finite({schema}, \"expected finite number\")"
                )))
            }
        }

        fn decimal(&mut self, wire: DecimalWire) -> Rendered {
            match wire {
                DecimalWire::Number => Rendered::exact("Type.Number()"),
                DecimalWire::String => {
                    Rendered::exact(format!("Type.String({{ pattern: {} }})", string(DECIMAL)))
                }
            }
        }

        fn bytes(&mut self) -> Rendered {
            self.need.insert("bytes");
            Rendered::exact("bytes()")
        }

        fn boolean(&mut self) -> Rendered {
            Rendered::exact("Type.Boolean()")
        }

        fn array(&mut self, item: &str, unique: bool) -> Rendered {
            if unique {
                Rendered::exact(format!("Type.Array({item}, {{ uniqueItems: true }})"))
            } else {
                Rendered::exact(format!("Type.Array({item})"))
            }
        }

        fn tuple(&mut self, items: &[String]) -> Rendered {
            Rendered::exact(format!("Type.Tuple([{}])", items.join(", ")))
        }

        fn map(&mut self, key: &str, value: &str) -> Rendered {
            if record_key(key) {
                Rendered::exact(format!("Type.Record({key}, {value})"))
            } else {
                Rendered::Widened(format!("Type.Record(Type.String(), {value})"), MAP)
            }
        }

        fn nullable(&mut self, inner: &str) -> Rendered {
            Rendered::exact(format!("Type.Union([{inner}, Type.Null()])"))
        }

        fn any(&mut self) -> Rendered {
            self.need.insert("present");
            Rendered::exact("present(Type.Unknown())")
        }

        fn unknown(&mut self) -> Rendered {
            self.need.insert("present");
            Rendered::exact("present(Type.Unknown())")
        }

        fn null(&mut self) -> Rendered {
            Rendered::exact("Type.Null()")
        }

        fn unit(&mut self) -> Rendered {
            Rendered::exact("Type.Null()")
        }

        fn validator(&mut self, schema: &str) -> String {
            format!("mismatch({})", bindings(schema))
        }

        fn module(&mut self, declarations: &[Declaration<'_>]) -> Result<String, EmitError> {
            if declarations.is_empty() {
                return Ok(String::new());
            }
            let names = declarations
                .iter()
                .map(|declaration| declaration.name)
                .collect::<BTreeSet<_>>();
            for declaration in declarations {
                for dependency in declaration.dependencies {
                    if !names.contains(dependency.as_str()) {
                        return Err(EmitError(format!(
                            "the typebox backend cannot declare {} without {dependency}",
                            declaration.name
                        )));
                    }
                }
            }
            let mut output = String::from("const M = Type.Module({\n");
            for declaration in declarations {
                let comment = jsdoc(declaration.doc);
                if !comment.is_empty() {
                    output.push_str("  ");
                    output.push_str(&comment);
                    output.push('\n');
                }
                output.push_str(&format!(
                    "  {}: {},\n",
                    declaration.name,
                    references(declaration.schema)
                ));
            }
            output.push_str("});\n");
            Ok(output)
        }
    }

    fn marker(name: &str) -> String {
        format!("{REFERENCE}{name}")
    }

    fn references(schema: &str) -> String {
        rewrite_markers(schema, |name| format!("Type.Ref({})", string(name)))
    }

    fn bindings(schema: &str) -> String {
        rewrite_markers(schema, |name| format!("M.{name}"))
    }

    fn rewrite_markers(schema: &str, render: impl Fn(&str) -> String) -> String {
        let mut output = String::with_capacity(schema.len());
        let mut rest = schema;
        while let Some(index) = rest.find(REFERENCE) {
            output.push_str(&rest[..index]);
            let after = &rest[index + REFERENCE.len_utf8()..];
            let end = after
                .find(|character: char| {
                    !(character.is_ascii_alphanumeric() || character == '_' || character == '$')
                })
                .unwrap_or(after.len());
            output.push_str(&render(&after[..end]));
            rest = &after[end..];
        }
        output.push_str(rest);
        output
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

    fn record_key(key: &str) -> bool {
        key == "Type.String()"
            || (key.starts_with("Type.String({ pattern: ") && key.ends_with(" })"))
    }
}

#[cfg(test)]
mod tests {
    use pons_ir::{DecimalWire, FieldFormat, Presence};

    use super::super::backend::{
        Backend, Declaration, DeclarationKind, IntCarrier, IntegerShape, Member, ObjectShape,
        Rendered, StringCarrier, StringShape,
    };
    use super::REFERENCE;
    use super::schema::Generator;

    fn rendered(rendered: Rendered) -> String {
        match rendered {
            Rendered::Exact(schema) => schema,
            Rendered::Widened(schema, _) => schema,
            Rendered::Unsupported(_) => panic!("typebox supports every shape"),
        }
    }

    #[test]
    fn an_object_keeps_the_openness_policy() {
        let mut generator = Generator::new();
        let members = [Member {
            key: "id".into(),
            schema: "Type.String()".into(),
            presence: Presence::Required,
            accepts_undefined: false,
        }];
        assert_eq!(
            rendered(generator.object(&members, &ObjectShape::Reject)),
            "Type.Object({\"id\": Type.String()}, { additionalProperties: false })"
        );
        assert_eq!(
            rendered(generator.object(&members, &ObjectShape::PreserveOpaque)),
            "Type.Object({\"id\": Type.String()})"
        );
        assert_eq!(
            rendered(generator.object(
                &members,
                &ObjectShape::PreserveTyped("Type.Integer()".into())
            )),
            "Type.Object({\"id\": Type.String()}, { additionalProperties: Type.Integer() })"
        );
    }

    #[test]
    fn a_record_key_that_is_not_plain_text_is_widened_under_validation_map() {
        let mut generator = Generator::new();
        match generator.map("format(Type.String(), \"uuid\")", "Type.String()") {
            Rendered::Widened(schema, capability) => {
                assert_eq!(schema, "Type.Record(Type.String(), Type.String())");
                assert_eq!(capability.id(), "validation.map");
            }
            _ => panic!("a formatted record key has no plain-text record form"),
        }
        assert_eq!(
            rendered(generator.map("Type.String()", "Type.String()")),
            "Type.Record(Type.String(), Type.String())"
        );
        assert_eq!(
            rendered(generator.map("Type.String({ pattern: \"^[a-z]+$\" })", "Type.String()")),
            "Type.Record(Type.String({ pattern: \"^[a-z]+$\" }), Type.String())"
        );
    }

    #[test]
    fn a_string_shape_keeps_the_format_before_the_code_point_bounds() {
        let mut generator = Generator::new();
        let schema = rendered(generator.string(&StringShape {
            format: Some(FieldFormat::Email),
            carrier: StringCarrier::Text,
            min_length: Some(2),
            max_length: Some(4),
            pattern: Some("^[a-z]+$"),
        }));
        assert_eq!(
            schema,
            "bounded(format(Type.String({ pattern: \"^[a-z]+$\" }), \"email\"), 2, 4)"
        );
    }

    #[test]
    fn a_wide_integer_names_its_carrier_and_bounds() {
        let mut generator = Generator::new();
        let minimum = serde_json::json!(3);
        let schema = rendered(
            generator
                .integer(&IntegerShape {
                    carrier: IntCarrier::Bigint,
                    bits: 64,
                    signed: false,
                    minimum: Some(&minimum),
                    maximum: None,
                    multiple_of: None,
                })
                .unwrap(),
        );
        assert_eq!(schema, "integer(Type.BigInt(), 64, false, \"3\")");
    }

    #[test]
    fn a_module_turns_each_binding_into_its_reference() {
        let mut generator = Generator::new();
        let schema =
            format!("Type.Object({{\"next\": Type.Union([{REFERENCE}Tree, Type.Null()])}})");
        let declarations = [Declaration {
            name: "Tree",
            schema: &schema,
            doc: None,
            dependencies: &[],
            kind: DeclarationKind::Type,
        }];
        let module = generator.module(&declarations).unwrap();
        assert!(module.contains("Type.Ref(\"Tree\")"), "{module}");
        assert_eq!(
            generator.validator(&format!("{REFERENCE}Tree")),
            "mismatch(M.Tree)"
        );
    }

    #[test]
    fn a_decimal_stays_text_when_the_wire_is_text() {
        let mut generator = Generator::new();
        assert_eq!(
            rendered(generator.decimal(DecimalWire::String)),
            "Type.String({ pattern: \"^-?(?:\\\\d+(?:\\\\.\\\\d*)?|\\\\.\\\\d+)(?:[eE][+-]?\\\\d+)?$\" })"
        );
    }
}
