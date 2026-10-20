use std::any::type_name;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use pons_ir::{
    Diagnostic, DiagnosticSeverity, Direction, Doc, InlineType, IntegerWire, Name, NamedRef,
    ObjectOpenness, TypeDef, TypeKind, TypeRef,
};

pub use pons_derive::{ApiError, Schema};
pub use pons_ir as ir;
pub use serde_json;

#[cfg(any(
    feature = "bytes",
    feature = "chrono",
    feature = "rust_decimal",
    feature = "serde_json",
    feature = "time",
    feature = "uuid"
))]
mod foreign;

pub trait Schema {
    fn schema(cx: &mut TypeCx, direction: Direction) -> TypeRef;
}

pub trait ErrorSpec {
    fn errors(cx: &mut TypeCx) -> Vec<pons_ir::ErrorResponse>;
}

#[derive(Debug, Default)]
pub struct TypeCx {
    definitions: BTreeMap<String, TypeDef>,
    registered: BTreeSet<(String, DirectionKey)>,
    diagnostics: Vec<Diagnostic>,
    building: Vec<String>,
    pending: BTreeMap<String, Vec<Diagnostic>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum DirectionKey {
    Input,
    Output,
    Both,
}

impl From<Direction> for DirectionKey {
    fn from(direction: Direction) -> Self {
        match direction {
            Direction::Input => Self::Input,
            Direction::Output => Self::Output,
            Direction::Both => Self::Both,
        }
    }
}

impl From<DirectionKey> for Direction {
    fn from(key: DirectionKey) -> Self {
        match key {
            DirectionKey::Input => Self::Input,
            DirectionKey::Output => Self::Output,
            DirectionKey::Both => Self::Both,
        }
    }
}

impl TypeCx {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn named<T: ?Sized>(
        &mut self,
        name: Name,
        direction: Direction,
        doc: Option<Doc>,
        build: impl FnOnce(&mut Self) -> TypeKind,
    ) -> TypeRef {
        let base = type_name::<T>().to_owned();
        let id = match direction {
            Direction::Both => base.clone(),
            Direction::Input => format!("{base}#input"),
            Direction::Output => format!("{base}#output"),
        };

        let origin = Some(base.clone());
        self.define((base, direction.into()), id, origin, name, doc, build)
    }

    pub fn register_as(
        &mut self,
        id: impl Into<String>,
        schema: impl FnOnce(&mut Self) -> TypeRef,
    ) -> TypeRef {
        let id = id.into();
        let reference = schema(self);
        let TypeRef::Named(source) = &reference else {
            return reference;
        };
        let Some(definition) = self.definitions.get(&source.id).cloned() else {
            return reference;
        };
        let direction = definition.direction;
        let origin = match direction {
            Direction::Both => Some(id.clone()),
            _ => Some(definition.origin.clone().unwrap_or_else(|| id.clone())),
        };
        self.define(
            (id.clone(), direction.into()),
            id,
            origin,
            definition.name,
            definition.doc,
            move |_| definition.definition,
        )
    }

    fn define(
        &mut self,
        key: (String, DirectionKey),
        id: String,
        origin: Option<String>,
        name: Name,
        doc: Option<Doc>,
        build: impl FnOnce(&mut Self) -> TypeKind,
    ) -> TypeRef {
        let direction = key.1.into();
        if self.registered.insert(key) {
            self.building.push(id.clone());
            let definition = build(self);
            self.building.pop();
            let diagnostics = self.pending.remove(&id).unwrap_or_default();
            self.definitions.insert(
                id.clone(),
                TypeDef {
                    name,
                    direction,
                    definition,
                    origin,
                    doc,
                    diagnostics,
                    ext: BTreeMap::new(),
                },
            );
        }
        TypeRef::Named(NamedRef { id, variant: None })
    }

    pub fn definitions(&self) -> &BTreeMap<String, TypeDef> {
        &self.definitions
    }

