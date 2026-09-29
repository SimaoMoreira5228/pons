use super::*;

pub(super) fn diff_operations(diff: &mut Diff, old: &Document, new: &Document) {
    let paths = operation_paths(old, new);
    let old: BTreeMap<_, _> = old
        .operations
        .iter()
        .map(|operation| (&operation.id, operation))
        .collect();
    let new: BTreeMap<_, _> = new
        .operations
        .iter()
        .map(|operation| (&operation.id, operation))
        .collect();
    for id in old.keys() {
        if !new.contains_key(id) {
            add(
                diff,
                ChangeLevel::Breaking,
                paths.get(*id).unwrap().clone(),
                "operation removed",
            );
        }
    }
    for (id, operation) in new {
        let path = paths.get(id).unwrap();
        let Some(previous) = old.get(id) else {
            add(
                diff,
                ChangeLevel::NonBreaking,
                path.clone(),
                "operation added",
            );
            continue;
        };
        diff_operation(diff, path, previous, operation);
    }
}

fn operation_paths(old: &Document, new: &Document) -> BTreeMap<Vec<Vec<String>>, String> {
    let ids = old
        .operations
        .iter()
        .chain(&new.operations)
        .map(|operation| operation.id.clone())
        .collect::<BTreeSet<_>>();
    let mut paths = BTreeMap::new();
    let mut used = BTreeSet::new();
    for id in ids {
        let readable = readable_operation_path(&id);
        let mut path = readable.clone();
        if !used.insert(path.clone()) {
            path = format!("operations.{}", crate::operation_path(&id));
            let mut suffix = 2;
            while !used.insert(path.clone()) {
                path = format!("{readable}#{suffix}");
                suffix += 1;
            }
        }
        paths.insert(id, path);
    }
    paths
}

fn readable_operation_path(id: &[Vec<String>]) -> String {
    format!(
        "operations.{}",
        id.iter()
            .map(|segment| segment.join("_"))
            .collect::<Vec<_>>()
            .join(".")
    )
}

fn diff_operation(diff: &mut Diff, path: &str, old: &Operation, new: &Operation) {
    if old.method != new.method {
        add(diff, ChangeLevel::Breaking, path, "HTTP method changed");
    }
    if old.semantics.safe != new.semantics.safe {
        add(
            diff,
            ChangeLevel::Breaking,
            format!("{path}.semantics"),
            "operation safety changed",
        );
    }
    if old.semantics.idempotent != new.semantics.idempotent {
        add(
            diff,
            ChangeLevel::Breaking,
            format!("{path}.semantics"),
            "operation idempotency changed",
        );
    }
    if route_shape(&old.path) != route_shape(&new.path) {
        add(diff, ChangeLevel::NonBreaking, path, "route URL changed");
    }
    if old.semantics.pagination != new.semantics.pagination {
        add(
            diff,
            ChangeLevel::Warning,
            format!("{path}.semantics.pagination"),
            "operation pagination changed",
        );
    }
    if old.path != new.path {
        let old_params: Vec<_> = old
            .path
            .iter()
            .filter_map(|segment| match segment {
                crate::PathSegment::Param { param, ty } => Some((param, ty)),
                crate::PathSegment::Literal { .. } => None,
            })
            .collect();
        let new_params: Vec<_> = new
            .path
            .iter()
            .filter_map(|segment| match segment {
                crate::PathSegment::Param { param, ty } => Some((param, ty)),
                crate::PathSegment::Literal { .. } => None,
            })
            .collect();
        if old_params.len() != new_params.len() {
            add(
                diff,
                ChangeLevel::Breaking,
                format!("{path}.path"),
                "path parameter count changed",
            );
        }
        for (index, ((old_name, before), (new_name, after))) in
            old_params.iter().zip(&new_params).enumerate()
        {
            if before != after {
                add(
                    diff,
                    ChangeLevel::Breaking,
                    format!("{path}.path[{index}]"),
                    "path parameter type changed",
                );
            }
            if old_name != new_name {
                add(
                    diff,
                    ChangeLevel::Breaking,
                    format!("{path}.path[{index}]"),
                    "path parameter name changed",
                );
            }
        }
    }
    if old.query != new.query {
        add(
            diff,
            ChangeLevel::Breaking,
            format!("{path}.query"),
            "query type changed",
        );
    }
    diff_headers(diff, path, &old.headers, &new.headers);
    diff_body(diff, path, old.body.as_ref(), new.body.as_ref());
    if old.responses != new.responses {
        diff_responses(diff, path, old, new);
    }
    diff_errors(diff, path, &old.errors, &new.errors);
    if old.errors_inherit != new.errors_inherit {
        add(
            diff,
            ChangeLevel::Breaking,
            format!("{path}.errors"),
            "inherited errors changed",
        );
    }
    if old.auth != new.auth {
        add(
            diff,
            ChangeLevel::Breaking,
            format!("{path}.auth"),
            "authentication requirement changed",
        );
    }
    if old.provides != new.provides {
        add(
            diff,
            ChangeLevel::Breaking,
            format!("{path}.provides"),
            "credential provision changed",
        );
    }
    if old.stream != new.stream {
        add(
            diff,
            ChangeLevel::Breaking,
            format!("{path}.stream"),
            "stream definition changed",
        );
    }
    if old.ext != new.ext {
        add(
            diff,
            ChangeLevel::Warning,
            format!("{path}.ext"),
            "operation metadata changed",
        );
    }
    add_doc_change(
        diff,
        &format!("{path}.doc"),
        old.doc.as_ref(),
        new.doc.as_ref(),
        "operation",
    );
}

