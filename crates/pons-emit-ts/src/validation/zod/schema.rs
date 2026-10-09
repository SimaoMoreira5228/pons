use std::collections::BTreeSet;

use pons_ir::{DecimalWire, FieldFormat, Presence};

use super::super::backend::{
    Backend, Declaration, DeclarationKind, IntCarrier, IntegerShape, Member, ObjectShape, Rendered,
    StringCarrier, StringShape,
};
use crate::{EmitError, jsdoc, string};

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

    fn integer(&mut self, shape: &IntegerShape) -> Rendered {
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
        let schema = rendered(generator.integer(&IntegerShape {
            carrier: IntCarrier::Number,
            bits: 16,
            signed: false,
            minimum: Some(&minimum),
            maximum: None,
            multiple_of: None,
        }));
        assert_eq!(schema, "integer(z.int(), 16, false, \"3\")");
    }
}