    pub fn flattened_fields(&self, ty: &TypeRef) -> Option<(Vec<pons_ir::Field>, Vec<TypeRef>)> {
        let TypeRef::Named(reference) = ty else {
            return None;
        };
        let definition = self.definitions.get(&reference.id)?;
        let TypeKind::Struct {
            fields,
            flattened,
            openness,
        } = &definition.definition
        else {
            return None;
        };
        if matches!(openness, ObjectOpenness::PreserveTyped { .. }) {
            return None;
        }
        Some((fields.clone(), flattened.clone()))
    }

    pub fn is_flattenable_carrier(&self, ty: &TypeRef) -> bool {
        let mut current = ty;
        let mut seen = BTreeSet::new();
        loop {
            let TypeRef::Named(reference) = current else {
                return matches!(current, TypeRef::Inline(InlineType::Opaque { .. }));
            };
            if reference.variant.is_some() || !seen.insert(reference.id.clone()) {
                return false;
            }
            match self
                .definitions
                .get(&reference.id)
                .map(|definition| &definition.definition)
            {
                Some(TypeKind::Struct { .. }) => return true,
                Some(TypeKind::Newtype { inner }) => current = inner,
                _ => return false,
            }
        }
    }

    pub fn into_definitions(self) -> BTreeMap<String, TypeDef> {
        self.definitions
    }

    pub fn push_diagnostic(&mut self, code: &str, message: impl Into<String>) {
        self.push(DiagnosticSeverity::Warning, code, message);
    }

    pub fn push_type_diagnostic(&mut self, code: &str, message: impl Into<String>) {
        let diagnostic = Diagnostic {
            severity: DiagnosticSeverity::Warning,
            code: code.to_owned(),
            message: message.into(),
            origin: None,
            ext: BTreeMap::new(),
        };
        match self.building.last() {
            Some(id) => {
                self.pending.entry(id.clone()).or_default().push(diagnostic);
            }
            None => self.diagnostics.push(diagnostic),
        }
    }

    pub fn push_error(&mut self, code: &str, message: impl Into<String>) {
        self.push(DiagnosticSeverity::Error, code, message);
    }

    fn push(&mut self, severity: DiagnosticSeverity, code: &str, message: impl Into<String>) {
        let message = message.into();
        if self
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == code && diagnostic.message == message)
        {
            return;
        }
        self.diagnostics.push(Diagnostic {
            severity,
            code: code.to_owned(),
            message,
            origin: None,
            ext: BTreeMap::new(),
        });
    }

    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    pub fn into_diagnostics(self) -> Vec<Diagnostic> {
        self.diagnostics
    }

    pub fn into_parts(self) -> (BTreeMap<String, TypeDef>, Vec<Diagnostic>) {
        (self.definitions, self.diagnostics)
    }
}

macro_rules! integer_schema {
    ($ty:ty, $bits:expr, $signed:expr) => {
        impl Schema for $ty {
            fn schema(_: &mut TypeCx, _: Direction) -> TypeRef {
                TypeRef::Inline(InlineType::Int {
                    bits: $bits,
                    signed: $signed,
                    wire: IntegerWire::Number,
                    minimum: None,
                    maximum: None,
                    multiple_of: None,
                })
            }
        }
    };
}

integer_schema!(i8, 8, true);
integer_schema!(i16, 16, true);
integer_schema!(i32, 32, true);
integer_schema!(i64, 64, true);
integer_schema!(i128, 128, true);
integer_schema!(u8, 8, false);
integer_schema!(u16, 16, false);
integer_schema!(u32, 32, false);
integer_schema!(u64, 64, false);
integer_schema!(u128, 128, false);
integer_schema!(usize, 64, false);
integer_schema!(isize, 64, true);

impl Schema for bool {
    fn schema(_: &mut TypeCx, _: Direction) -> TypeRef {
        TypeRef::Inline(InlineType::Bool)
    }
}

impl Schema for f32 {
    fn schema(_: &mut TypeCx, _: Direction) -> TypeRef {
        TypeRef::Inline(InlineType::Float { bits: 32 })
    }
}

