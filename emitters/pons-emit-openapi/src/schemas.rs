use pons_ir::{
    Access, ByteEncoding, DecimalWire, Document, EnumRepr, Field, FieldDefault, InlineType,
    IntegerWire, ObjectOpenness, Presence, TypeDef, TypeKind, TypeRef, Variant, VariantPayload,
};
use serde_json::{Map, Value, json};

use crate::{Builder, EmitError};

const BASE64_PATTERN: &str = r"^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$";
const BASE64URL_PATTERN: &str =
    r"^(?:[A-Za-z0-9_-]{4})*(?:[A-Za-z0-9_-]{2}(?:==)?|[A-Za-z0-9_-]{3}=?)?$";
const HEX_PATTERN: &str = "^(?:[0-9a-fA-F]{2})*$";
const DECIMAL_PATTERN: &str = r"^-?(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][+-]?\d+)?$";
const SIGNED_INTEGER_PATTERN: &str = "^-?(0|[1-9][0-9]*)$";
const UNSIGNED_INTEGER_PATTERN: &str = "^(0|[1-9][0-9]*)$";

fn reference(name: &str) -> Value {
    json!({"$ref": format!("#/components/schemas/{}", name.replace('~', "~0").replace('/', "~1"))})
}

fn variant_reference(name: &str, keyword: &str, index: usize) -> Value {
    let name = name.replace('~', "~0").replace('/', "~1");
    json!({"$ref": format!("#/components/schemas/{name}/{keyword}/{index}")})
}

fn add_doc(schema: &mut Value, doc: Option<&pons_ir::Doc>) {
    let Some(doc) = doc else { return };
    if let Some(description) = &doc.description {
        schema["description"] = json!(description);
    }
    if let Some(summary) = &doc.summary {
        schema["title"] = json!(summary);
    }
    if let Some(deprecated) = doc.deprecated {
        schema["deprecated"] = json!(deprecated);
    }
}

