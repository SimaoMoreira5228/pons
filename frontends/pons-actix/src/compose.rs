use pons_ir::{Diagnostic, PathSegment};

use crate::auth::merge_auth_requirements;
use crate::operation_cx::push_diagnostic;
use crate::router::Router;

impl<S> Router<S> {
    pub fn merge(self, other: Router<S>) -> Self {
        self.attach(&[], other)
    }

    pub fn nest(mut self, prefix: &str, other: Router<S>) -> Self {
        let Some(segments) = literal_prefix(prefix, &mut self.diagnostics) else {
            return self;
        };
        self.attach(&segments, other)
    }

    fn attach(mut self, prefix: &[String], other: Router<S>) -> Self {
        let Router {
            types,
            carriers,
            auth_schemes,
            common_errors,
            mut operations,
            diagnostics,
            mut resources,
            global_layers,
            global_auth,
            state,
            ..
        } = other;

        operations.iter_mut().for_each(|operation| {
            global_auth
                .iter()
                .for_each(|requirement| merge_auth_requirements(&mut operation.auth, requirement));
            prefix_operation(prefix, operation);
        });
        resources.iter_mut().for_each(|resource| {
            resource.path = prefixed_path(prefix, &resource.path);
            resource.layers.extend(global_layers.iter().cloned());
        });

        for (id, scheme) in auth_schemes {
            self.insert_auth_scheme(id, scheme);
        }
        self.types.extend(types);
        self.carriers.extend(carriers);
        self.common_errors.extend(common_errors);
        self.operations.extend(operations);
        self.diagnostics.extend(diagnostics);
        resources
            .into_iter()
            .for_each(|resource| self.push_resource(resource));
        self.state = self.state.take().or(state);
        self
    }
}

fn literal_prefix(prefix: &str, diagnostics: &mut Vec<Diagnostic>) -> Option<Vec<String>> {
    let prefix = prefix.trim();
    if prefix.is_empty() || prefix == "/" {
        return Some(Vec::new());
    }
    if !prefix.starts_with('/') {
        push_diagnostic(
            diagnostics,
            "PONS-E102",
            format!("the nest prefix {prefix:?} must start with a slash"),
        );
        return None;
    }
    if prefix.contains(['{', '}', '*']) {
        push_diagnostic(
            diagnostics,
            "PONS-E102",
            format!(
                "the nest prefix {prefix:?} names a parameter or a wildcard; only a literal prefix can be nested"
            ),
        );
        return None;
    }
    if prefix.contains(['?', '#']) {
        push_diagnostic(
            diagnostics,
            "PONS-E102",
            format!("the nest prefix {prefix:?} is not a path prefix"),
        );
        return None;
    }
    Some(
        prefix
            .split('/')
            .filter(|segment| !segment.is_empty())
            .map(str::to_owned)
            .collect(),
    )
}

fn prefix_operation(prefix: &[String], operation: &mut pons_ir::Operation) {
    if prefix.is_empty() {
        return;
    }
    let mut segments = prefix
        .iter()
        .map(|lit| PathSegment::Literal { lit: lit.clone() })
        .collect::<Vec<_>>();
    segments.append(&mut operation.path);
    operation.path = segments;
}

fn prefixed_path(prefix: &[String], path: &str) -> String {
    if prefix.is_empty() {
        return path.to_owned();
    }
    let prefix = format!("/{}", prefix.join("/"));
    if path == "/" || path.is_empty() {
        prefix
    } else if path.starts_with('/') {
        format!("{prefix}{path}")
    } else {
        format!("{prefix}/{path}")
    }
}
