use std::collections::{BTreeMap, BTreeSet};

use pons_ir::{Direction, Document, TypeKind, TypeRef, ValidationCoverage};

mod schema;

use super::{
    Capabilities, Generator as ValidationGenerator, Imports, Rendered, declaration_names,
    validator_names,
};
use crate::{EmitError, TsOptions};

const MISMATCH: &str = "function mismatch(schema: z.ZodType): (value: unknown) => ValidationIssue | null {\n  return (value) => {\n    const result = schema.safeParse(value);\n    if (result.success) return null;\n    const issue = result.error.issues[0];\n    if (issue === undefined) return { path: [], message: \"the value does not match the contract\" };\n    return { path: issue.path.map((step) => typeof step === \"number\" ? step : String(step)), message: issue.message };\n  };\n}\n";

struct Inline {
    name: String,
    expression: String,
}

pub(super) struct Generator<'a> {
    document: &'a Document,
    options: &'a TsOptions,
    schemas: schema::Generator<'a>,
    validators: BTreeMap<(String, Option<String>), String>,
    imports: BTreeSet<String>,
    exported: BTreeSet<String>,
    inlines: BTreeMap<String, Inline>,
    inline_count: usize,
}

impl<'a> Generator<'a> {
    pub(super) fn new(
        document: &'a Document,
        names: &'a BTreeMap<String, String>,
        options: &'a TsOptions,
    ) -> Self {
        let declarations = declaration_names(document, names);
        let validators = validator_names(document, &declarations, &[]);
        let exported = validators.values().cloned().collect();
        Generator {
            document,
            options,
            schemas: schema::Generator::new(document, names, options),
            validators,
            imports: BTreeSet::new(),
            exported,
            inlines: BTreeMap::new(),
            inline_count: 0,
        }
    }

    fn validator(&self, id: &str, variant: Option<&str>) -> Result<String, EmitError> {
        self.validators
            .get(&(id.to_owned(), variant.map(str::to_owned)))
            .cloned()
            .ok_or_else(|| match variant {
                Some(variant) => EmitError(format!("unknown enum variant {variant} of type {id}")),
                None => EmitError(format!("unknown validator for type {id}")),
            })
    }

    fn inline_validator(&mut self, reference: &TypeRef) -> Result<String, EmitError> {
        let json = crate::client::type_ref_json(reference, self.options)?;
        if let Some(inline) = self.inlines.get(&json) {
            return Ok(inline.name.clone());
        }
        let name = loop {
            self.inline_count += 1;
            let candidate = format!("validateInline{}", self.inline_count);
            if self.exported.insert(candidate.clone()) {
                break candidate;
            }
        };
        let expression = self.schemas.expression(reference)?;
        self.inlines.insert(
            json,
            Inline {
                name: name.clone(),
                expression,
            },
        );
        Ok(name)
    }
}

impl ValidationGenerator for Generator<'_> {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            module: "validators",
        }
    }

    fn coverage(&self, document: &Document) -> ValidationCoverage {
        ValidationCoverage::exact_for_document(document)
    }

    fn has_bindings(&self) -> bool {
        !self.validators.is_empty() || !self.inlines.is_empty()
    }

    fn validate(&mut self, reference: &TypeRef) -> Result<String, EmitError> {
        let name = match reference {
            TypeRef::Named(named) => self.validator(&named.id, named.variant.as_deref())?,
            TypeRef::Inline(_) => self.inline_validator(reference)?,
        };
        self.imports.insert(name.clone());
        Ok(name)
    }

    fn take_imports(&mut self) -> Imports {
        Imports {
            names: std::mem::take(&mut self.imports),
        }
    }

    fn render(&mut self) -> Result<Rendered, EmitError> {
        let mut body = self.schemas.render_body()?;
        body.push_str(MISMATCH);
        for (id, definition) in self.document.types.iter() {
            if definition.direction == Direction::Input {
                continue;
            }
            match &definition.definition {
                TypeKind::Enum { variants, .. } => {
                    for variant in variants {
                        self.push_binding(&mut body, id, Some(&variant.wire))?;
                    }
                    self.push_binding(&mut body, id, None)?;
                }
                _ => self.push_binding(&mut body, id, None)?,
            }
        }
        for inline in self.inlines.values() {
            body.push_str(&format!(
                "export const {}: Validator = mismatch({});\n",
                inline.name, inline.expression
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
                crate::string(package)
            ),
        })
    }
}

impl Generator<'_> {
    fn push_binding(
        &self,
        body: &mut String,
        id: &str,
        variant: Option<&str>,
    ) -> Result<(), EmitError> {
        let name = self.validator(id, variant)?;
        let schema = self.schemas.schema_name(id, variant)?;
        body.push_str(&format!(
            "export const {name}: Validator = mismatch({schema});\n"
        ));
        Ok(())
    }
}