fn add_extensions(schema: &mut Value, key: &str, metadata: Value) {
    let extensions = schema
        .as_object_mut()
        .unwrap()
        .entry("x-pons")
        .or_insert_with(|| json!({}));
    extensions[key] = metadata;
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

impl<'a> Builder<'a> {
    pub(super) fn field_schema(&self, field: &pons_ir::Field) -> Result<Value, EmitError> {
        let mut schema = self.type_schema(&field.ty)?;
        match field.access {
            Some(Access::ReadOnly) => schema["readOnly"] = json!(true),
            Some(Access::WriteOnly) => schema["writeOnly"] = json!(true),
            _ => {}
        }
        add_doc(&mut schema, field.doc.as_ref());
        if let FieldDefault::Value(default) = &field.default {
            schema["default"] = default.clone();
        }
        add_extensions(
            &mut schema,
            "field",
            json!({
                "ident":field.ident,
                "presence":field.presence,
                "access":field.access,
                "doc":field.doc,
                "ext":field.ext,
                "diagnostics":field.diagnostics,
            }),
        );
        Ok(schema)
    }

    pub(super) fn schemas(&self) -> Result<Map<String, Value>, EmitError> {
        let mut schemas = Map::new();
        for (id, definition) in &self.document.types {
            let mut schema = self.definition_schema(definition)?;
            add_doc(&mut schema, definition.doc.as_ref());
            let openness = match &definition.definition {
                TypeKind::Struct { openness, .. } => Some(openness),
                _ => None,
            };
            add_extensions(
                &mut schema,
                "type",
                json!({"id":id,"name":definition.name,"direction":definition.direction,"origin":definition.origin,"doc":definition.doc,"objectOpenness":openness,"ext":definition.ext,"diagnostics":definition.diagnostics}),
            );
            schemas.insert(self.schema_names[id].clone(), schema);
        }
        Ok(schemas)
    }

    fn definition_schema(&self, definition: &TypeDef) -> Result<Value, EmitError> {
        match &definition.definition {
            TypeKind::Struct {
                fields,
                flattened,
                openness,
            } => {
                let mut object = fields_schema_with(self, fields)?;
                let additional = self.openness_schema(openness)?;
                if flattened.is_empty() {
                    object["additionalProperties"] = additional;
                    Ok(object)
                } else {
                    let mut all_of = vec![object];
                    for ty in flattened {
                        self.flattened_schema(ty, &mut Vec::new(), &mut all_of)?;
                    }
                    let mut wrapper = json!({"allOf":all_of});
                    wrapper["unevaluatedProperties"] = additional;
                    Ok(wrapper)
                }
            }
            TypeKind::Enum { repr, variants } => self.enum_schema(repr, variants),
            TypeKind::Newtype { inner } => self.type_schema(inner),
        }
    }

    fn flattened_schema(
        &self,
        ty: &TypeRef,
        visiting: &mut Vec<String>,
        all_of: &mut Vec<Value>,
    ) -> Result<(), EmitError> {
        let TypeRef::Named(named) = ty else {
            return Err(not_a_struct_member(ty));
        };
        if visiting.contains(&named.id) {
            return Err(EmitError(format!(
                "the type {} cannot be inlined because flattened members form a cycle",
                named.id
            )));
        }
        let Some(definition) = self.document.types.get(&named.id) else {
            return Err(EmitError(format!("unknown type {}", named.id)));
        };
        visiting.push(named.id.clone());
        let result = match &definition.definition {
            TypeKind::Newtype { inner } => self.flattened_schema(inner, visiting, all_of),
            TypeKind::Enum { .. } => Err(not_a_struct_member(ty)),
            TypeKind::Struct {
                fields, flattened, ..
            } => {
                all_of.push(fields_schema_with(self, fields)?);
                for member in flattened {
                    self.flattened_schema(member, visiting, all_of)?;
                }
                Ok(())
            }
        };
        visiting.pop();
        result
    }

    pub(super) fn type_schema(&self, ty: &TypeRef) -> Result<Value, EmitError> {
        match ty {
            TypeRef::Named(named) => {
                let definition = self
                    .document
                    .types
                    .get(&named.id)
                    .ok_or_else(|| EmitError(format!("unknown type {}", named.id)))?;
                if let Some(variant_name) = &named.variant {
                    let TypeKind::Enum { repr, variants } = &definition.definition else {
                        return Err(EmitError(format!("type {} is not an enum", named.id)));
                    };
                    let index = variants
                        .iter()
                        .position(|candidate| candidate.wire == *variant_name)
                        .ok_or_else(|| EmitError(format!("unknown variant {variant_name}")))?;
                    let active = (named.id.clone(), variant_name.clone());
                    if !self.selected_variants.borrow_mut().insert(active.clone()) {
                        let keyword = if matches!(repr, EnumRepr::Untagged) {
                            "anyOf"
                        } else {
                            "oneOf"
                        };
                        return Ok(variant_reference(
                            &self.schema_names[&named.id],
                            keyword,
                            index,
                        ));
                    }
                    let result = self.enum_variant_schema(repr, &variants[index]);
                    self.selected_variants.borrow_mut().remove(&active);
                    return result;
                }
                Ok(reference(&self.schema_names[&named.id]))
            }
            TypeRef::Inline(ty) => self.inline_schema(ty),
        }
    }

    fn inline_schema(&self, ty: &InlineType) -> Result<Value, EmitError> {
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

    fn enum_schema(&self, repr: &EnumRepr, variants: &[Variant]) -> Result<Value, EmitError> {
        if let EnumRepr::Internal { .. } = repr {
            for variant in variants {
                match &variant.payload {
                    VariantPayload::Unit | VariantPayload::Struct { .. } => {}
                    VariantPayload::Newtype { ty } if self.object_type(ty, &mut Vec::new())? => {}
                    VariantPayload::Newtype { .. } | VariantPayload::Tuple { .. } => {
                        return Err(EmitError(format!(
                            "internally tagged enum variant {} must have an object payload",
                            variant.wire
                        )));
                    }
                }
            }
        }
        let schemas = variants
            .iter()
            .map(|variant| self.enum_variant_schema(repr, variant))
            .collect::<Result<Vec<_>, _>>()?;
        let mut schema = if matches!(repr, EnumRepr::Untagged) {
            json!({"anyOf":schemas})
        } else {
            json!({"oneOf":schemas})
        };
        schema["x-pons-enum-repr"] = json!(repr);
        Ok(schema)
    }

    fn object_type(&self, ty: &TypeRef, visited: &mut Vec<String>) -> Result<bool, EmitError> {
        match ty {
            TypeRef::Inline(InlineType::Map { .. }) => Ok(true),
            TypeRef::Named(named) if visited.contains(&named.id) => Ok(false),
            TypeRef::Named(named) => {
                let definition = self
                    .document
                    .types
                    .get(&named.id)
                    .ok_or_else(|| EmitError(format!("unknown type {}", named.id)))?;
                match &definition.definition {
                    TypeKind::Struct { .. } => Ok(true),
                    TypeKind::Newtype { inner } => {
                        visited.push(named.id.clone());
                        let result = self.object_type(inner, visited);
                        visited.pop();
                        result
                    }
                    TypeKind::Enum { .. } => Ok(false),
                }
            }
            TypeRef::Inline(_) => Ok(false),
        }
    }

    fn enum_variant_schema(&self, repr: &EnumRepr, variant: &Variant) -> Result<Value, EmitError> {
        let payload = self.payload_schema(&variant.payload)?;
        let mut schema = match repr {
            EnumRepr::String => json!({"type":"string","const":variant.wire}),
            EnumRepr::External => match variant.payload {
                VariantPayload::Unit => json!({"type":"string","const":variant.wire}),
                _ => {
                    let mut properties = Map::new();
                    properties.insert(variant.wire.clone(), payload);
                    json!({"type":"object","properties":properties,"required":[variant.wire],"minProperties":1,"maxProperties":1})
                }
            },
            EnumRepr::Internal { tag } => match &variant.payload {
                VariantPayload::Unit => tagged_schema(tag, &variant.wire),
                VariantPayload::Struct { fields } => {
                    tagged(fields_schema_with(self, fields)?, tag, &variant.wire)
                }

                VariantPayload::Newtype { ty } => match self.inlined_object(ty)? {
                    Some(object) => tagged(object, tag, &variant.wire),
                    None => json!({"allOf":[tagged_schema(tag, &variant.wire),payload]}),
                },
                _ => json!({"allOf":[tagged_schema(tag, &variant.wire),payload]}),
            },
            EnumRepr::Adjacent { tag, content } => {
                let mut properties = Map::new();
                properties.insert(tag.clone(), json!({"const":variant.wire}));
                let mut required = vec![json!(tag)];
                if !matches!(variant.payload, VariantPayload::Unit) {
                    properties.insert(content.clone(), payload);
                    required.push(json!(content));
                }
                json!({"type":"object","properties":properties,"required":required})
            }
            EnumRepr::Untagged => payload,
        };
        add_doc(&mut schema, variant.doc.as_ref());
        add_extensions(
            &mut schema,
            "variant",
            json!({
                "wire":variant.wire,
                "ident":variant.ident,
                "doc":variant.doc,
                "ext":variant.ext,
                "diagnostics":variant.diagnostics,
            }),
        );
        Ok(schema)
    }

    fn payload_schema(&self, payload: &VariantPayload) -> Result<Value, EmitError> {
        match payload {
            VariantPayload::Unit => Ok(json!({"type":"null"})),
            VariantPayload::Newtype { ty } => self.type_schema(ty),
            VariantPayload::Tuple { items } => self.inline_schema(&InlineType::Tuple {
                items: items.clone(),
            }),
            VariantPayload::Struct { fields } => fields_schema_with(self, fields),
        }
    }

    fn inlined_object(&self, ty: &TypeRef) -> Result<Option<Value>, EmitError> {
        let Some((fields, openness)) = self.object_fields(ty, &mut Vec::new())? else {
            return Ok(None);
        };
        let mut object = fields_schema_with(self, fields)?;
        object["additionalProperties"] = self.openness_schema(&openness)?;
        Ok(Some(object))
    }

    pub(super) fn object_fields<'b>(
        &'b self,
        ty: &'b TypeRef,
        visiting: &mut Vec<String>,
    ) -> Result<Option<(Vec<&'b Field>, ObjectOpenness)>, EmitError> {
        let TypeRef::Named(named) = ty else {
            return Ok(None);
        };
        if named.variant.is_some() || visiting.contains(&named.id) {
            return Ok(None);
        }
        let Some(definition) = self.document.types.get(&named.id) else {
            return Err(EmitError(format!("unknown type {}", named.id)));
        };
        visiting.push(named.id.clone());
        let result = match &definition.definition {
            TypeKind::Newtype { inner } => self.object_fields(inner, visiting),
            TypeKind::Enum { .. } => Ok(None),
            TypeKind::Struct {
                fields,
                flattened,
                openness,
            } => {
                let mut collected = fields.iter().collect::<Vec<_>>();
                for member in flattened {
                    let Some((inner, _)) = self.object_fields(member, visiting)? else {
                        return Err(not_a_struct_member(member));
                    };
                    collected.extend(inner);
                }
                Ok(Some((collected, openness.clone())))
            }
        };
        visiting.pop();
        result
    }

    fn openness_schema(&self, openness: &ObjectOpenness) -> Result<Value, EmitError> {
        Ok(match openness {
            ObjectOpenness::Reject => json!(false),
            ObjectOpenness::PreserveTyped { value } => {
                self.type_schema(openness_value(value, self.document))?
            }
            ObjectOpenness::PreserveOpaque => json!(true),
        })
    }

    pub(super) fn wire_form(
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
pub(super) enum WireForm {
    Text,

    Json,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Place {
    Header,

    Part { required: bool },
}

fn stated_pattern(pattern: &str) -> Result<Value, EmitError> {
    if compiles(pattern) {
        Ok(json!(pattern))
    } else {
        Err(EmitError(format!(
            "the pattern {pattern:?} cannot be stated because it is not a regular expression the document and the TypeScript client can both compile"
        )))
    }
}

fn compiles(pattern: &str) -> bool {
    let chars = pattern.chars().collect::<Vec<_>>();
    let mut groups: Vec<char> = Vec::new();
    let mut at = 0;

    let mut repeatable = false;
    while at < chars.len() {
        match chars[at] {
            '\\' => {
                at += 1;
                if at == chars.len() {
                    return false;
                }
                at += 1;
                repeatable = true;
            }
            '[' => {
                at += 1;
                if chars.get(at) == Some(&'^') {
                    at += 1;
                }
                loop {
                    match chars.get(at) {
                        None => return false,
                        Some(']') => {
                            at += 1;
                            break;
                        }
                        Some('\\') => {
                            at += 1;
                            if at == chars.len() {
                                return false;
                            }
                            at += 1;
                        }
                        Some(_) => at += 1,
                    }
                }
                repeatable = true;
            }
            '(' => {
                at += 1;
                if chars.get(at) == Some(&'?')
                    && matches!(chars.get(at + 1), Some(':' | '=' | '!' | '<' | 'P'))
                {
                    at += 2;
                }
                groups.push('(');
                repeatable = false;
            }
            ')' => {
                if groups.pop() != Some('(') {
                    return false;
                }
                at += 1;
                repeatable = true;
            }
            '{' => {
                if !repeatable {
                    return false;
                }
                at += 1;
                let digits = at;
                while chars.get(at).is_some_and(char::is_ascii_digit) {
                    at += 1;
                }
                if at == digits {
                    return false;
                }
                if chars.get(at) == Some(&',') {
                    at += 1;
                    while chars.get(at).is_some_and(char::is_ascii_digit) {
                        at += 1;
                    }
                }
                if chars.get(at) != Some(&'}') {
                    return false;
                }
                at += 1;
                if chars.get(at) == Some(&'?') {
                    at += 1;
                }
                repeatable = false;
            }
            '}' => return false,
            '*' | '+' | '?' => {
                if !repeatable {
                    return false;
                }
                at += 1;
                if chars.get(at) == Some(&'?') {
                    at += 1;
                }
                repeatable = false;
            }
            '|' => {
                at += 1;
                repeatable = false;
            }
            _ => {
                at += 1;
                repeatable = true;
            }
        }
    }
    groups.is_empty()
}

fn not_a_struct_member(ty: &TypeRef) -> EmitError {
    let name = match ty {
        TypeRef::Named(named) => match named.variant.as_deref() {
            Some(variant) => format!("the {variant} variant of the type {}", named.id),
            None => format!("the type {}", named.id),
        },
        TypeRef::Inline(InlineType::Map { .. }) => "an inline map".into(),
        TypeRef::Inline(_) => "an inline type".into(),
    };
    EmitError(format!(
        "{name} cannot be inlined into a struct because only struct members are representable"
    ))
}

fn openness_value<'a>(ty: &'a TypeRef, document: &'a Document) -> &'a TypeRef {
    let mut current = ty;
    let mut visited = Vec::new();
    loop {
        match current {
            TypeRef::Inline(InlineType::Map { value, .. }) => return value,
            TypeRef::Named(named) if !visited.contains(&named.id) => {
                let Some(TypeKind::Newtype { inner }) = document
                    .types
                    .get(&named.id)
                    .map(|definition| &definition.definition)
                else {
                    return current;
                };
                visited.push(named.id.clone());
                current = inner;
            }
            _ => return current,
        }
    }
}

fn tagged_schema(tag: &str, variant: &str) -> Value {
    let mut properties = Map::new();
    properties.insert(tag.into(), json!({"const":variant}));
    json!({"type":"object","properties":properties,"required":[tag]})
}

fn tagged(mut object: Value, tag: &str, variant: &str) -> Value {
    object["properties"][tag] = json!({"const":variant});
    match object["required"].as_array_mut() {
        Some(required) => {
            if !required.iter().any(|name| name.as_str() == Some(tag)) {
                required.push(json!(tag));
            }
        }
        None => object["required"] = json!([tag]),
    }
    object
}

fn fields_schema_with<'a>(
    builder: &Builder<'_>,
    fields: impl IntoIterator<Item = &'a Field>,
) -> Result<Value, EmitError> {
    let mut properties = Map::new();
    let mut required = Vec::new();
    for field in fields {
        let mut schema = builder.field_schema(field)?;
        properties.insert(field.wire.clone(), schema.take());

        if field.presence == Presence::Required
            && !matches!(field.access, Some(Access::ReadOnly | Access::WriteOnly))
        {
            required.push(field.wire.clone());
        }
    }
    let mut object = json!({"type":"object","properties":properties});
    if !required.is_empty() {
        object["required"] = json!(required);
    }
    Ok(object)
}

#[cfg(test)]
mod tests {
    use pons_ir::{
        Access, ByteEncoding, DecimalWire, Direction, EnumRepr, Field, FieldFormat, InlineType,
        IntegerWire, NamedRef, ObjectOpenness, Presence, TypeDef, TypeKind, TypeRef, Variant,
        VariantPayload,
    };
    use serde_json::{Value, json};

    fn fixture() -> pons_ir::Document {
        serde_json::from_str(include_str!("../../../spec/fixtures/login.json")).unwrap()
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

    fn enum_type(name: &str) -> TypeDef {
        TypeDef {
            name: vec![name.into()],
            direction: Direction::Both,
            definition: TypeKind::Enum {
                repr: EnumRepr::External,
                variants: vec![Variant {
                    wire: "x".into(),
                    ident: vec!["x".into()],
                    payload: VariantPayload::Unit,
                    doc: None,
                    diagnostics: Vec::new(),
                    ext: Default::default(),
                }],
            },
            origin: None,
            doc: None,
            diagnostics: Vec::new(),
            ext: Default::default(),
        }
    }

    fn map() -> TypeRef {
        TypeRef::Inline(InlineType::Map {
            key: Box::new(string()),
            value: Box::new(string()),
        })
    }

    fn bytes(encoding: ByteEncoding) -> TypeRef {
        TypeRef::Inline(InlineType::Bytes { encoding })
    }

    fn patterned(pattern: &str) -> TypeRef {
        TypeRef::Inline(InlineType::String {
            format: None,
            min_length: None,
            max_length: None,
            pattern: Some(pattern.into()),
        })
    }

    fn formatted(format: FieldFormat) -> TypeRef {
        TypeRef::Inline(InlineType::String {
            format: Some(format),
            min_length: None,
            max_length: None,
            pattern: None,
        })
    }

    fn directional(wire: &str, access: Access) -> Field {
        Field {
            access: Some(access),
            default: pons_ir::FieldDefault::Value(json!("x")),
            ..field(wire, string())
        }
    }

    fn internal(name: &str, tag: &str, variants: Vec<(&str, VariantPayload)>) -> TypeDef {
        TypeDef {
            name: vec![name.into()],
            direction: Direction::Both,
            definition: TypeKind::Enum {
                repr: EnumRepr::Internal { tag: tag.into() },
                variants: variants
                    .into_iter()
                    .map(|(wire, payload)| Variant {
                        wire: wire.into(),
                        ident: vec![wire.into()],
                        payload,
                        doc: None,
                        diagnostics: Vec::new(),
                        ext: Default::default(),
                    })
                    .collect(),
            },
            origin: None,
            doc: None,
            diagnostics: Vec::new(),
            ext: Default::default(),
        }
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

    fn decimal(wire: DecimalWire) -> TypeRef {
        TypeRef::Inline(InlineType::Decimal { wire })
    }

    fn float(bits: u8) -> TypeRef {
        TypeRef::Inline(InlineType::Float { bits })
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
    fn flattened_members_inline_their_properties() {
        let mut document = fixture();
        document.types.insert(
            "Inner".into(),
            closed("inner", vec![field("b", integer(16, false))], Vec::new()),
        );
        document.types.insert(
            "Outer".into(),
            closed("outer", vec![field("a", string())], vec![named("Inner")]),
        );
        let output: Value = serde_json::from_str(&crate::emit(&document).unwrap()).unwrap();

        assert_eq!(
            output["components"]["schemas"]["Inner"]["additionalProperties"],
            json!(false)
        );
        let outer = &output["components"]["schemas"]["Outer"];
        assert_eq!(outer["unevaluatedProperties"], json!(false));
        assert!(outer.get("additionalProperties").is_none(), "{outer:#}");
        let all_of = outer["allOf"].as_array().unwrap();
        assert!(all_of[0].get("unevaluatedProperties").is_none());
        assert_eq!(all_of[0]["properties"]["a"]["type"], "string");
        assert_eq!(all_of[1]["properties"]["b"]["type"], "integer");
        assert_eq!(all_of[1]["required"], json!(["b"]));
        assert!(all_of[1].get("$ref").is_none(), "{:#}", all_of[1]);
        assert!(
            all_of[1].get("additionalProperties").is_none(),
            "{:#}",
            all_of[1]
        );
        assert!(
            all_of[1].get("unevaluatedProperties").is_none(),
            "{:#}",
            all_of[1]
        );
    }

    #[test]
    fn flattened_transitive_members_stay_inline() {
        let mut document = fixture();
        document.types.insert(
            "Leaf".into(),
            closed("leaf", vec![field("c", integer(16, false))], Vec::new()),
        );
        document.types.insert(
            "Inner".into(),
            closed(
                "inner",
                vec![field("b", integer(16, false))],
                vec![named("Leaf")],
            ),
        );
        document.types.insert(
            "Outer".into(),
            closed("outer", Vec::new(), vec![named("Inner")]),
        );
        let output: Value = serde_json::from_str(&crate::emit(&document).unwrap()).unwrap();
        let outer = &output["components"]["schemas"]["Outer"];
        assert_eq!(outer["unevaluatedProperties"], json!(false));
        let all_of = outer["allOf"].as_array().unwrap();
        assert_eq!(all_of[1]["properties"]["b"]["type"], "integer");
        assert_eq!(all_of[2]["properties"]["c"]["type"], "integer");
        for entry in all_of.iter().skip(1) {
            assert!(entry.get("additionalProperties").is_none(), "{entry:#}");
        }
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
    fn a_flattened_struct_validates_the_payloads_the_typescript_type_accepts() {
        let mut document = fixture();
        document.types.insert(
            "Inner".into(),
            closed("inner", vec![field("b", integer(16, false))], Vec::new()),
        );
        document.types.insert(
            "Outer".into(),
            closed("outer", vec![field("a", string())], vec![named("Inner")]),
        );

        assert!(validates(&document, "Outer", json!({"a":"x","b":2})));
        assert!(!validates(&document, "Outer", json!({"a":"x","b":2,"c":3})));
        assert!(!validates(&document, "Outer", json!({"a":"x"})));
    }

    #[test]
    fn a_flattened_struct_keeps_typing_the_extra_keys_of_a_typed_openness() {
        let mut document = fixture();
        document.types.insert(
            "Inner".into(),
            closed("inner", vec![field("b", integer(16, false))], Vec::new()),
        );
        document.types.insert(
            "Outer".into(),
            struct_type(
                "outer",
                vec![field("a", string())],
                vec![named("Inner")],
                ObjectOpenness::PreserveTyped { value: string() },
            ),
        );

        assert!(validates(&document, "Outer", json!({"a":"x","b":2})));
        assert!(validates(
            &document,
            "Outer",
            json!({"a":"x","b":2,"c":"extra"})
        ));
        assert!(!validates(&document, "Outer", json!({"a":"x","b":2,"c":3})));
    }

    #[test]
    fn rejects_a_flattened_map() {
        let mut document = fixture();
        document.types.insert(
            "Outer".into(),
            closed("outer", vec![field("a", string())], vec![map()]),
        );
        assert!(crate::emit(&document).unwrap_err().0.contains(
            "is not a struct, because a flattened member contributes its fields to one object"
        ));
    }

    #[test]
    fn rejects_a_flattened_enum() {
        let mut document = fixture();
        document.types.insert("Choice".into(), enum_type("choice"));
        document.types.insert(
            "Outer".into(),
            closed("outer", vec![field("a", string())], vec![named("Choice")]),
        );
        assert!(crate::emit(&document).unwrap_err().0.contains(
            "is not a struct, because a flattened member contributes its fields to one object"
        ));
    }

    #[test]
    fn rejects_a_cyclic_flatten() {
        let mut document = fixture();
        document.types.insert(
            "Outer".into(),
            closed("outer", vec![field("a", string())], vec![named("Inner")]),
        );
        document.types.insert(
            "Inner".into(),
            closed("inner", vec![field("b", string())], vec![named("Outer")]),
        );
        assert!(
            crate::emit(&document)
                .unwrap_err()
                .0
                .contains("flattened members form a cycle")
        );
    }

    #[test]
    fn rejects_a_cyclic_flatten_through_a_newtype() {
        let mut document = fixture();
        document
            .types
            .insert("Alias".into(), newtype("alias", named("Inner")));
        document.types.insert(
            "Outer".into(),
            closed("outer", vec![field("a", string())], vec![named("Alias")]),
        );
        document.types.insert(
            "Inner".into(),
            closed("inner", vec![field("b", string())], vec![named("Alias")]),
        );
        assert!(
            crate::emit(&document)
                .unwrap_err()
                .0
                .contains("cannot be inlined because flattened members form a cycle")
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
    fn emits_a_newtype_that_resolves_to_a_struct() {
        let mut document = fixture();
        document
            .types
            .insert("Alias".into(), newtype("alias", named("User")));
        let output: Value = serde_json::from_str(&crate::emit(&document).unwrap()).unwrap();
        assert_eq!(
            output["components"]["schemas"]["Alias"]["$ref"],
            json!("#/components/schemas/User")
        );
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
    fn a_directional_field_is_never_required_of_the_payload_that_drops_it() {
        let mut document = fixture();
        document.types.insert(
            "Account".into(),
            closed(
                "account",
                vec![
                    directional("id", Access::ReadOnly),
                    directional("secret", Access::WriteOnly),
                    field("name", string()),
                ],
                Vec::new(),
            ),
        );
        document.types.insert(
            "Envelope".into(),
            closed(
                "envelope",
                vec![field("kind", string())],
                vec![named("Account")],
            ),
        );
        let output: Value = serde_json::from_str(&crate::emit(&document).unwrap()).unwrap();
        let account = &output["components"]["schemas"]["Account"];
        assert_eq!(account["required"], json!(["name"]));
        assert_eq!(account["properties"]["id"]["readOnly"], true);
        assert_eq!(account["properties"]["secret"]["writeOnly"], true);
        let all_of = output["components"]["schemas"]["Envelope"]["allOf"]
            .as_array()
            .unwrap();
        assert_eq!(all_of[1]["required"], json!(["name"]));

        for instance in [
            json!({"name":"ada"}),
            json!({"id":"7","name":"ada"}),
            json!({"secret":"s","name":"ada"}),
            json!({"id":"7","secret":"s","name":"ada"}),
        ] {
            assert!(
                validates(&document, "Account", instance.clone()),
                "the document rejects {instance}"
            );
        }
        assert!(!validates(&document, "Account", json!({"id":"7"})));
        assert!(!validates(
            &document,
            "Account",
            json!({"name":"ada","id":7})
        ));
        assert!(validates(
            &document,
            "Envelope",
            json!({"kind":"k","name":"ada"})
        ));
        assert!(!validates(
            &document,
            "Envelope",
            json!({"kind":"k","id":"7"})
        ));
    }

    #[test]
    fn an_internally_tagged_newtype_payload_composes_with_its_tag() {
        for (id, definition) in [
            (
                "Detail",
                closed("detail", vec![field("reason", string())], Vec::new()),
            ),
            (
                "Typed",
                struct_type(
                    "typed",
                    vec![field("reason", string())],
                    Vec::new(),
                    ObjectOpenness::PreserveTyped { value: string() },
                ),
            ),
            (
                "Open",
                struct_type(
                    "open",
                    vec![field("reason", string())],
                    Vec::new(),
                    ObjectOpenness::PreserveOpaque,
                ),
            ),
        ] {
            let mut document = fixture();
            document.types.insert(id.into(), definition);
            document.types.insert(
                "Notice".into(),
                internal(
                    "notice",
                    "code",
                    vec![
                        ("stop", VariantPayload::Newtype { ty: named(id) }),
                        ("quiet", VariantPayload::Unit),
                    ],
                ),
            );

            let output: Value = serde_json::from_str(&crate::emit(&document).unwrap()).unwrap();
            let branches = output["components"]["schemas"]["Notice"]["oneOf"]
                .as_array()
                .unwrap();
            let stop = branches
                .iter()
                .find(|schema| schema["properties"]["code"]["const"] == "stop")
                .unwrap();
            assert!(stop.get("allOf").is_none(), "{stop:#}");
            assert!(stop.get("$ref").is_none(), "{stop:#}");
            assert_eq!(stop["properties"]["reason"]["type"], "string");
            assert_eq!(stop["required"], json!(["reason", "code"]));

            assert!(validates(
                &document,
                "Notice",
                json!({"reason":"x","code":"stop"})
            ));
            assert!(validates(&document, "Notice", json!({"code":"quiet"})));
            assert!(
                !validates(&document, "Notice", json!({"code":"stop"})),
                "the payload fields are still required"
            );
            assert_eq!(
                validates(
                    &document,
                    "Notice",
                    json!({"reason":"x","code":"stop","extra":1})
                ),
                id == "Open",
                "the payload's own openness still judges the keys beside the tag"
            );
        }
    }

    #[test]
    fn a_tag_named_like_a_payload_field_is_refused_before_emission() {
        for presence in [Presence::Required, Presence::Optional] {
            let mut detail = field("code", string());
            detail.presence = presence;
            let mut document = fixture();
            document
                .types
                .insert("Detail".into(), closed("detail", vec![detail], Vec::new()));
            document.types.insert(
                "Notice".into(),
                internal(
                    "notice",
                    "code",
                    vec![(
                        "stop",
                        VariantPayload::Newtype {
                            ty: named("Detail"),
                        },
                    )],
                ),
            );
            let refusal = crate::emit(&document).expect_err(
                "a tag that collides with a payload field through a newtype must not emit",
            );
            assert!(refusal.0.contains("reserved enum field"), "{}", refusal.0);
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

    #[test]
    fn refuses_a_pattern_the_document_cannot_compile() {
        fn id_pattern(document: &mut pons_ir::Document, pattern: &str) {
            let TypeKind::Struct { fields, .. } =
                &mut document.types.get_mut("User").unwrap().definition
            else {
                unreachable!()
            };
            fields[0].ty = patterned(pattern);
        }
        let mut document = fixture();

        id_pattern(&mut document, "[a-");
        let error = crate::emit(&document).unwrap_err().0;
        assert!(error.contains("[a-"), "{error}");

        id_pattern(&mut document, "a{");
        assert!(document.validate().is_ok(), "the document itself is legal");
        let error = crate::emit(&document).unwrap_err().0;
        assert!(error.contains("a{"), "{error}");

        id_pattern(&mut document, "^[a-z0-9_-]{2,}$");
        let output: Value = serde_json::from_str(&crate::emit(&document).unwrap()).unwrap();
        assert_eq!(
            output["components"]["schemas"]["User"]["properties"]["id"]["pattern"],
            json!("^[a-z0-9_-]{2,}$")
        );
    }
}