impl Schema for f64 {
    fn schema(_: &mut TypeCx, _: Direction) -> TypeRef {
        TypeRef::Inline(InlineType::Float { bits: 64 })
    }
}

impl Schema for String {
    fn schema(_: &mut TypeCx, _: Direction) -> TypeRef {
        string_schema()
    }
}

impl Schema for str {
    fn schema(_: &mut TypeCx, _: Direction) -> TypeRef {
        string_schema()
    }
}

impl<T: Schema + ?Sized> Schema for &T {
    fn schema(cx: &mut TypeCx, direction: Direction) -> TypeRef {
        T::schema(cx, direction)
    }
}

fn string_schema() -> TypeRef {
    TypeRef::Inline(InlineType::String {
        format: None,
        min_length: None,
        max_length: None,
        pattern: None,
    })
}

impl Schema for () {
    fn schema(_: &mut TypeCx, _: Direction) -> TypeRef {
        TypeRef::Inline(InlineType::Null)
    }
}

impl<T: Schema> Schema for Option<T> {
    fn schema(cx: &mut TypeCx, direction: Direction) -> TypeRef {
        TypeRef::Inline(InlineType::Nullable {
            inner: Box::new(T::schema(cx, direction)),
        })
    }
}

impl<T: Schema> Schema for Vec<T> {
    fn schema(cx: &mut TypeCx, direction: Direction) -> TypeRef {
        TypeRef::Inline(InlineType::Array {
            item: Box::new(T::schema(cx, direction)),
            unique: None,
        })
    }
}

impl<T: Schema> Schema for BTreeSet<T> {
    fn schema(cx: &mut TypeCx, direction: Direction) -> TypeRef {
        TypeRef::Inline(InlineType::Array {
            item: Box::new(T::schema(cx, direction)),
            unique: Some(true),
        })
    }
}

impl<T: Schema> Schema for HashSet<T> {
    fn schema(cx: &mut TypeCx, direction: Direction) -> TypeRef {
        <BTreeSet<T> as Schema>::schema(cx, direction)
    }
}

impl<T: Schema> Schema for Box<T> {
    fn schema(cx: &mut TypeCx, direction: Direction) -> TypeRef {
        T::schema(cx, direction)
    }
}

impl<T: Schema, const N: usize> Schema for [T; N] {
    fn schema(cx: &mut TypeCx, direction: Direction) -> TypeRef {
        TypeRef::Inline(InlineType::Tuple {
            items: (0..N).map(|_| T::schema(cx, direction)).collect(),
        })
    }
}

macro_rules! tuple_schema {
    ($($type:ident),+ $(,)?) => {
        impl<$($type: Schema),+> Schema for ($($type,)+) {
            fn schema(cx: &mut TypeCx, direction: Direction) -> TypeRef {
                TypeRef::Inline(InlineType::Tuple {
                    items: vec![$($type::schema(cx, direction)),+],
                })
            }
        }
    };
}

tuple_schema!(A);
tuple_schema!(A, B);
tuple_schema!(A, B, C);
tuple_schema!(A, B, C, D);

impl<T: Schema> Schema for BTreeMap<String, T> {
    fn schema(cx: &mut TypeCx, direction: Direction) -> TypeRef {
        map_schema::<T>(cx, direction)
    }
}

impl<T: Schema> Schema for HashMap<String, T> {
    fn schema(cx: &mut TypeCx, direction: Direction) -> TypeRef {
        map_schema::<T>(cx, direction)
    }
}

fn map_schema<T: Schema>(cx: &mut TypeCx, direction: Direction) -> TypeRef {
    TypeRef::Inline(InlineType::Map {
        key: Box::new(TypeRef::Inline(InlineType::String {
            format: None,
            min_length: None,
            max_length: None,
            pattern: None,
        })),
        value: Box::new(T::schema(cx, direction)),
    })
}

impl Schema for char {
    fn schema(_: &mut TypeCx, _: Direction) -> TypeRef {
        TypeRef::Inline(InlineType::String {
            format: None,
            min_length: Some(1),
            max_length: Some(1),
            pattern: None,
        })
    }
}
