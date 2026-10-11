mod arktype;
mod backend;
mod builtin;
mod typebox;
mod valibot;
mod walk;
mod yup;
mod zod;

use std::collections::{BTreeMap, BTreeSet};

use pons_ir::{Document, TypeRef, ValidationCoverage};

use crate::{EmitError, GeneratedFile, TsOptions, ValidationBackend};

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
    pub(crate) names: BTreeSet<String>,
}

impl Imports {
    pub(crate) fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    pub(crate) fn merge(&mut self, other: Imports) {
        self.names.extend(other.names);
    }
}

pub(super) trait Generator {
    fn capabilities(&self) -> Capabilities;
    fn coverage(&mut self, document: &Document) -> Result<ValidationCoverage, EmitError>;
    fn has_bindings(&self) -> bool;
    fn validate(&mut self, reference: &TypeRef, subject: &str) -> Result<String, EmitError>;
    fn take_imports(&mut self) -> Imports;
    fn render(&mut self) -> Result<Rendered, EmitError>;
}

pub(crate) struct Backend<'a> {
    generator: Option<Box<dyn Generator + 'a>>,
}

impl<'a> Backend<'a> {
    pub(crate) fn new(
        document: &'a Document,
        names: &'a BTreeMap<String, String>,
        options: &'a TsOptions,
    ) -> Result<Self, EmitError> {
        let generator: Option<Box<dyn Generator + 'a>> = match options.validation.backend {
            ValidationBackend::None => None,
            ValidationBackend::Zod => Some(Box::new(zod::Generator::new(document, names, options))),
            ValidationBackend::Valibot => {
                Some(Box::new(valibot::Generator::new(document, names, options)))
            }
            ValidationBackend::Arktype => {
                Some(Box::new(arktype::Generator::new(document, names, options)))
            }
            ValidationBackend::Builtin => {
                Some(Box::new(builtin::Generator::new(document, names, options)))
            }
            ValidationBackend::Typebox => {
                Some(Box::new(typebox::Generator::new(document, names, options)))
            }
            ValidationBackend::Yup => Some(Box::new(yup::Generator::new(document, names, options))),
            backend => {
                return Err(EmitError(format!(
                    "the {} validation backend is not implemented yet",
                    backend.name()
                )));
            }
        };
        Ok(Backend { generator })
    }

    pub(crate) fn validate(
        &mut self,
        reference: &TypeRef,
        subject: &str,
    ) -> Result<Option<String>, EmitError> {
        match self.generator.as_mut() {
            Some(generator) => generator.validate(reference, subject).map(Some),
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
            .filter(|generator| generator.has_bindings())
            .map(|generator| generator.capabilities())
    }

    pub(crate) fn coverage(
        &mut self,
        document: &Document,
    ) -> Result<ValidationCoverage, EmitError> {
        match self.generator.as_mut() {
            Some(generator) => generator.coverage(document),
            None => Ok(ValidationCoverage::default()),
        }
    }

    pub(crate) fn render(&mut self) -> Result<Option<GeneratedFile>, EmitError> {
        let Some(generator) = self.generator.as_mut() else {
            return Ok(None);
        };
        if !generator.has_bindings() {
            return Ok(None);
        }
        let rendered = generator.render()?;
        Ok(Some(GeneratedFile {
            path: rendered.file.into(),
            content: rendered.content,
        }))
    }
}
