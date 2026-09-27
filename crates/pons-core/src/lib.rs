use std::any::type_name;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use pons_ir::{
    Diagnostic, DiagnosticSeverity, Direction, Doc, InlineType, IntegerWire, Name, NamedRef,
    ObjectOpenness, TypeDef, TypeKind, TypeRef,
};

pub use pons_derive::{ApiError, Schema};
pub use pons_ir as ir;

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
        let key = (base.clone(), direction.into());
        let id = match direction {
            Direction::Both => base,
            Direction::Input => format!("{base}#input"),
            Direction::Output => format!("{base}#output"),
        };
        if self.registered.insert(key) {
            let definition = build(self);
            self.definitions.insert(
                id.clone(),
                TypeDef {
                    name,
                    direction,
                    definition,
                    origin: None,
                    doc,
                    diagnostics: Vec::new(),
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

    pub fn into_definitions(self) -> BTreeMap<String, TypeDef> {
        self.definitions
    }

    pub fn push_diagnostic(&mut self, code: &str, message: impl Into<String>) {
        let message = message.into();
        if self
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == code && diagnostic.message == message)
        {
            return;
        }
        self.diagnostics.push(Diagnostic {
            severity: DiagnosticSeverity::Warning,
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

impl Schema for serde_json::Value {
    fn schema(_: &mut TypeCx, _: Direction) -> TypeRef {
        TypeRef::Inline(InlineType::Any)
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
