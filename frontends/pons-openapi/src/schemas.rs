use std::collections::{BTreeMap, BTreeSet};

use pons_core::TypeCx;
use pons_ir::{
    ByteEncoding, Diagnostic, Direction, InlineType, ObjectOpenness, TypeDef, TypeKind, TypeRef,
    VariantPayload,
};
use pons_json_schema::{RootType, lower};
use serde_json::{Map, Value};

use crate::dialect::Dialect;

const SCHEMA_PREFIX: &str = "#/components/schemas/";
const DEFS_PREFIX: &str = "#/$defs/";

pub(crate) struct TypeLowering {
    components: Map<String, Value>,
    dialect: Dialect,
    types: BTreeMap<String, TypeDef>,
    active: BTreeSet<String>,
    diagnostics: Vec<Diagnostic>,
}

impl TypeLowering {
    pub(crate) fn new(mut components: Map<String, Value>, dialect: Dialect) -> Self {
        let mut diagnostics = Vec::new();
        for (name, schema) in components.iter_mut() {
            dialect.translate(
                schema,
                &format!("components.schemas.{name}"),
                &mut diagnostics,
            );
        }
        Self {
            components,
            dialect,
            types: BTreeMap::new(),
            active: BTreeSet::new(),
            diagnostics,
        }
    }

    pub(crate) fn into_parts(self) -> (BTreeMap<String, TypeDef>, Vec<Diagnostic>) {
        (self.types, self.diagnostics)
    }

    pub(crate) fn schema_type(
        &mut self,
        id: &str,
        name: &str,
        schema: &Value,
        direction: Direction,
    ) -> TypeRef {
        match component_ref(schema) {
            Some(component) if self.components.contains_key(component) => {
                self.component(component, direction)
            }
            _ => self.inline(id, name, schema, direction),
        }
    }

    pub(crate) fn component(&mut self, name: &str, direction: Direction) -> TypeRef {
        let id = scoped(&component_id(name), direction);
        if self.types.contains_key(&id) || !self.active.insert(id.clone()) {
            return named(&id);
        }
        let schema = self
            .components
            .get(name)
            .cloned()
            .unwrap_or(Value::Bool(true));
        let root = self.lower_prepared(&component_id(name), name, &schema, direction);
        self.active.remove(&id);
        root
    }

    pub(crate) fn inline(
        &mut self,
        id: &str,
        name: &str,
        schema: &Value,
        direction: Direction,
    ) -> TypeRef {
        self.lower_root(id, name, schema, direction)
    }

    pub(crate) fn payload(
        &mut self,
        id: &str,
        name: &str,
        schema: Option<&Value>,
        bytes: bool,
        text: bool,
        direction: Direction,
    ) -> TypeRef {
        if bytes {
            return TypeRef::Inline(InlineType::Bytes {
                encoding: ByteEncoding::Base64,
            });
        }
        match schema {
            Some(schema) => self.schema_type(id, name, schema, direction),
            None if text => TypeRef::Inline(InlineType::String {
                format: None,
                min_length: None,
                max_length: None,
                pattern: None,
            }),
            None => TypeRef::Inline(InlineType::Any),
        }
    }

    fn lower_root(
        &mut self,
        id: &str,
        name: &str,
        schema: &Value,
        direction: Direction,
    ) -> TypeRef {
        let mut schema = schema.clone();
        self.dialect
            .translate(&mut schema, id, &mut self.diagnostics);
        self.lower_prepared(id, name, &schema, direction)
    }

    fn lower_prepared(
        &mut self,
        id: &str,
        name: &str,
        schema: &Value,
        direction: Direction,
    ) -> TypeRef {
        let prepared = prepare(schema, &self.components);
        let mut cx = TypeCx::new();
        let root = lower(
            &mut cx,
            prepared,
            RootType {
                id,
                name,
                origin: Some(id),
                direction,
            },
        );
        let (definitions, referenced) =
            rewrite_definitions(cx.into_definitions(), id, direction, &self.components);
        for (key, definition) in definitions {
            self.types.entry(key).or_insert(definition);
        }
        for component in referenced {
            self.component(&component, direction);
        }
        root
    }
}

fn component_id(name: &str) -> String {
    format!("openapi::{name}")
}

fn scoped(id: &str, direction: Direction) -> String {
    match direction {
        Direction::Both => id.to_owned(),
        Direction::Input => format!("{id}#input"),
        Direction::Output => format!("{id}#output"),
    }
}

fn named(id: &str) -> TypeRef {
    TypeRef::Named(pons_ir::NamedRef {
        id: id.to_owned(),
        variant: None,
    })
}

