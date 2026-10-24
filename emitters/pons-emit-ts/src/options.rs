#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum EmitMode {
    #[default]
    Strict,
    Lossy,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ErrorStrategy {
    #[default]
    Neverthrow,
    Throw,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Namespaces {
    #[default]
    Nested,
    Flat,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FieldCase {
    #[default]
    Wire,
    Camel,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Newtypes {
    #[default]
    Alias,
    Branded,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Int64 {
    #[default]
    Bigint,
    String,
    Number,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DateMode {
    #[default]
    String,
    Date,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Runtime {
    #[default]
    Vendor,
    Package,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ValidationBackend {
    #[default]
    None,
    Builtin,
    Zod,
    Valibot,
    Arktype,
    Typebox,
    Yup,
    Superstruct,
}

impl ValidationBackend {
    pub fn name(self) -> &'static str {
        match self {
            ValidationBackend::None => "none",
            ValidationBackend::Builtin => "builtin",
            ValidationBackend::Zod => "zod",
            ValidationBackend::Valibot => "valibot",
            ValidationBackend::Arktype => "arktype",
            ValidationBackend::Typebox => "typebox",
            ValidationBackend::Yup => "yup",
            ValidationBackend::Superstruct => "superstruct",
        }
    }

    pub fn has_package(self) -> bool {
        matches!(
            self,
            ValidationBackend::Zod
                | ValidationBackend::Valibot
                | ValidationBackend::Arktype
                | ValidationBackend::Typebox
                | ValidationBackend::Yup
                | ValidationBackend::Superstruct
        )
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ValidationOptions {
    pub backend: ValidationBackend,
    pub import_from: Option<String>,
}

impl ValidationOptions {
    pub fn new(backend: ValidationBackend) -> Self {
        ValidationOptions {
            backend,
            import_from: None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TsOptions {
    pub mode: EmitMode,
    pub errors: ErrorStrategy,
    pub namespaces: Namespaces,
    pub field_case: FieldCase,
    pub newtypes: Newtypes,
    pub int64: Int64,
    pub date_mode: DateMode,
    pub runtime: Runtime,
    pub validation: ValidationOptions,
}
