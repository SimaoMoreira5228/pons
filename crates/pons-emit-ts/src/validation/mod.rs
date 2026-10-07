mod builtin;

use std::collections::{BTreeMap, BTreeSet};

use pons_ir::{Document, TypeRef};

use crate::{EmitError, GeneratedFile, TsOptions, Validation, schemas};

#[derive(Clone, Copy)]
pub(crate) struct Capabilities {
    pub(crate) module: &'static str,
}

pub(crate) struct Rendered {
    pub(crate) file: &'static str,
    pub(crate) content: String,
}

#[derive(Default)]
pub(crate) struct Imports {
    pub(crate) prefix: Option<&'static str>,
    pub(crate) names: BTreeSet<String>,
}

impl Imports {
    pub(crate) fn is_empty(&self) -> bool {
        self.prefix.is_none() && self.names.is_empty()
    }

    pub(crate) fn merge(&mut self, other: Imports) {
        self.prefix = self.prefix.or(other.prefix);
        self.names.extend(other.names);
    }
}

pub(super) trait Generator {
    fn capabilities(&self) -> Capabilities;
    fn validate(&mut self, reference: &TypeRef) -> Result<String, EmitError>;
    fn take_imports(&mut self) -> Imports;
    fn render(&mut self) -> Result<Rendered, EmitError>;
}

struct Zod<'a> {
    schemas: schemas::Generator<'a>,
    mismatch: bool,
}

impl<'a> Zod<'a> {
    fn new(
        document: &'a Document,
        names: &'a BTreeMap<String, String>,
        options: &'a TsOptions,
    ) -> Self {
        Zod {
            schemas: schemas::Generator::new(document, names, options),
            mismatch: false,
        }
    }
}

impl Generator for Zod<'_> {
    fn capabilities(&self) -> Capabilities {
        Capabilities { module: "schemas" }
    }

    fn validate(&mut self, reference: &TypeRef) -> Result<String, EmitError> {
        let expression = schemas::Generator::validate(&mut self.schemas, reference)?;
        self.mismatch = true;
        Ok(format!("mismatch({expression})"))
    }

    fn take_imports(&mut self) -> Imports {
        Imports {
            prefix: std::mem::take(&mut self.mismatch).then_some("mismatch"),
            names: schemas::Generator::take_imports(&mut self.schemas),
        }
    }

    fn render(&mut self) -> Result<Rendered, EmitError> {
        Ok(Rendered {
            file: "schemas.ts",
            content: schemas::Generator::render(&mut self.schemas)?,
        })
    }
}

pub(crate) struct Backend<'a> {
    generator: Option<Box<dyn Generator + 'a>>,
}

impl<'a> Backend<'a> {
    pub(crate) fn new(
        document: &'a Document,
        names: &'a BTreeMap<String, String>,
        options: &'a TsOptions,
    ) -> Self {
        let generator: Option<Box<dyn Generator + 'a>> = match options.validation {
            Validation::None => None,
            Validation::Zod => Some(Box::new(Zod::new(document, names, options))),
            Validation::Builtin => {
                Some(Box::new(builtin::Generator::new(document, names, options)))
            }
        };
        Backend { generator }
    }

    pub(crate) fn validate(&mut self, reference: &TypeRef) -> Result<Option<String>, EmitError> {
        match self.generator.as_mut() {
            Some(generator) => generator.validate(reference).map(Some),
            None => Ok(None),
        }
    }

    pub(crate) fn take_imports(&mut self) -> Imports {
        match self.generator.as_mut() {
            Some(generator) => generator.take_imports(),
            None => Imports::default(),
        }
    }

    pub(crate) fn capabilities(&self) -> Option<Capabilities> {
        self.generator
            .as_ref()
            .map(|generator| generator.capabilities())
    }

    pub(crate) fn render(&mut self) -> Result<Option<GeneratedFile>, EmitError> {
        let Some(generator) = self.generator.as_mut() else {
            return Ok(None);
        };
        let rendered = generator.render()?;
        Ok(Some(GeneratedFile {
            path: rendered.file.into(),
            content: rendered.content,
        }))
    }
}