fn component_ref(schema: &Value) -> Option<&str> {
    let object = schema.as_object()?;
    if object
        .keys()
        .any(|key| !matches!(key.as_str(), "$ref" | "description" | "summary"))
    {
        return None;
    }
    object.get("$ref")?.as_str()?.strip_prefix(SCHEMA_PREFIX)
}

fn prepare(schema: &Value, components: &Map<String, Value>) -> Value {
    let mut schema = schema.clone();
    rewrite_references(&mut schema);
    let Value::Object(object) = &mut schema else {
        return schema;
    };
    let mut definitions = Map::new();
    for (name, definition) in components {
        let mut definition = definition.clone();
        rewrite_references(&mut definition);
        definitions.insert(name.clone(), definition);
    }
    match object.get_mut("$defs") {
        Some(Value::Object(existing)) => {
            for (name, definition) in definitions {
                existing.entry(name).or_insert(definition);
            }
        }
        _ => {
            object.insert("$defs".into(), Value::Object(definitions));
        }
    }
    schema
}

fn rewrite_references(value: &mut Value) {
    match value {
        Value::Object(object) => {
            if let Some(Value::String(reference)) = object.get_mut("$ref")
                && let Some(name) = reference.strip_prefix(SCHEMA_PREFIX)
            {
                *reference = format!("{DEFS_PREFIX}{name}");
            }
            for value in object.values_mut() {
                rewrite_references(value);
            }
        }
        Value::Array(items) => items.iter_mut().for_each(rewrite_references),
        _ => {}
    }
}

fn rewrite_definitions(
    mut definitions: BTreeMap<String, TypeDef>,
    root: &str,
    direction: Direction,
    components: &Map<String, Value>,
) -> (BTreeMap<String, TypeDef>, BTreeSet<String>) {
    let prefix = format!("{root}::");
    let suffix = match direction {
        Direction::Input => "#input",
        Direction::Output => "#output",
        Direction::Both => "",
    };
    let mut ids = BTreeMap::new();
    let mut referenced = BTreeSet::new();
    for key in definitions.keys() {
        let Some(rest) = key.strip_prefix(&prefix) else {
            continue;
        };
        let Some(name) = rest.strip_suffix(suffix).filter(|name| !name.is_empty()) else {
            continue;
        };
        if components.contains_key(name) {
            ids.insert(key.clone(), scoped(&component_id(name), direction));
            referenced.insert(name.to_owned());
        }
    }
    for definition in definitions.values_mut() {
        rewrite_definition(definition, &ids);
    }
    for key in ids.keys() {
        definitions.remove(key);
    }
    (definitions, referenced)
}

fn rewrite_definition(definition: &mut TypeDef, ids: &BTreeMap<String, String>) {
    match &mut definition.definition {
        TypeKind::Struct {
            fields,
            flattened,
            openness,
        } => {
            for field in fields {
                rewrite_type_ref(&mut field.ty, ids);
            }
            for ty in flattened {
                rewrite_type_ref(ty, ids);
            }
            if let ObjectOpenness::PreserveTyped { value } = openness {
                rewrite_type_ref(value, ids);
            }
        }
        TypeKind::Enum { variants, .. } => {
            for variant in variants {
                match &mut variant.payload {
                    VariantPayload::Unit => {}
                    VariantPayload::Newtype { ty } => rewrite_type_ref(ty, ids),
                    VariantPayload::Tuple { items } => {
                        items.iter_mut().for_each(|ty| rewrite_type_ref(ty, ids));
                    }
                    VariantPayload::Struct { fields } => {
                        fields
                            .iter_mut()
                            .for_each(|field| rewrite_type_ref(&mut field.ty, ids));
                    }
                }
            }
        }
        TypeKind::Newtype { inner } => rewrite_type_ref(inner, ids),
    }
}

fn rewrite_type_ref(ty: &mut TypeRef, ids: &BTreeMap<String, String>) {
    match ty {
        TypeRef::Named(named) => {
            if let Some(target) = ids.get(&named.id) {
                named.id = target.clone();
            }
        }
        TypeRef::Inline(InlineType::Nullable { inner }) => rewrite_type_ref(inner, ids),
        TypeRef::Inline(InlineType::Array { item, .. }) => rewrite_type_ref(item, ids),
        TypeRef::Inline(InlineType::Map { key, value }) => {
            rewrite_type_ref(key, ids);
            rewrite_type_ref(value, ids);
        }
        TypeRef::Inline(InlineType::Tuple { items }) => {
            items.iter_mut().for_each(|ty| rewrite_type_ref(ty, ids));
        }
        TypeRef::Inline(_) => {}
    }
}
