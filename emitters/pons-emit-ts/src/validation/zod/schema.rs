use std::collections::BTreeSet;

use pons_ir::{DecimalWire, FieldFormat, Presence};

use super::super::backend::{
    Backend, Declaration, DeclarationKind, IntCarrier, IntegerShape, Member, ObjectShape, Rendered,
    StringCarrier, StringShape, integral_constraint,
};
use crate::EmitError;
use crate::render::{jsdoc, string};

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
        "date",
        r"const DATE = /^(\d{4})-(\d{2})-(\d{2})$/;
",
    ),
    (
        "time",
        r"const TIME = /^(\d{2}):(\d{2}):(\d{2})(?:\.\d+)?(?:[Zz]|([+-])(\d{2}):(\d{2}))$/;
",
    ),
    (
        "date-time",
        r"const DATE_TIME = /^(\d{4})-(\d{2})-(\d{2})[Tt](\d{2}):(\d{2}):(\d{2})(?:\.\d{1,3})?(?:[Zz]|([+-])(\d{2}):(\d{2}))$/;
",
    ),
    (
        "uuid",
        r"const UUID = /^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$/;
",
    ),
    (
        "email",
        r"const EMAIL = /^(?:[A-Za-z0-9_'+\-]+\.)*[A-Za-z0-9_'+\-]*[A-Za-z0-9_+-]@(?:[A-Za-z0-9][A-Za-z0-9\-]*\.)+[A-Za-z]{2,}$/;
",
    ),
    (
        "duration",
        r"const DURATION = /^P(?:(\d+W)|(?!.*W)(?=\d|T\d)(\d+Y)?(\d+M)?(\d+D)?(T(?=\d)(\d+H)?(\d+M)?(\d+([.,]\d+)?S)?)?)$/;
",
    ),
    (
        "ipv4",
        r"const IPV4 = /^(?:(?:25[0-5]|2[0-4][0-9]|1[0-9][0-9]|[1-9]?[0-9])\.){3}(?:25[0-5]|2[0-4][0-9]|1[0-9][0-9]|[1-9]?[0-9])$/;
",
    ),
    (
        "ipv6",
        r"const IPV6 = /^(([0-9a-fA-F]{1,4}:){7}[0-9a-fA-F]{1,4}|([0-9a-fA-F]{1,4}:){1,7}:|([0-9a-fA-F]{1,4}:){1,6}:[0-9a-fA-F]{1,4}|([0-9a-fA-F]{1,4}:){1,5}(:[0-9a-fA-F]{1,4}){1,2}|([0-9a-fA-F]{1,4}:){1,4}(:[0-9a-fA-F]{1,4}){1,3}|([0-9a-fA-F]{1,4}:){1,3}(:[0-9a-fA-F]{1,4}){1,4}|([0-9a-fA-F]{1,4}:){1,2}(:[0-9a-fA-F]{1,4}){1,5}|[0-9a-fA-F]{1,4}:((:[0-9a-fA-F]{1,4}){1,6})|:((:[0-9a-fA-F]{1,4}){1,7}|:))$/;
",
    ),
    (
        "calendar-day",
        r#"function isCalendarDay(year: string | undefined, month: string | undefined, day: string | undefined): boolean {
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
"#,
    ),
    (
        "clock-bounds",
        r#"function isClock(hour: string | undefined, minute: string | undefined, second: string | undefined): boolean {
  return Number(hour) <= 23 && Number(minute) <= 59 && Number(second) <= 59;
}
function isOffset(hour: string | undefined, minute: string | undefined): boolean {
  return hour === undefined || (Number(hour) <= 23 && Number(minute) <= 59);
}
"#,
    ),
    (
        "instant",
        r#"const instant = () => z.string().regex(DATE_TIME, "the date-time is not a full-date and a full-time").refine((value) => {
  const match = DATE_TIME.exec(value);
  return match !== null && isCalendarDay(match[1], match[2], match[3]) && isClock(match[4], match[5], match[6]) && isOffset(match[8], match[9]) && !Number.isNaN(Date.parse(value));
}, "date-time is not an instant a Date can hold");
"#,
    ),
    (
        "day",
        r#"const day = () => z.string().regex(DATE, "the date is not a full-date").refine((value) => {
  const match = DATE.exec(value);
  return match !== null && isCalendarDay(match[1], match[2], match[3]);
}, "date is not a day the calendar has");
"#,
    ),
    (
        "clock",
        r#"const clock = () => z.string().regex(TIME, "the time is not a full-time").refine((value) => {
  const match = TIME.exec(value);
  return match !== null && isClock(match[1], match[2], match[3]) && isOffset(match[5], match[6]);
}, "the time is not a clock the contract accepts");
"#,
    ),
    (
        "uri",
        r#"const uri = () => z.string().refine((value) => {
  if (/\s/.test(value)) return false;
  try {
    return new URL(value).protocol.length > 1;
  } catch {
    return false;
  }
}, "expected a uri");
"#,
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

