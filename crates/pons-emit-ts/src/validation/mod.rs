mod builtin;
mod zod;

use std::collections::{BTreeMap, BTreeSet};

use pons_ir::{Direction, Document, TypeKind, TypeRef};

use crate::{EmitError, GeneratedFile, TsOptions, ValidationBackend, pascal, unique_names};

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
    fn validate(&mut self, reference: &TypeRef) -> Result<String, EmitError>;
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
            ValidationBackend::Builtin => {
                Some(Box::new(builtin::Generator::new(document, names, options)))
            }
            backend => {
                return Err(EmitError(format!(
                    "the {} validation backend is not implemented yet",
                    backend.name()
                )));
            }
        };
        Ok(Backend { generator })
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

pub(crate) fn declaration_names(
    document: &Document,
    names: &BTreeMap<String, String>,
) -> BTreeMap<(String, Option<String>), String> {
    let mut keys = Vec::new();
    let mut bases = Vec::new();
    for (id, definition) in &document.types {
        let name = &names[id];
        if let TypeKind::Enum { variants, .. } = &definition.definition {
            for variant in variants {
                keys.push((id.clone(), Some(variant.wire.clone())));
                bases.push(format!("{name}{}", pascal(&variant.ident)));
            }
        }
        keys.push((id.clone(), None));
        bases.push(name.clone());
    }
    keys.into_iter().zip(unique_names(bases, &[])).collect()
}

pub(crate) fn validator_names(
    document: &Document,
    declarations: &BTreeMap<(String, Option<String>), String>,
    reserved: &[&str],
) -> BTreeMap<(String, Option<String>), String> {
    let mut keys = Vec::new();
    let mut bases = Vec::new();
    for (key, name) in declarations {
        let input_only = document
            .types
            .get(&key.0)
            .is_some_and(|definition| definition.direction == Direction::Input);
        if input_only {
            continue;
        }
        keys.push(key.clone());
        bases.push(format!("validate{name}"));
    }
    keys.into_iter()
        .zip(unique_names(bases, reserved))
        .collect()
}
