use std::collections::BTreeSet;

use pons_ir::Diagnostic;
use serde_json::{Map, Value};

use crate::document::push_warning;

#[derive(Clone, Copy)]
pub(crate) enum Site<'a> {
    Operation(&'a str),
    Path(&'a str),
    Document,
}

impl std::fmt::Display for Site<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Operation(operation) => write!(formatter, "operation {operation}"),
            Self::Path(path) => write!(formatter, "path {path:?}"),
            Self::Document => formatter.write_str("document"),
        }
    }
}

pub(crate) struct Resolver<'a> {
    root: &'a Map<String, Value>,
}

impl<'a> Resolver<'a> {
    pub(crate) fn new(root: &'a Map<String, Value>) -> Self {
        Self { root }
    }

    pub(crate) fn resolve<'v>(
        &'v self,
        value: &'v Value,
        site: Site<'_>,
        diagnostics: &mut Vec<Diagnostic>,
    ) -> Option<&'v Value> {
        let mut current = value;
        let mut in_flight = BTreeSet::new();
        loop {
            let Some(reference) = current.get("$ref").and_then(Value::as_str) else {
                return Some(current);
            };
            if !in_flight.insert(reference) {
                report(diagnostics, site, reference, "forms a reference cycle");
                return None;
            }
            if !reference.starts_with("#/") {
                report(
                    diagnostics,
                    site,
                    reference,
                    "is not a local #/ reference, and only local references are resolved",
                );
                return None;
            }
            let Some(target) = self.pointer(reference) else {
                report(
                    diagnostics,
                    site,
                    reference,
                    "does not resolve against the document",
                );
                return None;
            };
            current = target;
        }
    }

    fn pointer<'v>(&'v self, reference: &str) -> Option<&'v Value> {
        let mut tokens = reference.strip_prefix("#/")?.split('/');
        let mut current = self.root.get(&unescape(tokens.next()?))?;
        for token in tokens {
            current = step(current, &unescape(token))?;
        }
        Some(current)
    }
}

fn step<'v>(value: &'v Value, token: &str) -> Option<&'v Value> {
    match value {
        Value::Object(object) => object.get(token),
        Value::Array(items) => items.get(token.parse::<usize>().ok()?),
        _ => None,
    }
}

fn unescape(token: &str) -> String {
    token.replace("~1", "/").replace("~0", "~")
}

fn report(diagnostics: &mut Vec<Diagnostic>, site: Site<'_>, reference: &str, reason: &str) {
    let message = format!("{site}: the reference {reference:?} {reason}, so it is dropped");
    push_warning(diagnostics, message);
}