pub(crate) struct Generator {
    need: BTreeSet<&'static str>,
}

impl Generator {
    pub(crate) fn new() -> Self {
        Generator {
            need: BTreeSet::new(),
        }
    }
}

impl Backend for Generator {
    fn name(&self) -> &'static str {
        "zod"
    }

    fn reference(&mut self, name: &str, _variant: Option<&str>) -> Rendered {
        Rendered::exact(format!("z.lazy(() => {name}Schema)"))
    }

    fn schema(&self, name: &str) -> String {
        format!("{name}Schema")
    }

    fn object(&mut self, members: &[Member], openness: &ObjectShape) -> Rendered {
        let members = members
            .iter()
            .map(|member| {
                let mut schema = member.schema.clone();
                if member.presence == Presence::Required && member.accepts_undefined {
                    self.need.insert("required");
                    schema = format!("required({schema})");
                } else if member.presence == Presence::Optional {
                    schema.push_str(".optional()");
                }
                format!("{}: {schema}", string(&member.key))
            })
            .collect::<Vec<_>>()
            .join(", ");
        let mut schema = format!("z.object({{{members}}})");
        match openness {
            ObjectShape::Reject => schema.push_str(".strict()"),
            ObjectShape::PreserveTyped(value) => schema.push_str(&format!(".catchall({value})")),
            ObjectShape::PreserveOpaque => {}
        }
        Rendered::exact(schema)
    }

    fn union(&mut self, members: &[String]) -> Rendered {
        Rendered::exact(format!("z.union([{}])", members.join(", ")))
    }

    fn literal(&mut self, value: &str) -> Rendered {
        Rendered::exact(format!("z.literal({})", string(value)))
    }

    fn external(&mut self, wire: &str, payload: Option<&str>) -> Rendered {
        self.need.insert("one-key");
        self.need.insert("required");
        match payload {
            Some(payload) => Rendered::exact(format!("oneKey({}, {payload})", string(wire))),
            None => Rendered::exact(format!(
                "z.union([z.literal({wire}), oneKey({wire}, z.unknown())])",
                wire = string(wire)
            )),
        }
    }

    fn string(&mut self, shape: &StringShape) -> Rendered {
        if shape.carrier == StringCarrier::Date {
            return Rendered::exact("z.date()");
        }
        let mut schema = match shape.format {
            None => "z.string()".to_owned(),
            Some(FieldFormat::DateTime) => {
                self.need.insert("date-time");
                self.need.insert("calendar-day");
                self.need.insert("clock-bounds");
                self.need.insert("instant");
                "instant()".into()
            }
            Some(FieldFormat::Date) => {
                self.need.insert("date");
                self.need.insert("calendar-day");
                self.need.insert("day");
                "day()".into()
            }
            Some(FieldFormat::Time) => {
                self.need.insert("time");
                self.need.insert("clock-bounds");
                self.need.insert("clock");
                "clock()".into()
            }
            Some(FieldFormat::Duration) => {
                self.need.insert("duration");
                "z.string().regex(DURATION, \"expected a duration\")".into()
            }
            Some(FieldFormat::Uuid) => {
                self.need.insert("uuid");
                "z.string().regex(UUID, \"expected a uuid\")".into()
            }
            Some(FieldFormat::Uri) => {
                self.need.insert("uri");
                "uri()".into()
            }
            Some(FieldFormat::Email) => {
                self.need.insert("email");
                "z.string().regex(EMAIL, \"expected an email\")".into()
            }
            Some(FieldFormat::Ipv4) => {
                self.need.insert("ipv4");
                "z.string().regex(IPV4, \"expected an ipv4 address\")".into()
            }
            Some(FieldFormat::Ipv6) => {
                self.need.insert("ipv6");
                "z.string().regex(IPV6, \"expected an ipv6 address\")".into()
            }
        };
        if let Some(pattern) = shape.pattern {
            schema.push_str(&format!(".regex(new RegExp({}))", string(pattern)));
        }
        if let Some(minimum) = shape.min_length {
            schema.push_str(&format!(
                ".refine((value) => [...value].length >= {minimum}, \"string is shorter than the contract\")"
            ));
        }
        if let Some(maximum) = shape.max_length {
            schema.push_str(&format!(
                ".refine((value) => [...value].length <= {maximum}, \"string is longer than the contract\")"
            ));
        }
        Rendered::exact(schema)
    }

    fn integer(&mut self, shape: &IntegerShape) -> Result<Rendered, EmitError> {
        self.need.insert("integer");
        let base = match shape.carrier {
            IntCarrier::Text => {
                self.need.insert(if shape.signed {
                    "canonical-signed-integer"
                } else {
                    "canonical-integer"
                });
                format!(
                    "z.string().regex({})",
                    if shape.signed {
                        "CANONICAL_SIGNED_INTEGER"
                    } else {
                        "CANONICAL_INTEGER"
                    }
                )
            }
            IntCarrier::Bigint => "z.bigint()".into(),
            IntCarrier::Number if shape.bits <= 53 => "z.int()".into(),
            IntCarrier::Number => "z.number().int()".into(),
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

    fn float(&mut self, bits: u8) -> Rendered {
        if bits == 32 {
            self.need.insert("binary32");
            Rendered::exact("binary32()")
        } else {
            Rendered::exact("z.number()")
        }
    }

    fn decimal(&mut self, wire: DecimalWire) -> Rendered {
        match wire {
            DecimalWire::Number => Rendered::exact("z.number()"),
            DecimalWire::String => {
                self.need.insert("decimal");
                Rendered::exact("z.string().regex(DECIMAL)")
            }
        }
    }

    fn bytes(&mut self) -> Rendered {
        Rendered::exact("z.instanceof(Uint8Array)")
    }

    fn boolean(&mut self) -> Rendered {
        Rendered::exact("z.boolean()")
    }

    fn array(&mut self, item: &str, unique: bool) -> Rendered {
        if unique {
            self.need.insert("unique");
            Rendered::exact(format!("uniqueItems({item})"))
        } else {
            Rendered::exact(format!("z.array({item})"))
        }
    }

    fn tuple(&mut self, items: &[String]) -> Rendered {
        Rendered::exact(format!("z.tuple([{}])", items.join(", ")))
    }

    fn map(&mut self, key: &str, value: &str) -> Rendered {
        Rendered::exact(format!("z.record({key}, {value})"))
    }

    fn nullable(&mut self, inner: &str) -> Rendered {
        Rendered::exact(format!("{inner}.nullable()"))
    }

    fn any(&mut self) -> Rendered {
        Rendered::exact("z.any()")
    }

    fn unknown(&mut self) -> Rendered {
        Rendered::exact("z.unknown()")
    }

    fn null(&mut self) -> Rendered {
        Rendered::exact("z.null()")
    }

    fn unit(&mut self) -> Rendered {
        Rendered::exact("z.null()")
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
                        "the zod backend cannot declare {} without {dependency}",
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
            Rendered::Unsupported(_) => panic!("zod supports every shape"),
        }
    }

    #[test]
    fn a_string_shape_keeps_the_regex_and_the_code_point_bounds_in_order() {
        let mut generator = Generator::new();
        let schema = rendered(generator.string(&StringShape {
            format: None,
            carrier: StringCarrier::Text,
            min_length: Some(2),
            max_length: Some(4),
            pattern: Some("^[a-z]+$"),
        }));
        assert_eq!(
            schema,
            "z.string().regex(new RegExp(\"^[a-z]+$\")).refine((value) => [...value].length >= 2, \"string is shorter than the contract\").refine((value) => [...value].length <= 4, \"string is longer than the contract\")"
        );
    }

    #[test]
    fn an_instant_becomes_a_date_only_under_the_date_carrier() {
        let mut generator = Generator::new();
        let text = rendered(generator.string(&StringShape {
            format: Some(FieldFormat::DateTime),
            carrier: StringCarrier::Text,
            min_length: None,
            max_length: None,
            pattern: None,
        }));
        assert_eq!(text, "instant()");
        let date = rendered(generator.string(&StringShape {
            format: Some(FieldFormat::DateTime),
            carrier: StringCarrier::Date,
            min_length: None,
            max_length: None,
            pattern: None,
        }));
        assert_eq!(date, "z.date()");
    }

    #[test]
    fn trailing_undefined_constraints_are_dropped_from_the_integer_call() {
        let mut generator = Generator::new();
        let minimum = serde_json::json!(3);
        let schema = rendered(
            generator
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
        assert_eq!(schema, "integer(z.int(), 16, false, \"3\")");
    }

    #[test]
    fn integral_decimal_constraints_are_canonicalised() {
        let mut generator = Generator::new();
        let minimum = serde_json::json!(5.0);
        let maximum = serde_json::json!(1e3);
        let schema = rendered(
            generator
                .integer(&IntegerShape {
                    carrier: IntCarrier::Number,
                    bits: 32,
                    signed: true,
                    minimum: Some(&minimum),
                    maximum: Some(&maximum),
                    multiple_of: None,
                })
                .unwrap(),
        );
        assert_eq!(schema, "integer(z.int(), 32, true, \"5\", \"1000\")");
    }

    #[test]
    fn a_fractional_constraint_is_rejected() {
        let mut generator = Generator::new();
        let minimum = serde_json::json!(1.5);
        let error = generator
            .integer(&IntegerShape {
                carrier: IntCarrier::Number,
                bits: 32,
                signed: true,
                minimum: Some(&minimum),
                maximum: None,
                multiple_of: None,
            })
            .map(|_| ())
            .unwrap_err();
        assert!(error.0.contains("not an integral value"));
    }
}
