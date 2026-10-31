use std::any::type_name;
use std::collections::{BTreeMap, BTreeSet};

use pons_core::TypeCx;
use pons_ir::{
    Access, Direction, Doc, EnumRepr, Field, FieldDefault, InlineType, Name, ObjectOpenness,
    OpaqueWire, Presence, TypeKind, TypeRef, Variant, VariantPayload,
};
use schemars::generate::{SchemaGenerator, SchemaSettings};
use schemars::{JsonSchema, Schema};
use serde_json::{Map, Value};

use crate::scalars::{DROPPED_KEYWORD, lower_integer, lower_number, lower_string, string_type};

const UNEXPRESSIBLE: &str = "PONS-W202";

pub(crate) fn lower<T>(cx: &mut TypeCx, direction: Direction) -> TypeRef
where
    T: JsonSchema + ?Sized,
{
    let settings = match direction {
        Direction::Input => SchemaSettings::draft2020_12().for_deserialize(),
        Direction::Output => SchemaSettings::draft2020_12().for_serialize(),
        Direction::Both => SchemaSettings::draft2020_12(),
    };
    let mut generator = SchemaGenerator::new(settings);
    let schema: Schema = generator.root_schema_for::<T>();
    let mut value = schema.to_value();
    let defs = take_definitions(&mut value);
    let base = type_name::<T>().to_owned();
    let root_id = scoped_id(&base, direction);
    let context = Context {
        base: &base,
        root_id: &root_id,
        direction,
        defs: &defs,
    };
    let name = words(&T::schema_name());
    let doc = doc_of(&value);
    cx.named_as(
        root_id.clone(),
        Some(base.clone()),
        name,
        direction,
        doc,
        move |cx| lower_named(cx, &context, &value),
    )
}

#[derive(Clone, Copy)]
struct Context<'a> {
    base: &'a str,
    root_id: &'a str,
    direction: Direction,
    defs: &'a Map<String, Value>,
}

fn take_definitions(value: &mut Value) -> Map<String, Value> {
    let Value::Object(object) = value else {
        return Map::new();
    };
    let mut defs = Map::new();
    for key in ["$defs", "definitions"] {
        let Some(Value::Object(found)) = object.remove(key) else {
            continue;
        };
        for (name, schema) in found {
            defs.entry(name).or_insert(schema);
        }
    }
    defs
}

fn lower_named(cx: &mut TypeCx, context: &Context<'_>, schema: &Value) -> TypeKind {
    let Some(object) = schema.as_object() else {
        return TypeKind::Newtype {
            inner: lower_ref(cx, context, schema),
        };
    };
    if let Some(inner) = composite(cx, context, object) {
        return TypeKind::Newtype { inner };
    }
    if let Some(variants) = string_enum(object) {
        return TypeKind::Enum {
            repr: EnumRepr::String,
            variants,
        };
    }
    if names_fields(object) {
        return lower_struct(cx, context, object);
    }
    TypeKind::Newtype {
        inner: lower_ref(cx, context, schema),
    }
}

fn composite(
    cx: &mut TypeCx,
    context: &Context<'_>,
    object: &Map<String, Value>,
) -> Option<TypeRef> {
    if object.contains_key("discriminator") {
        return Some(opaque(
            cx,
            "json-schema:discriminator",
            "a discriminator selects a variant at runtime and has no Pons enum representation",
        ));
    }
    for keyword in ["oneOf", "anyOf"] {
        if let Some(branches) = object.get(keyword) {
            return Some(lower_branches(cx, context, keyword, branches));
        }
    }
    if let Some(branches) = object.get("allOf") {
        return Some(lower_all_of(cx, context, branches));
    }
    None
}

fn lower_branches(
    cx: &mut TypeCx,
    context: &Context<'_>,
    keyword: &str,
    branches: &Value,
) -> TypeRef {
    let Some(branches) = branches.as_array() else {
        return opaque(
            cx,
            &format!("json-schema:{keyword}"),
            format!("{keyword} must be an array of schemas"),
        );
    };
    if let Some(inner) = nullable_branch(branches) {
        return TypeRef::Inline(InlineType::Nullable {
            inner: Box::new(lower_ref(cx, context, inner)),
        });
    }
    opaque(
        cx,
        &format!("json-schema:{keyword}"),
        format!(
            "{keyword} with {} branches has no Pons union representation",
            branches.len()
        ),
    )
}

fn nullable_branch(branches: &[Value]) -> Option<&Value> {
    let [left, right] = branches else {
        return None;
    };
    match (is_null_schema(left), is_null_schema(right)) {
        (true, false) => Some(right),
        (false, true) => Some(left),
        _ => None,
    }
}

fn is_null_schema(schema: &Value) -> bool {
    schema
        .as_object()
        .is_some_and(|object| object.get("type").and_then(Value::as_str) == Some("null"))
}

