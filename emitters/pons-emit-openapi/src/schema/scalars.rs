use pons_ir::{
    ByteEncoding, DecimalWire, EnumRepr, InlineType, IntegerWire, TypeKind, TypeRef, VariantPayload,
};
use serde_json::{Value, json};

use super::patterns::stated_pattern;
use crate::{Builder, EmitError};

const BASE64_PATTERN: &str = r"^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$";
const BASE64URL_PATTERN: &str =
    r"^(?:[A-Za-z0-9_-]{4})*(?:[A-Za-z0-9_-]{2}(?:==)?|[A-Za-z0-9_-]{3}=?)?$";
const HEX_PATTERN: &str = "^(?:[0-9a-fA-F]{2})*$";
const DECIMAL_PATTERN: &str = r"^-?(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][+-]?\d+)?$";
const SIGNED_INTEGER_PATTERN: &str = "^-?(0|[1-9][0-9]*)$";
const UNSIGNED_INTEGER_PATTERN: &str = "^(0|[1-9][0-9]*)$";

impl<'a> Builder<'a> {
    pub(super) fn inline_schema(&self, ty: &InlineType) -> Result<Value, EmitError> {
        Ok(match ty {
            InlineType::Bool => json!({"type":"boolean"}),
            InlineType::Int {
                bits,
                signed,
                wire,
                minimum,
                maximum,
                multiple_of,
            } => {
                if *wire == IntegerWire::String {
                    let pattern = if *signed {
                        SIGNED_INTEGER_PATTERN
                    } else {
                        UNSIGNED_INTEGER_PATTERN
                    };
                    json!({
                        "type":"string",
                        "pattern":stated_pattern(pattern)?,
                        "x-pons-integer":{
                            "bits":bits,
                            "signed":signed,
                            "wire":"string",
                            "minimum":minimum,
                            "maximum":maximum,
                            "multiple_of":multiple_of
                        }
                    })
                } else {
                    let mut schema = json!({"type":"integer"});
                    match (*bits, *signed) {
                        (32, true) => schema["format"] = json!("int32"),
                        (64, true) => schema["format"] = json!("int64"),
                        _ => {}
                    }
                    schema["x-pons-integer"] = json!({"bits":bits,"signed":signed,"wire":"number"});
                    if let Some((minimum, maximum)) = integer_bounds(*bits, *signed) {
                        schema["minimum"] = minimum;
                        schema["maximum"] = maximum;
                    }
                    if let Some(minimum) = minimum {
                        schema["minimum"] = minimum.clone();
                    }
                    if let Some(maximum) = maximum {
                        schema["maximum"] = maximum.clone();
                    }
                    if let Some(multiple_of) = multiple_of {
                        schema["multipleOf"] = multiple_of.clone();
                    }
                    schema
                }
            }
            InlineType::Float { bits } => {
                let mut schema =
                    json!({"type":"number","format":if *bits == 32 {"float"} else {"double"}});
                if *bits == 32 {
                    schema["minimum"] = json!(-(f32::MAX as f64));
                    schema["maximum"] = json!(f32::MAX as f64);
                }
                schema
            }
            InlineType::Decimal { wire } => {
                let mut schema = json!({
                    "type":if *wire == DecimalWire::String {"string"} else {"number"},
                });
                if *wire == DecimalWire::String {
                    schema["pattern"] = stated_pattern(DECIMAL_PATTERN)?;
                }
                schema["x-pons-decimal-wire"] = json!(wire);
                schema
            }
            InlineType::String {
                format,
                min_length,
                max_length,
                pattern,
            } => {
                let mut schema = json!({"type":"string"});
                if let Some(format) = format {
                    schema["x-pons-format"] = json!(format);
                }
                if let Some(minimum) = min_length {
                    schema["minLength"] = json!(minimum);
                }
                if let Some(maximum) = max_length {
                    schema["maxLength"] = json!(maximum);
                }
                if let Some(pattern) = pattern {
                    schema["pattern"] = stated_pattern(pattern)?;
                }
                schema
            }
            InlineType::Bytes { encoding } => match encoding {
                ByteEncoding::Base64 => json!({
                    "type":"string",
                    "contentEncoding":"base64",
                    "pattern":stated_pattern(BASE64_PATTERN)?,
                }),
                ByteEncoding::Base64url => json!({
                    "type":"string",
                    "contentEncoding":"base64url",
                    "pattern":stated_pattern(BASE64URL_PATTERN)?,
                }),
                ByteEncoding::Hex => json!({
                    "type":"string",
                    "pattern":stated_pattern(HEX_PATTERN)?,
                }),
                ByteEncoding::Array => {
                    json!({"type":"array","items":{"type":"integer","minimum":0,"maximum":255}})
                }
            },
            InlineType::Null => json!({"type":"null"}),
            InlineType::Any => json!({}),
            InlineType::Opaque { codec, wire, .. } => {
                let mut schema = json!({});
                schema["x-pons-opaque"] = json!({"codec":codec,"wire":wire});
                schema
            }
            InlineType::Nullable { inner } => {
                json!({"anyOf":[self.type_schema(inner)?,{"type":"null"}]})
            }
            InlineType::Array { item, unique } => {
                let mut schema = json!({"type":"array","items":self.type_schema(item)?});
                if let Some(unique) = unique {
                    schema["uniqueItems"] = json!(unique);
                }
                schema
            }
            InlineType::Map { key, value } => {
                json!({
                    "type":"object",
                    "propertyNames":self.type_schema(key)?,
                    "additionalProperties":self.type_schema(value)?
                })
            }
            InlineType::Tuple { items } => {
                json!({"type":"array","prefixItems":items.iter().map(|item| self.type_schema(item)).collect::<Result<Vec<_>,_>>()?,"minItems":items.len(),"maxItems":items.len(),"items":false})
            }
        })
    }

