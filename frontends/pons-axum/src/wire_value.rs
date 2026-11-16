use pons_ir::{DecimalWire, Document, InlineType, IntegerWire, TypeKind, TypeRef, Violation};
use serde_json::Value;

pub(crate) fn object_value(
    document: &Document,
    ty: &TypeRef,
    pairs: Vec<(String, String)>,
) -> (Value, Vec<Violation>) {
    let mut declared = Vec::new();
    members(document, ty, &mut declared, &mut Vec::new());
    let mut entries = serde_json::Map::new();
    let mut violations = Vec::new();
    for (wire, texts) in grouped(pairs) {
        let field = declared
            .iter()
            .find(|(name, _)| name == &wire)
            .map(|(_, field)| *field);
        let value = match field.map(|field| (field, array_item(document, field))) {
            Some((field, Some(_))) => array_value(document, field, &texts),
            Some((field, _)) if texts.len() > 1 => {
                violations.push(Violation::Type {
                    path: pointer(&wire),
                    expected: "one value for a field that is not an array".to_owned(),
                    found: format!("{} values", texts.len()),
                });
                token_value(document, field, &texts[texts.len() - 1])
            }
            Some((field, _)) => token_value(document, field, &texts[0]),
            None => Value::String(texts[texts.len() - 1].clone()),
        };
        entries.insert(wire, value);
    }
    (Value::Object(entries), violations)
}

fn array_value(document: &Document, field: &TypeRef, texts: &[String]) -> Value {
    let item = array_item(document, field).expect("an array field spells its item");
    Value::Array(
        texts
            .iter()
            .flat_map(|text| match spells_array(text) {
                Some(items) => items,
                None => vec![item_value(document, item, text)],
            })
            .collect(),
    )
}

fn item_value(document: &Document, item: &TypeRef, text: &str) -> Value {
    if carries_json(document, item) {
        return serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.to_owned()));
    }
    token_value(document, item, text)
}

pub(crate) fn token_value(document: &Document, ty: &TypeRef, text: &str) -> Value {
    if let Some(item) = array_item(document, ty) {
        return Value::Array(match serde_json::from_str::<Value>(text) {
            Ok(Value::Array(items)) => items,
            _ => vec![token_value(document, item, text)],
        });
    }
    match ty {
        TypeRef::Named(named) => {
            match document.types.get(&named.id).map(|found| &found.definition) {
                Some(TypeKind::Newtype { inner }) => token_value(document, inner, text),
                _ => Value::String(text.to_owned()),
            }
        }
        TypeRef::Inline(InlineType::Nullable { inner }) => token_value(document, inner, text),
        TypeRef::Inline(InlineType::Bool) => match text {
            "true" => Value::Bool(true),
            "false" => Value::Bool(false),
            _ => Value::String(text.to_owned()),
        },
        TypeRef::Inline(
            InlineType::Int {
                wire: IntegerWire::Number,
                ..
            }
            | InlineType::Float { .. }
            | InlineType::Decimal {
                wire: DecimalWire::Number,
            },
        ) => match serde_json::from_str::<Value>(text) {
            Ok(number @ Value::Number(_)) => number,
            _ => Value::String(text.to_owned()),
        },
        _ => Value::String(text.to_owned()),
    }
}

fn array_item<'a>(document: &'a Document, ty: &'a TypeRef) -> Option<&'a TypeRef> {
    match ty {
        TypeRef::Named(named) => match document.types.get(&named.id).map(|found| &found.definition)
        {
            Some(TypeKind::Newtype { inner }) => array_item(document, inner),
            _ => None,
        },
        TypeRef::Inline(InlineType::Nullable { inner }) => array_item(document, inner),
        TypeRef::Inline(InlineType::Array { item, .. }) => Some(item),
        _ => None,
    }
}

fn spells_array(text: &str) -> Option<Vec<Value>> {
    match serde_json::from_str::<Value>(text) {
        Ok(Value::Array(items)) => Some(items),
        _ => None,
    }
}

fn carries_json(document: &Document, ty: &TypeRef) -> bool {
    match ty {
        TypeRef::Inline(InlineType::Any | InlineType::Map { .. } | InlineType::Tuple { .. }) => {
            true
        }
        TypeRef::Inline(InlineType::Nullable { inner }) => carries_json(document, inner),
        TypeRef::Named(named) => match document.types.get(&named.id).map(|found| &found.definition)
        {
            Some(TypeKind::Struct { .. }) => true,
            Some(TypeKind::Newtype { inner }) => carries_json(document, inner),
            _ => false,
        },
        _ => false,
    }
}

fn grouped(pairs: Vec<(String, String)>) -> Vec<(String, Vec<String>)> {
    let mut groups: Vec<(String, Vec<String>)> = Vec::new();
    for (wire, text) in pairs {
        match groups.iter_mut().find(|(name, _)| name == &wire) {
            Some((_, texts)) => texts.push(text),
            None => groups.push((wire, vec![text])),
        }
    }
    groups
}

fn members<'a>(
    document: &'a Document,
    ty: &'a TypeRef,
    found: &mut Vec<(String, &'a TypeRef)>,
    seen: &mut Vec<String>,
) {
    let TypeRef::Named(named) = ty else {
        return;
    };
    if seen.contains(&named.id) {
        return;
    }
    let Some(definition) = document.types.get(&named.id).map(|found| &found.definition) else {
        return;
    };
    seen.push(named.id.clone());
    match definition {
        TypeKind::Newtype { inner } => members(document, inner, found, seen),
        TypeKind::Struct {
            fields, flattened, ..
        } => {
            found.extend(fields.iter().map(|field| (field.wire.clone(), &field.ty)));
            flattened
                .iter()
                .for_each(|ty| members(document, ty, found, seen));
        }
        TypeKind::Enum { .. } => {}
    }
    seen.pop();
}

pub(crate) fn path_value(document: &Document, ty: &TypeRef, pairs: Vec<(String, String)>) -> Value {
    match shape(document, ty) {
        Shape::Struct => object_value(document, ty, pairs).0,
        Shape::Tuple(items) => Value::Array(
            pairs
                .iter()
                .enumerate()
                .map(|(index, (_, text))| match items.get(index) {
                    Some(item) => token_value(document, item, text),
                    None => Value::String(text.clone()),
                })
                .collect(),
        ),
        Shape::Single => match pairs.first() {
            Some((_, text)) => token_value(document, ty, text),
            None => Value::Null,
        },
    }
}

enum Shape<'a> {
    Struct,
    Tuple(&'a [TypeRef]),
    Single,
}

fn shape<'a>(document: &'a Document, ty: &'a TypeRef) -> Shape<'a> {
    match ty {
        TypeRef::Inline(InlineType::Tuple { items }) => Shape::Tuple(items),
        TypeRef::Named(named) => match document.types.get(&named.id).map(|found| &found.definition)
        {
            Some(TypeKind::Newtype { inner }) => shape(document, inner),
            Some(TypeKind::Struct { .. }) => Shape::Struct,
            _ => Shape::Single,
        },
        _ => Shape::Single,
    }
}

fn pointer(wire: &str) -> String {
    format!("/{}", wire.replace('~', "~0").replace('/', "~1"))
}