fn lower_all_of(cx: &mut TypeCx, context: &Context<'_>, branches: &Value) -> TypeRef {
    let Some(branches) = branches.as_array() else {
        return opaque(cx, "json-schema:allOf", "allOf must be an array of schemas");
    };
    if let [branch] = branches.as_slice()
        && branch.get("$ref").and_then(Value::as_str).is_some()
    {
        return lower_ref(cx, context, branch);
    }
    opaque(
        cx,
        "json-schema:allOf",
        format!(
            "allOf with {} branches composes schemas and has no Pons type representation",
            branches.len()
        ),
    )
}

fn names_fields(object: &Map<String, Value>) -> bool {
    object
        .get("properties")
        .and_then(Value::as_object)
        .is_some_and(|properties| !properties.is_empty())
        || (object.get("type").and_then(Value::as_str) == Some("object")
            && matches!(object.get("additionalProperties"), Some(Value::Bool(false))))
}

fn lower_struct(cx: &mut TypeCx, context: &Context<'_>, object: &Map<String, Value>) -> TypeKind {
    let required: BTreeSet<&str> = object
        .get("required")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let mut fields = Vec::new();
    if let Some(properties) = object.get("properties").and_then(Value::as_object) {
        for (wire, schema) in properties {
            fields.push(lower_field(
                cx,
                context,
                wire,
                schema,
                required.contains(wire.as_str()),
            ));
        }
    }
    let openness = match object.get("additionalProperties") {
        Some(Value::Bool(false)) => ObjectOpenness::Reject,
        Some(value @ Value::Object(_)) => ObjectOpenness::PreserveTyped {
            value: lower_ref(cx, context, value),
        },
        _ => ObjectOpenness::PreserveOpaque,
    };
    TypeKind::Struct {
        fields,
        flattened: Vec::new(),
        openness,
    }
}

fn lower_field(
    cx: &mut TypeCx,
    context: &Context<'_>,
    wire: &str,
    schema: &Value,
    required: bool,
) -> Field {
    let access = access_of(schema);
    let presence = if required {
        Presence::Required
    } else {
        Presence::Optional
    };
    let default = match schema.get("default") {
        Some(_) if required && !matches!(access, Some(Access::ReadOnly | Access::WriteOnly)) => {
            cx.push_type_diagnostic(
                DROPPED_KEYWORD,
                format!(
                    "field \"{wire}\" is always present, so a default on it has no Pons meaning and is dropped"
                ),
            );
            FieldDefault::Unspecified
        }
        Some(value) => FieldDefault::Value(value.clone()),
        None => FieldDefault::Unspecified,
    };
    Field {
        wire: wire.to_owned(),
        ident: words(wire),
        ty: lower_ref(cx, context, schema),
        presence,
        default,
        access,
        doc: doc_of(schema),
        diagnostics: Vec::new(),
        ext: BTreeMap::new(),
    }
}

fn access_of(schema: &Value) -> Option<Access> {
    if schema.get("readOnly").and_then(Value::as_bool) == Some(true) {
        return Some(Access::ReadOnly);
    }
    if schema.get("writeOnly").and_then(Value::as_bool) == Some(true) {
        return Some(Access::WriteOnly);
    }
    None
}

fn lower_ref(cx: &mut TypeCx, context: &Context<'_>, schema: &Value) -> TypeRef {
    match schema {
        Value::Bool(true) => TypeRef::Inline(InlineType::Any),
        Value::Bool(false) => opaque(
            cx,
            "json-schema:never",
            "a false schema accepts no value, which Pons cannot express",
        ),
        Value::Object(object) => lower_object_ref(cx, context, object),
        _ => opaque(
            cx,
            "json-schema:value",
            "a JSON Schema must be an object or a boolean",
        ),
    }
}

fn lower_object_ref(
    cx: &mut TypeCx,
    context: &Context<'_>,
    object: &Map<String, Value>,
) -> TypeRef {
    if let Some(reference) = object.get("$ref").and_then(Value::as_str) {
        return resolve_ref(cx, context, reference);
    }
    if let Some(ty) = composite(cx, context, object) {
        return ty;
    }
    if object.contains_key("enum") {
        return inline_enum(cx, context, object);
    }
    let Some(shape) = type_shape(object) else {
        if object.get("type").is_some() {
            return opaque(
                cx,
                "json-schema:type-list",
                "the schema lists several non-null JSON types, which Pons has no union for",
            );
        }
        if object.keys().all(|key| {
            matches!(
                key.as_str(),
                "$comment"
                    | "$id"
                    | "$schema"
                    | "$defs"
                    | "definitions"
                    | "default"
                    | "deprecated"
                    | "description"
                    | "examples"
                    | "title"
            )
        }) {
            return TypeRef::Inline(InlineType::Any);
        }
        return opaque(
            cx,
            "json-schema:untyped",
            "the schema states no type and no construct Pons can lower",
        );
    };
    let ty = match shape.name {
        "object" => lower_object_type(cx, context, object),
        "array" => lower_array(cx, context, object),
        "string" => lower_string(cx, object),
        "integer" => lower_integer(cx, object),
        "number" => lower_number(object),
        "boolean" => TypeRef::Inline(InlineType::Bool),
        "null" => TypeRef::Inline(InlineType::Null),
        other => opaque(
            cx,
            &format!("json-schema:type:{other}"),
            format!("the JSON type \"{other}\" has no Pons representation"),
        ),
    };
    if shape.nullable {
        TypeRef::Inline(InlineType::Nullable {
            inner: Box::new(ty),
        })
    } else {
        ty
    }
}