    pub(crate) fn wire_form(
        &self,
        ty: &TypeRef,
        place: Place,
    ) -> Result<Option<WireForm>, EmitError> {
        self.wire_form_of(ty, place, &mut Vec::new())
    }

    fn wire_form_of(
        &self,
        ty: &TypeRef,
        place: Place,
        visiting: &mut Vec<String>,
    ) -> Result<Option<WireForm>, EmitError> {
        let inline = match ty {
            TypeRef::Inline(inline) => inline,
            TypeRef::Named(named) => {
                let definition = self
                    .document
                    .types
                    .get(&named.id)
                    .ok_or_else(|| EmitError(format!("unknown type {}", named.id)))?;
                if let Some(wire) = &named.variant {
                    let TypeKind::Enum { variants, .. } = &definition.definition else {
                        return Err(EmitError(format!("type {} is not an enum", named.id)));
                    };
                    let Some(variant) = variants.iter().find(|one| &one.wire == wire) else {
                        return Err(EmitError(format!("unknown variant {wire}")));
                    };
                    return match &variant.payload {
                        VariantPayload::Unit => Ok(Some(WireForm::Text)),
                        VariantPayload::Newtype { ty } => self.wire_form_of(ty, place, visiting),
                        VariantPayload::Struct { .. } | VariantPayload::Tuple { .. } => {
                            Ok(Some(WireForm::Json))
                        }
                    };
                }
                if visiting.contains(&named.id) {
                    return Err(EmitError(format!(
                        "the type {} cannot be resolved because newtypes form a cycle",
                        named.id
                    )));
                }
                visiting.push(named.id.clone());
                let result = match &definition.definition {
                    TypeKind::Newtype { inner } => self.wire_form_of(inner, place, visiting),
                    TypeKind::Struct { .. } => Ok(Some(WireForm::Json)),
                    TypeKind::Enum { repr, .. } => Ok(match repr {
                        EnumRepr::String => Some(WireForm::Text),

                        EnumRepr::External => None,
                        EnumRepr::Internal { .. }
                        | EnumRepr::Adjacent { .. }
                        | EnumRepr::Untagged => Some(WireForm::Json),
                    }),
                };
                visiting.pop();
                return result;
            }
        };
        Ok(Some(match inline {
            InlineType::String { .. }
            | InlineType::Int {
                wire: IntegerWire::String,
                ..
            }
            | InlineType::Decimal {
                wire: DecimalWire::String,
            }
            | InlineType::Bytes {
                encoding: ByteEncoding::Base64 | ByteEncoding::Base64url | ByteEncoding::Hex,
            } => WireForm::Text,

            InlineType::Bool
            | InlineType::Int {
                wire: IntegerWire::Number,
                ..
            }
            | InlineType::Float { .. }
            | InlineType::Decimal {
                wire: DecimalWire::Number,
            }
            | InlineType::Any => match place {
                Place::Header => WireForm::Text,
                Place::Part { .. } => WireForm::Json,
            },

            InlineType::Opaque { .. } | InlineType::Null => return Ok(None),
            InlineType::Nullable { inner } => {
                return match place {
                    Place::Part { required: true } => Ok(None),
                    place => self.wire_form_of(inner, place, visiting),
                };
            }
            InlineType::Array { .. } | InlineType::Map { .. } | InlineType::Tuple { .. } => {
                WireForm::Json
            }

            InlineType::Bytes {
                encoding: ByteEncoding::Array,
            } => WireForm::Json,
        }))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WireForm {
    Text,
    Json,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Place {
    Header,
    Part { required: bool },
}

fn integer_bounds(bits: u16, signed: bool) -> Option<(Value, Value)> {
    if signed {
        if bits == 128 {
            return Some((json!(i128::MIN), json!(i128::MAX)));
        }
        if bits == 64 {
            return Some((json!(i64::MIN), json!(i64::MAX)));
        }
        if (1..64).contains(&bits) {
            let edge = 1_i64 << (bits - 1);
            return Some((json!(-edge), json!(edge - 1)));
        }
    } else if bits == 128 {
        return Some((json!(0), json!(u128::MAX)));
    } else if bits == 64 {
        return Some((json!(0), json!(u64::MAX)));
    } else if (1..64).contains(&bits) {
        return Some((json!(0), json!((1_u64 << bits) - 1)));
    }
    None
}

#[cfg(test)]
mod tests {
    use pons_ir::{
        ByteEncoding, DecimalWire, Direction, Field, FieldFormat, InlineType, IntegerWire,
        NamedRef, ObjectOpenness, Presence, TypeDef, TypeKind, TypeRef,
    };
    use serde_json::{Value, json};

    fn fixture() -> pons_ir::Document {
        serde_json::from_str(include_str!("../../../../spec/fixtures/login.json")).unwrap()
    }

    fn named(id: &str) -> TypeRef {
        TypeRef::Named(NamedRef {
            id: id.into(),
            variant: None,
        })
    }

    fn string() -> TypeRef {
        TypeRef::Inline(InlineType::String {
            format: None,
            min_length: None,
            max_length: None,
            pattern: None,
        })
    }

    fn integer(bits: u16, signed: bool) -> TypeRef {
        TypeRef::Inline(InlineType::Int {
            bits,
            signed,
            wire: IntegerWire::Number,
            minimum: None,
            maximum: None,
            multiple_of: None,
        })
    }

    fn field(wire: &str, ty: TypeRef) -> Field {
        Field {
            wire: wire.into(),
            ident: vec![wire.into()],
            ty,
            presence: Presence::Required,
            default: Default::default(),
            access: None,
            doc: None,
            diagnostics: Vec::new(),
            ext: Default::default(),
        }
    }

    fn struct_type(
        name: &str,
        fields: Vec<Field>,
        flattened: Vec<TypeRef>,
        openness: ObjectOpenness,
    ) -> TypeDef {
        TypeDef {
            name: vec![name.into()],
            direction: Direction::Both,
            definition: TypeKind::Struct {
                fields,
                flattened,
                openness,
            },
            origin: None,
            doc: None,
            diagnostics: Vec::new(),
            ext: Default::default(),
        }
    }

    fn closed(name: &str, fields: Vec<Field>, flattened: Vec<TypeRef>) -> TypeDef {
        struct_type(name, fields, flattened, ObjectOpenness::Reject)
    }

    fn newtype(name: &str, inner: TypeRef) -> TypeDef {
        TypeDef {
            name: vec![name.into()],
            direction: Direction::Both,
            definition: TypeKind::Newtype { inner },
            origin: None,
            doc: None,
            diagnostics: Vec::new(),
            ext: Default::default(),
        }
    }

    fn bytes(encoding: ByteEncoding) -> TypeRef {
        TypeRef::Inline(InlineType::Bytes { encoding })
    }

    fn decimal(wire: DecimalWire) -> TypeRef {
        TypeRef::Inline(InlineType::Decimal { wire })
    }

    fn float(bits: u8) -> TypeRef {
        TypeRef::Inline(InlineType::Float { bits })
    }

    fn formatted(format: FieldFormat) -> TypeRef {
        TypeRef::Inline(InlineType::String {
            format: Some(format),
            min_length: None,
            max_length: None,
            pattern: None,
        })
    }

    fn component_schema(document: &pons_ir::Document, name: &str) -> Value {
        let output: Value = serde_json::from_str(&crate::emit(document).unwrap()).unwrap();
        let mut schema = json!({"$ref":format!("#/components/schemas/{name}")});
        schema["components"] = output["components"].clone();
        schema
    }

    fn validates(document: &pons_ir::Document, name: &str, instance: Value) -> bool {
        jsonschema::Validator::new(&component_schema(document, name))
            .expect("the emitted schema must be a valid JSON Schema")
            .is_valid(&instance)
    }

    #[test]
    fn states_the_decimal_string_grammar_the_client_enforces() {
        let mut document = fixture();
        document.types.insert(
            "Prices".into(),
            struct_type(
                "prices",
                vec![
                    field("text", decimal(DecimalWire::String)),
                    field("number", decimal(DecimalWire::Number)),
                ],
                Vec::new(),
                ObjectOpenness::Reject,
            ),
        );
        let output: Value = serde_json::from_str(&crate::emit(&document).unwrap()).unwrap();
        let properties = &output["components"]["schemas"]["Prices"]["properties"];

        assert_eq!(properties["text"]["type"], "string");
        assert_eq!(
            properties["text"]["pattern"],
            json!(r"^-?(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][+-]?\d+)?$")
        );
        assert_eq!(properties["text"]["x-pons-decimal-wire"], json!("string"));
        assert_eq!(properties["number"]["type"], "number");
        assert!(
            properties["number"].get("pattern").is_none(),
            "{:#}",
            properties["number"]
        );

        for value in [
            "0", "-0", "1", "-1", "1.5", "-1.5", "1.", ".5", "-.5", "1e10", "1E+10", "1e-10",
            "0.0", "007",
        ] {
            assert!(
                validates(&document, "Prices", json!({ "text": value, "number": 1.5 })),
                "the document rejects {value}"
            );
        }
        for value in [
            "", "-", ".", "abc", "1.2.3", "e5", "+1", " 1", "1 ", "1\n", "1,5", "Infinity", "NaN",
            "1e", "--1", "1e+", "0x10",
        ] {
            assert!(
                !validates(&document, "Prices", json!({ "text": value, "number": 1.5 })),
                "the document accepts {value}"
            );
        }
        assert!(validates(
            &document,
            "Prices",
            json!({ "text": "1.5", "number": 1.5 })
        ));
        assert!(!validates(
            &document,
            "Prices",
            json!({ "text": "1.5", "number": "1.5" })
        ));
    }

    #[test]
    fn states_the_binary32_range_the_client_range_checks() {
        let mut document = fixture();
        document.types.insert(
            "Samples".into(),
            struct_type(
                "samples",
                vec![field("single", float(32)), field("wide", float(64))],
                Vec::new(),
                ObjectOpenness::Reject,
            ),
        );
        let output: Value = serde_json::from_str(&crate::emit(&document).unwrap()).unwrap();
        let properties = &output["components"]["schemas"]["Samples"]["properties"];

        assert_eq!(properties["single"]["type"], "number");
        assert_eq!(properties["single"]["format"], json!("float"));
        assert_eq!(properties["single"]["minimum"], json!(-(f32::MAX as f64)));
        assert_eq!(properties["single"]["maximum"], json!(f32::MAX as f64));
        assert!(validates(
            &document,
            "Samples",
            json!({ "single": 0.0, "wide": 1.0 })
        ));
        assert!(validates(
            &document,
            "Samples",
            json!({ "single": f32::MAX as f64, "wide": 1.0 })
        ));
        assert!(validates(
            &document,
            "Samples",
            json!({ "single": -(f32::MAX as f64), "wide": 1.0 })
        ));
        assert!(!validates(
            &document,
            "Samples",
            json!({ "single": 3.5e38, "wide": 1.0 })
        ));
        assert!(!validates(
            &document,
            "Samples",
            json!({ "single": -3.5e38, "wide": 1.0 })
        ));

        assert_eq!(properties["wide"]["format"], json!("double"));
        for keyword in ["minimum", "maximum"] {
            assert!(
                properties["wide"].get(keyword).is_none(),
                "a 64-bit float has no range the client checks: {:#}",
                properties["wide"]
            );
        }
        assert!(validates(
            &document,
            "Samples",
            json!({ "single": 1.0, "wide": 1e308 })
        ));
    }

    #[test]
    fn a_string_format_is_annotated_because_nothing_enforces_it() {
        let mut document = fixture();
        document.types.insert(
            "Contact".into(),
            closed(
                "contact",
                vec![
                    field("email", formatted(FieldFormat::Email)),
                    field("text", string()),
                ],
                Vec::new(),
            ),
        );
        document.types.insert(
            "Handles".into(),
            newtype(
                "handles",
                TypeRef::Inline(InlineType::Map {
                    key: Box::new(formatted(FieldFormat::Uuid)),
                    value: Box::new(string()),
                }),
            ),
        );
        let output: Value = serde_json::from_str(&crate::emit(&document).unwrap()).unwrap();
        let properties = &output["components"]["schemas"]["Contact"]["properties"];

        assert_eq!(properties["email"]["type"], "string");
        assert_eq!(properties["email"]["x-pons-format"], json!("email"));
        assert!(
            properties["email"].get("format").is_none(),
            "{:#}",
            properties["email"]
        );
        for keyword in ["format", "x-pons-format"] {
            assert!(
                properties["text"].get(keyword).is_none(),
                "a string with no format states neither: {:#}",
                properties["text"]
            );
        }
        let key = &output["components"]["schemas"]["Handles"]["propertyNames"];
        assert_eq!(key["type"], "string");
        assert_eq!(key["x-pons-format"], json!("uuid"));
        assert!(key.get("format").is_none(), "{key:#}");

        let instance = json!({"email": "not-an-email", "text": "x"});
        assert!(
            validates(&document, "Contact", instance.clone()),
            "the document accepts what it never checks"
        );
        assert!(
            jsonschema::options()
                .should_validate_formats(true)
                .build(&component_schema(&document, "Contact"))
                .expect("the emitted schema must be a valid JSON Schema")
                .is_valid(&instance),
            "a consumer that asserts formats must not read a constraint this emitter no longer states"
        );
    }

    #[test]
    fn rejects_a_self_referential_newtype() {
        let mut document = fixture();
        document
            .types
            .insert("Loop".into(), newtype("loop", named("Loop")));
        assert!(
            crate::emit(&document)
                .unwrap_err()
                .0
                .contains("the type Loop cannot be resolved because newtypes form a cycle")
        );
    }

    #[test]
    fn rejects_a_cycle_of_newtypes() {
        let mut document = fixture();
        document.types.insert("A".into(), newtype("a", named("B")));
        document.types.insert("B".into(), newtype("b", named("A")));
        let error = crate::emit(&document).unwrap_err();
        for member in ["A", "B"] {
            let expected =
                format!("the type {member} cannot be resolved because newtypes form a cycle");
            if error.0.contains(&expected) {
                return;
            }
        }
        panic!("{}", error.0);
    }

    #[test]
    fn emits_exact_bounds_for_128_bit_integers() {
        let mut document = fixture();
        document.types.insert(
            "Wide".into(),
            closed(
                "wide",
                vec![
                    field("unsigned", integer(128, false)),
                    field("signed", integer(128, true)),
                ],
                Vec::new(),
            ),
        );
        let output: Value = serde_json::from_str(&crate::emit(&document).unwrap()).unwrap();
        let properties = &output["components"]["schemas"]["Wide"]["properties"];
        assert_eq!(properties["unsigned"]["type"], "integer");
        assert_eq!(properties["unsigned"]["minimum"].to_string(), "0");
        assert_eq!(
            properties["unsigned"]["maximum"].to_string(),
            "340282366920938463463374607431768211455"
        );
        assert_eq!(
            properties["signed"]["minimum"].to_string(),
            "-170141183460469231731687303715884105728"
        );
        assert_eq!(
            properties["signed"]["maximum"].to_string(),
            "170141183460469231731687303715884105727"
        );
    }

    #[test]
    fn a_width_no_client_can_send_is_no_range_rather_than_a_shift_by_the_whole_word() {
        for signed in [true, false] {
            assert_eq!(
                super::integer_bounds(0, signed),
                None,
                "a width of zero is refused by the IR, and the emitter must survive a caller \
                 that skipped that check"
            );
        }
        for (bits, signed, minimum, maximum) in [
            (8, true, json!(-128), json!(127)),
            (8, false, json!(0), json!(255)),
            (16, true, json!(-32768), json!(32767)),
            (16, false, json!(0), json!(65535)),
            (32, true, json!(-2147483648), json!(2147483647)),
            (32, false, json!(0), json!(4294967295_u32)),
        ] {
            assert_eq!(
                super::integer_bounds(bits, signed),
                Some((minimum, maximum))
            );
        }
    }

    #[test]
    fn states_the_padding_rule_the_client_enforces_for_each_byte_encoding() {
        let mut document = fixture();
        document.types.insert(
            "Blobs".into(),
            closed(
                "blobs",
                vec![
                    field("standard", bytes(ByteEncoding::Base64)),
                    field("url", bytes(ByteEncoding::Base64url)),
                    field("hex", bytes(ByteEncoding::Hex)),
                ],
                Vec::new(),
            ),
        );
        let output: Value = serde_json::from_str(&crate::emit(&document).unwrap()).unwrap();
        let properties = &output["components"]["schemas"]["Blobs"]["properties"];
        assert_eq!(properties["standard"]["contentEncoding"], "base64");
        assert_eq!(
            properties["standard"]["pattern"],
            json!("^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$")
        );
        assert_eq!(properties["url"]["contentEncoding"], "base64url");
        assert_eq!(
            properties["url"]["pattern"],
            json!("^(?:[A-Za-z0-9_-]{4})*(?:[A-Za-z0-9_-]{2}(?:==)?|[A-Za-z0-9_-]{3}=?)?$")
        );
        assert_eq!(properties["hex"]["pattern"], json!("^(?:[0-9a-fA-F]{2})*$"));

        let instance = |wire: &str, value: &str| {
            let mut instance = json!({"standard":"QQ==","url":"QQ","hex":"00"});
            instance[wire] = json!(value);
            instance
        };
        for (wire, accepted, rejected) in [
            (
                "standard",
                vec!["", "QQ==", "QUE=", "QUJD", "QUJDRA==", "a+b/"],
                vec!["QQ", "QUJDQ", "QQ=", "a-b_", "===="],
            ),
            (
                "url",
                vec!["", "QQ", "QQ==", "QUE", "QUE=", "QUJD", "QUJDRA", "a-b_"],
                vec!["QQ=", "QQ===", "QUJDQ", "a+b/", "+", "===="],
            ),
            (
                "hex",
                vec!["", "00", "0a1B", "deadBEEF"],
                vec!["0", "abc", "0g", "0 0"],
            ),
        ] {
            for value in accepted {
                assert!(
                    validates(&document, "Blobs", instance(wire, value)),
                    "the document rejects {wire} {value:?}"
                );
            }
            for value in rejected {
                assert!(
                    !validates(&document, "Blobs", instance(wire, value)),
                    "the document accepts {wire} {value:?}"
                );
            }
        }
    }
}
