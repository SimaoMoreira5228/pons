use std::collections::{BTreeMap, BTreeSet};

use pons_ir::{Direction, Document, NamedRef, TypeKind, TypeRef};

use super::{Capabilities, Generator as ValidationGenerator, Imports, Rendered, declaration_names};
use crate::{EmitError, TsOptions, jsdoc, unique_names};

pub(super) struct Generator<'a> {
    document: &'a Document,
    options: &'a TsOptions,
    validators: BTreeMap<(String, Option<String>), String>,
    exported: BTreeSet<String>,
    imports: BTreeSet<String>,
    inlines: BTreeMap<String, String>,
    inline_count: usize,
}

impl<'a> Generator<'a> {
    pub(super) fn new(
        document: &'a Document,
        names: &'a BTreeMap<String, String>,
        options: &'a TsOptions,
    ) -> Self {
        let declarations = declaration_names(document, names);
        let validator_names = validator_names(document, &declarations);
        let exported = validator_names.values().cloned().collect();
        Generator {
            document,
            options,
            validators: validator_names,
            exported,
            imports: BTreeSet::new(),
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
        if let Some(name) = self.inlines.get(&json) {
            return Ok(name.clone());
        }
        let name = loop {
            self.inline_count += 1;
            let candidate = format!("validateInline{}", self.inline_count);
            if self.exported.insert(candidate.clone()) {
                break candidate;
            }
        };
        self.inlines.insert(json, name.clone());
        Ok(name)
    }
}

impl ValidationGenerator for Generator<'_> {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            module: "validators",
        }
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
        let mut body = String::new();
        for (id, definition) in &self.document.types {
            if definition.direction == Direction::Input {
                continue;
            }
            match &definition.definition {
                TypeKind::Enum { variants, .. } => {
                    for variant in variants {
                        let name = self.validator(id, Some(&variant.wire))?;
                        let comment = jsdoc(variant.doc.as_ref());
                        if !comment.is_empty() {
                            body.push_str(&comment);
                            body.push('\n');
                        }
                        body.push_str(&binding(&name, &named_json(id, Some(&variant.wire))));
                    }
                    let name = self.validator(id, None)?;
                    let comment = jsdoc(definition.doc.as_ref());
                    if !comment.is_empty() {
                        body.push_str(&comment);
                        body.push('\n');
                    }
                    body.push_str(&binding(&name, &named_json(id, None)));
                }
                _ => {
                    let name = self.validator(id, None)?;
                    let comment = jsdoc(definition.doc.as_ref());
                    if !comment.is_empty() {
                        body.push_str(&comment);
                        body.push('\n');
                    }
                    body.push_str(&binding(&name, &named_json(id, None)));
                }
            }
        }
        for (reference, name) in &self.inlines {
            body.push_str(&binding(name, reference));
        }
        let specifier = crate::runtime::specifier(self.options.runtime, "\"./runtime.js\"");
        Ok(Rendered {
            file: "validators.ts",
            content: format!(
                "import {{ validateContract, type Validator }} from {specifier};\n\
                 export type {{ ValidationIssue, Validator }} from {specifier};\n\n{body}"
            ),
        })
    }
}

fn validator_names(
    document: &Document,
    declarations: &BTreeMap<(String, Option<String>), String>,
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
        .zip(unique_names(bases, &["validateContract"]))
        .collect()
}

fn named_json(id: &str, variant: Option<&str>) -> String {
    serde_json::to_string(&TypeRef::Named(NamedRef {
        id: id.to_owned(),
        variant: variant.map(str::to_owned),
    }))
    .expect("a named reference is JSON data")
}

fn binding(name: &str, reference: &str) -> String {
    format!("export const {name}: Validator = (value) => validateContract(value, {reference});\n")
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use pons_ir::Document;
    use serde_json::{Value, json};

    use crate::{Runtime, TsOptions, Validation, emit_with_options};

    fn fixture() -> Value {
        serde_json::from_str(include_str!("../../../../fixtures/login.json")).unwrap()
    }

    fn files(document: &Document, options: TsOptions) -> BTreeMap<String, String> {
        emit_with_options(document, options)
            .unwrap()
            .files
            .into_iter()
            .map(|file| (file.path, file.content))
            .collect()
    }

    fn builtin(options: TsOptions) -> TsOptions {
        TsOptions {
            validation: Validation::Builtin,
            ..options
        }
    }

    #[test]
    fn the_builtin_backend_binds_each_named_type_and_variant_to_the_runtime() {
        let document: Document = serde_json::from_value(fixture()).unwrap();
        let files = files(&document, builtin(TsOptions::default()));
        let validators = &files["validators.ts"];
        assert!(
            validators.starts_with(
                "import { validateContract, type Validator } from \"./runtime.js\";\nexport type { ValidationIssue, Validator } from \"./runtime.js\";\n\n"
            ),
            "{validators}"
        );
        for expected in [
            "export const validateUser: Validator = (value) => validateContract(value, {\"ref\":\"User\"});",
            "export const validateLoginError: Validator = (value) => validateContract(value, {\"ref\":\"LoginError\"});",
            "export const validateLoginErrorInvalid: Validator = (value) => validateContract(value, {\"ref\":\"LoginError\",\"variant\":\"invalid\"});",
            "export const validateLoginErrorLocked: Validator = (value) => validateContract(value, {\"ref\":\"LoginError\",\"variant\":\"locked\"});",
        ] {
            assert!(
                validators.contains(expected),
                "{expected} is missing: {validators}"
            );
        }
        assert!(
            files["index.ts"].contains("export * from \"./validators.js\";"),
            "{}",
            files["index.ts"]
        );
        let auth = &files["operations/auth.ts"];
        assert!(
            auth.contains(
                "import { validateLoginErrorInvalid, validateUser } from \"../validators.js\";"
            ),
            "{auth}"
        );
        assert!(auth.contains("validate: validateUser"), "{auth}");
        assert!(
            auth.contains("validate: validateLoginErrorInvalid"),
            "{auth}"
        );
    }

    #[test]
    fn an_inline_body_gets_one_private_constant_and_the_package_names_itself() {
        let mut raw = fixture();
        raw["operations"][0]["responses"][0]["body"] = json!({"kind": "string", "format": "email"});
        let document: Document = serde_json::from_value(raw).unwrap();
        let files = files(
            &document,
            builtin(TsOptions {
                runtime: Runtime::Package,
                ..TsOptions::default()
            }),
        );
        let validators = &files["validators.ts"];
        assert!(
            validators.starts_with(
                "import { validateContract, type Validator } from \"@pons/runtime\";\nexport type { ValidationIssue, Validator } from \"@pons/runtime\";\n\n"
            ),
            "{validators}"
        );
        assert!(
            validators.contains(
                "export const validateInline1: Validator = (value) => validateContract(value, {\"format\":\"email\",\"kind\":\"string\"});"
            ),
            "{validators}"
        );
        let auth = &files["operations/auth.ts"];
        assert!(
            auth.contains(
                "import { validateInline1, validateLoginErrorInvalid } from \"../validators.js\";"
            ),
            "{auth}"
        );
        assert!(auth.contains("validate: validateInline1"), "{auth}");
        assert!(
            !auth.contains("validateContract"),
            "the operation imports the binding, not the walker: {auth}"
        );
    }
}