struct Shape<'a> {
    name: &'a str,
    nullable: bool,
}

fn type_shape(object: &Map<String, Value>) -> Option<Shape<'_>> {
    match object.get("type")? {
        Value::String(name) => Some(Shape {
            name,
            nullable: false,
        }),
        Value::Array(types) => {
            let names: Vec<&str> = types.iter().filter_map(Value::as_str).collect();
            if names.len() != types.len() {
                return None;
            }
            if names == ["null"] {
                return Some(Shape {
                    name: "null",
                    nullable: false,
                });
            }
            let mut non_null = names.iter().copied().filter(|name| *name != "null");
            let name = non_null.next()?;
            if non_null.next().is_some() {
                return None;
            }
            Some(Shape {
                name,
                nullable: names.len() > 1,
            })
        }
        _ => None,
    }
}

fn lower_object_type(
    cx: &mut TypeCx,
    context: &Context<'_>,
    object: &Map<String, Value>,
) -> TypeRef {
    if object
        .get("properties")
        .and_then(Value::as_object)
        .is_some_and(|properties| !properties.is_empty())
    {
        return opaque(
            cx,
            "json-schema:inline-object",
            "an object with named properties has no inline Pons form and no definition name here",
        );
    }
    match object.get("additionalProperties") {
        Some(value @ Value::Object(_)) => TypeRef::Inline(InlineType::Map {
            key: Box::new(string_type()),
            value: Box::new(lower_ref(cx, context, value)),
        }),
        Some(Value::Bool(true)) | None => TypeRef::Inline(InlineType::Map {
            key: Box::new(string_type()),
            value: Box::new(TypeRef::Inline(InlineType::Any)),
        }),
        Some(Value::Bool(false)) => opaque(
            cx,
            "json-schema:empty-object",
            "an inline object that rejects every key has no Pons form",
        ),
        Some(_) => opaque(
            cx,
            "json-schema:additionalProperties",
            "additionalProperties must be a schema or a boolean",
        ),
    }
}

fn lower_array(cx: &mut TypeCx, context: &Context<'_>, object: &Map<String, Value>) -> TypeRef {
    if let Some(prefix) = object.get("prefixItems").and_then(Value::as_array) {
        let fixed = object.get("maxItems").and_then(Value::as_u64) == Some(prefix.len() as u64)
            && object
                .get("items")
                .is_none_or(|items| matches!(items, Value::Bool(false)));
        if !fixed {
            return opaque(
                cx,
                "json-schema:tuple",
                "prefixItems without a fixed length describes a variadic tuple, which Pons cannot express",
            );
        }
        return TypeRef::Inline(InlineType::Tuple {
            items: prefix
                .iter()
                .map(|item| lower_ref(cx, context, item))
                .collect(),
        });
    }
    if matches!(object.get("items"), Some(Value::Array(_))) {
        return opaque(
            cx,
            "json-schema:tuple",
            "an items array is the draft-07 tuple form, which Pons cannot express",
        );
    }
    let item = match object.get("items") {
        Some(Value::Bool(true)) | None => TypeRef::Inline(InlineType::Any),
        Some(Value::Bool(false)) => opaque(
            cx,
            "json-schema:never",
            "items: false accepts no item, which Pons cannot express",
        ),
        Some(value @ Value::Object(_)) => lower_ref(cx, context, value),
        Some(_) => opaque(
            cx,
            "json-schema:items",
            "items must be a schema or a boolean",
        ),
    };
    TypeRef::Inline(InlineType::Array {
        item: Box::new(item),
        unique: object.get("uniqueItems").and_then(Value::as_bool),
    })
}

