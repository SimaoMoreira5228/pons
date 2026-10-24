use std::collections::BTreeSet;

use pons_ir::{Document, TypeRef, ValidationCoverage};

mod schema;

use super::backend::Backend as _;
use super::walk::Walk;
use super::{Capabilities, Generator as ValidationGenerator, Imports, Rendered};
use crate::{EmitError, TsOptions};

const MISMATCH: &str = "function mismatch(schema: z.ZodType): (value: unknown) => ValidationIssue | null {\n  return (value) => {\n    const result = schema.safeParse(value);\n    if (result.success) return null;\n    const issue = result.error.issues[0];\n    if (issue === undefined) return { path: [], message: \"the value does not match the contract\" };\n    return { path: issue.path.map((step) => typeof step === \"number\" ? step : String(step)), message: issue.message };\n  };\n}\n";

pub(super) struct Generator<'a> {
    options: &'a TsOptions,
    walk: Walk<'a>,
    schemas: schema::Generator,
    imports: BTreeSet<String>,
}

impl<'a> Generator<'a> {
    pub(super) fn new(
        document: &'a Document,
        names: &'a std::collections::BTreeMap<String, String>,
        options: &'a TsOptions,
    ) -> Self {
        Generator {
            options,
            walk: Walk::new(document, names, options, &[]),
            schemas: schema::Generator::new(),
            imports: BTreeSet::new(),
        }
    }
}

impl ValidationGenerator for Generator<'_> {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            module: "validators",
        }
    }

    fn coverage(&mut self, _document: &Document) -> Result<ValidationCoverage, EmitError> {
        self.walk.declarations(&mut self.schemas)?;
        Ok(self.walk.coverage())
    }

    fn has_bindings(&self) -> bool {
        self.walk.has_bindings()
    }

    fn validate(&mut self, reference: &TypeRef, subject: &str) -> Result<String, EmitError> {
        let name = self.walk.validate(&mut self.schemas, reference, subject)?;
        self.imports.insert(name.clone());
        Ok(name)
    }

    fn take_imports(&mut self) -> Imports {
        Imports {
            names: std::mem::take(&mut self.imports),
        }
    }

    fn render(&mut self) -> Result<Rendered, EmitError> {
        self.walk.declarations(&mut self.schemas)?;
        let mut body = self.walk.module(&mut self.schemas)?;
        body.push_str(MISMATCH);
        for binding in self.walk.bindings()? {
            body.push_str(&format!(
                "export const {}: Validator = {};\n",
                binding.name,
                self.schemas.validator(&binding.schema)
            ));
        }
        let specifier = crate::runtime::specifier(self.options.runtime, "\"./runtime.js\"");
        let package = self
            .options
            .validation
            .import_from
            .as_deref()
            .unwrap_or("zod");
        Ok(Rendered {
            file: "validators.ts",
            content: format!(
                "import {{ z }} from {};\n\
                 import type {{ ValidationIssue, Validator }} from {specifier};\n\
                 export type {{ ValidationIssue, Validator }} from {specifier};\n\n{body}",
                crate::render::string(package)
            ),
        })
    }
}