fn route_shape(path: &[crate::PathSegment]) -> Vec<String> {
    path.iter()
        .map(|segment| match segment {
            crate::PathSegment::Literal { lit } => lit.clone(),
            crate::PathSegment::Param { param, .. } => format!("{{{param}}}"),
        })
        .collect()
}

fn diff_body(diff: &mut Diff, path: &str, old: Option<&Body>, new: Option<&Body>) {
    let (Some(old), Some(new)) = (old, new) else {
        if old.is_some() || new.is_some() {
            add(
                diff,
                ChangeLevel::Breaking,
                format!("{path}.body"),
                "request body changed",
            );
        }
        return;
    };
    if old.ty != new.ty
        || old.content != new.content
        || old.media_type != new.media_type
        || old.ext != new.ext
    {
        add(
            diff,
            ChangeLevel::Breaking,
            format!("{path}.body"),
            "request body changed",
        );
    } else if old.required != new.required {
        add(
            diff,
            if old.required {
                ChangeLevel::NonBreaking
            } else {
                ChangeLevel::Breaking
            },
            format!("{path}.body"),
            if old.required {
                "request body became optional"
            } else {
                "request body became required"
            },
        );
    }
}

fn diff_headers(
    diff: &mut Diff,
    path: &str,
    old: &[crate::HeaderParameter],
    new: &[crate::HeaderParameter],
) {
    let old: BTreeMap<String, _> = old
        .iter()
        .map(|header| (header.wire.to_ascii_lowercase(), header))
        .collect();
    let new: BTreeMap<String, _> = new
        .iter()
        .map(|header| (header.wire.to_ascii_lowercase(), header))
        .collect();
    for (key, header) in &old {
        if !new.contains_key(key) {
            add(
                diff,
                ChangeLevel::Breaking,
                format!("{path}.headers.{}", header.wire),
                "request header removed",
            );
        }
    }
    for (key, header) in &new {
        let header_path = format!("{path}.headers.{}", header.wire);
        let Some(previous) = old.get(key) else {
            let (level, detail) = if header.presence == Presence::Required {
                (ChangeLevel::Breaking, "required request header added")
            } else {
                (ChangeLevel::NonBreaking, "optional request header added")
            };
            add(diff, level, header_path, detail);
            continue;
        };
        if previous.ty != header.ty {
            add(
                diff,
                ChangeLevel::Breaking,
                &header_path,
                "request header type changed",
            );
        }
        if previous.presence != header.presence {
            let (level, detail) = if previous.presence == Presence::Optional {
                (
                    ChangeLevel::Breaking,
                    "optional request header became required",
                )
            } else {
                (ChangeLevel::NonBreaking, "request header presence changed")
            };
            add(diff, level, &header_path, detail);
        }
        if previous.ident != header.ident {
            add(
                diff,
                ChangeLevel::Breaking,
                &header_path,
                "request header identifier changed",
            );
        }
        if previous.ext != header.ext {
            add(
                diff,
                ChangeLevel::Warning,
                &header_path,
                "request header metadata changed",
            );
        }
    }
}