fn string_enum(object: &Map<String, Value>) -> Option<Vec<Variant>> {
    let Value::Array(values) = object.get("enum")? else {
        return None;
    };
    let is_string = match object.get("type") {
        None => true,
        Some(Value::String(name)) => name == "string",
        Some(_) => false,
    };
    if !is_string {
        return None;
    }
    let mut variants = Vec::new();
    for value in values {
        let Value::String(wire) = value else {
            return None;
        };
        variants.push(Variant {
            wire: wire.clone(),
            ident: words(wire),
            payload: VariantPayload::Unit,
            doc: None,
            diagnostics: Vec::new(),
            ext: BTreeMap::new(),
        });
    }
    Some(variants)
}

fn inline_enum(cx: &mut TypeCx, context: &Context<'_>, object: &Map<String, Value>) -> TypeRef {
    let Some(title) = object.get("title").and_then(Value::as_str) else {
        return opaque(
            cx,
            "json-schema:enum",
            "a string enum appears inline without a title, so it has no stable definition name",
        );
    };
    let id = member_id(context.base, title, context.direction);
    let origin = member_origin(context.base, title);
    let name = words(title);
    let schema = Value::Object(object.clone());
    let doc = doc_of(&schema);
    let context = *context;
    cx.named_as(id, Some(origin), name, context.direction, doc, move |cx| {
        lower_named(cx, &context, &schema)
    })
}

fn resolve_ref(cx: &mut TypeCx, context: &Context<'_>, reference: &str) -> TypeRef {
    if reference == "#" {
        return TypeRef::Named(pons_ir::NamedRef {
            id: context.root_id.to_owned(),
            variant: None,
        });
    }
    let Some(name) = definition_name(reference) else {
        return opaque(
            cx,
            "json-schema:ref",
            format!("the reference \"{reference}\" does not point into this schema's definitions"),
        );
    };
    define_definition(cx, context, name)
}

fn definition_name(reference: &str) -> Option<&str> {
    let pointer = reference.strip_prefix('#')?;
    let name = pointer
        .strip_prefix("/$defs/")
        .or_else(|| pointer.strip_prefix("/definitions/"))?;
    (!name.is_empty() && !name.contains('/')).then_some(name)
}

fn define_definition(cx: &mut TypeCx, context: &Context<'_>, name: &str) -> TypeRef {
    let Some(schema) = context.defs.get(name) else {
        return opaque(
            cx,
            "json-schema:ref",
            format!("the reference names \"{name}\", which is missing from the definitions"),
        );
    };
    let id = member_id(context.base, name, context.direction);
    let origin = member_origin(context.base, name);
    let display = schema.get("title").and_then(Value::as_str).unwrap_or(name);
    let display_name = words(display);
    let doc = doc_of(schema);
    let schema = schema.clone();
    let context = *context;
    cx.named_as(
        id,
        Some(origin),
        display_name,
        context.direction,
        doc,
        move |cx| lower_named(cx, &context, &schema),
    )
}

fn opaque(cx: &mut TypeCx, codec: &str, message: impl Into<String>) -> TypeRef {
    cx.push_type_diagnostic(UNEXPRESSIBLE, message);
    TypeRef::Inline(InlineType::Opaque {
        codec: codec.to_owned(),
        wire: OpaqueWire::Json,
        ext: BTreeMap::new(),
    })
}

fn doc_of(schema: &Value) -> Option<Doc> {
    let object = schema.as_object()?;
    let description = object
        .get("description")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let deprecated = object.get("deprecated").and_then(Value::as_bool);
    if description.is_none() && deprecated.is_none() {
        return None;
    }
    Some(Doc {
        summary: None,
        description,
        deprecated,
        tags: Vec::new(),
    })
}

fn scoped_id(path: &str, direction: Direction) -> String {
    match direction {
        Direction::Both => path.to_owned(),
        Direction::Input => format!("{path}#input"),
        Direction::Output => format!("{path}#output"),
    }
}

fn member_id(base: &str, name: &str, direction: Direction) -> String {
    scoped_id(&member_origin(base, name), direction)
}

fn member_origin(base: &str, name: &str) -> String {
    format!("{base}::{name}")
}

fn words(source: &str) -> Name {
    let mut result = Vec::new();
    let mut current = String::new();
    let chars: Vec<char> = source.chars().collect();
    for (index, character) in chars.iter().copied().enumerate() {
        if !character.is_alphanumeric() {
            if !current.is_empty() {
                result.push(std::mem::take(&mut current));
            }
            continue;
        }
        if character.is_uppercase() && !current.is_empty() {
            let previous = chars
                .get(index.wrapping_sub(1))
                .copied()
                .unwrap_or_default();
            let next = chars.get(index + 1).copied().unwrap_or_default();
            if previous.is_lowercase() || next.is_lowercase() && previous.is_uppercase() {
                result.push(std::mem::take(&mut current));
            }
        }
        current.extend(character.to_lowercase());
    }
    if !current.is_empty() {
        result.push(current);
    }
    if result.is_empty() {
        result.push("value".to_owned());
    }
    result
}