fn diff_responses(diff: &mut Diff, path: &str, old: &Operation, new: &Operation) {
    let old: BTreeMap<_, _> = old
        .responses
        .iter()
        .map(|response| (response.status, response))
        .collect();
    let new: BTreeMap<_, _> = new
        .responses
        .iter()
        .map(|response| (response.status, response))
        .collect();
    for status in old.keys() {
        if !new.contains_key(status) {
            add(
                diff,
                ChangeLevel::Breaking,
                format!("{path}.responses.{status}"),
                "response removed",
            );
        }
    }
    for (status, response) in new {
        let response_path = format!("{path}.responses.{status}");
        let Some(previous) = old.get(&status) else {
            add(
                diff,
                ChangeLevel::Breaking,
                response_path,
                "response status added",
            );
            continue;
        };
        if !response_wire_equivalent(previous, response) {
            add(
                diff,
                ChangeLevel::Breaking,
                &response_path,
                "response changed",
            );
        }
        if previous.ext != response.ext {
            add(
                diff,
                ChangeLevel::Warning,
                &response_path,
                "response metadata changed",
            );
        }
    }
}

fn response_wire_equivalent(old: &Response, new: &Response) -> bool {
    let wire = |response: &Response| {
        let mut response = response.clone();
        response.ext.clear();
        response
    };
    wire(old).equivalent_ignoring_header_case(&wire(new))
}

fn error_wire_equivalent(old: &ErrorResponse, new: &ErrorResponse) -> bool {
    let wire = |error: &ErrorResponse| {
        let mut error = error.clone();
        error.doc = None;
        error.ext.clear();
        error
    };
    wire(old).equivalent_ignoring_header_case(&wire(new))
}

pub(super) fn diff_errors(
    diff: &mut Diff,
    path: &str,
    old: &[ErrorResponse],
    new: &[ErrorResponse],
) {
    let old: BTreeMap<_, _> = old
        .iter()
        .map(|error| ((&error.status, &error.name), error))
        .collect();
    let new: BTreeMap<_, _> = new
        .iter()
        .map(|error| ((&error.status, &error.name), error))
        .collect();
    for key in old.keys() {
        if !new.contains_key(key) {
            add(
                diff,
                ChangeLevel::Breaking,
                format!("{path}.errors.{}", key.1.join("_")),
                "error removed",
            );
        }
    }
    for (key, error) in new {
        let error_path = format!("{path}.errors.{}", key.1.join("_"));
        let Some(previous) = old.get(&key) else {
            add(
                diff,
                ChangeLevel::Breaking,
                &error_path,
                "error status added",
            );
            continue;
        };
        if !error_wire_equivalent(previous, error) {
            add(
                diff,
                ChangeLevel::Breaking,
                &error_path,
                "error definition changed",
            );
        }
        add_doc_change(
            diff,
            &error_path,
            previous.doc.as_ref(),
            error.doc.as_ref(),
            "error",
        );
        if previous.ext != error.ext {
            add(
                diff,
                ChangeLevel::Warning,
                &error_path,
                "error metadata changed",
            );
        }
    }
}
