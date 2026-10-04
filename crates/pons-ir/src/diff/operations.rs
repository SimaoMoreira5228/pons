use super::*;

pub(super) fn diff_operations(
    diff: &mut Diff,
    used: &mut BTreeSet<String>,
    old: &Document,
    new: &Document,
) {
    let paths = operation_paths(used, old, new);
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
        diff_operation(diff, used, path, previous, operation);
    }
}

fn operation_paths(
    used: &mut BTreeSet<String>,
    old: &Document,
    new: &Document,
) -> BTreeMap<Vec<Vec<String>>, String> {
    let ids = old
        .operations
        .iter()
        .chain(&new.operations)
        .map(|operation| operation.id.clone())
        .collect::<BTreeSet<_>>();
    injective_paths(
        used,
        ids,
        |id| readable_operation_path(id),
        |id| format!("operations.{}", crate::operation_path(id)),
    )
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

fn injective_paths<K: Ord>(
    used: &mut BTreeSet<String>,
    keys: impl IntoIterator<Item = K>,
    readable: impl Fn(&K) -> String,
    escaped: impl Fn(&K) -> String,
) -> BTreeMap<K, String> {
    let mut paths = BTreeMap::new();
    for key in keys {
        let read = readable(&key);
        let mut path = read.clone();
        if !used.insert(path.clone()) {
            path = escaped(&key);
            let mut suffix = 2;
            while !used.insert(path.clone()) {
                path = format!("{read}#{suffix}");
                suffix += 1;
            }
        }
        paths.insert(key, path);
    }
    paths
}

fn diff_operation(
    diff: &mut Diff,
    paths: &mut BTreeSet<String>,
    path: &str,
    old: &Operation,
    new: &Operation,
) {
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
    diff_errors(diff, paths, path, &old.errors, &new.errors);
    if old.errors_inherit != new.errors_inherit {
        add(
            diff,
            ChangeLevel::Breaking,
            format!("{path}.errors"),
            "inherited errors changed",
        );
    }
    if old.auth.alternatives != new.auth.alternatives
        || old.auth.scopes != new.auth.scopes
        || old.auth.optional != new.auth.optional
    {
        add(
            diff,
            ChangeLevel::Breaking,
            format!("{path}.auth"),
            "authentication requirement changed",
        );
    }
    if old.auth.ext != new.auth.ext {
        add(
            diff,
            ChangeLevel::Warning,
            format!("{path}.auth"),
            "authentication requirement metadata changed",
        );
    }
    match (&old.provides, &new.provides) {
        (Some(old), Some(new)) => {
            if old.scheme != new.scheme
                || old.value != new.value
                || old.expires_at != new.expires_at
            {
                add(
                    diff,
                    ChangeLevel::Breaking,
                    format!("{path}.provides"),
                    "credential provision changed",
                );
            }
            if old.ext != new.ext {
                add(
                    diff,
                    ChangeLevel::Warning,
                    format!("{path}.provides"),
                    "credential provision metadata changed",
                );
            }
        }
        (Some(_), None) | (None, Some(_)) => add(
            diff,
            ChangeLevel::Breaking,
            format!("{path}.provides"),
            "credential provision changed",
        ),
        (None, None) => {}
    }
    diff_stream(diff, path, old.stream.as_ref(), new.stream.as_ref());
    diff_diagnostics(diff, path, &old.diagnostics, &new.diagnostics);
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
    if old.ty != new.ty || old.content != new.content || old.media_type != new.media_type {
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
    if old.ext != new.ext {
        add(
            diff,
            ChangeLevel::Warning,
            format!("{path}.body"),
            "request body metadata changed",
        );
    }
}

fn diff_stream(
    diff: &mut Diff,
    path: &str,
    old: Option<&crate::StreamDescription>,
    new: Option<&crate::StreamDescription>,
) {
    match (old, new) {
        (Some(old), Some(new)) => match (&old.kind, &new.kind) {
            (crate::StreamKind::Sse { events: old }, crate::StreamKind::Sse { events: new }) => {
                let old: BTreeMap<_, _> = old.iter().map(|event| (&event.event, event)).collect();
                let new: BTreeMap<_, _> = new.iter().map(|event| (&event.event, event)).collect();
                for event in old.keys() {
                    if !new.contains_key(event) {
                        add(
                            diff,
                            ChangeLevel::Breaking,
                            format!("{path}.stream.events.{event}"),
                            "stream event removed",
                        );
                    }
                }
                for (event, message) in new {
                    let event_path = format!("{path}.stream.events.{event}");
                    match old.get(event) {
                        Some(previous) if previous.data != message.data => add(
                            diff,
                            ChangeLevel::Breaking,
                            event_path,
                            "stream event data type changed",
                        ),
                        Some(_) => {}
                        None => add(
                            diff,
                            ChangeLevel::NonBreaking,
                            event_path,
                            "stream event added",
                        ),
                    }
                }
            }
            (crate::StreamKind::Ndjson { item: old }, crate::StreamKind::Ndjson { item: new }) => {
                if old != new {
                    add(
                        diff,
                        ChangeLevel::Breaking,
                        format!("{path}.stream.item"),
                        "stream item type changed",
                    )
                }
            }
            (
                crate::StreamKind::Bytes { content_type: old },
                crate::StreamKind::Bytes { content_type: new },
            ) => {
                if old != new {
                    add(
                        diff,
                        ChangeLevel::Breaking,
                        format!("{path}.stream.content_type"),
                        "stream content type changed",
                    )
                }
            }
            (
                crate::StreamKind::Websocket {
                    client: old_client,
                    server: old_server,
                },
                crate::StreamKind::Websocket {
                    client: new_client,
                    server: new_server,
                },
            ) => {
                if old_client != new_client {
                    add(
                        diff,
                        ChangeLevel::Breaking,
                        format!("{path}.stream.client"),
                        "stream client message type changed",
                    );
                }
                if old_server != new_server {
                    add(
                        diff,
                        ChangeLevel::Breaking,
                        format!("{path}.stream.server"),
                        "stream server message type changed",
                    );
                }
            }
            _ => add(
                diff,
                ChangeLevel::Breaking,
                format!("{path}.stream"),
                "stream kind changed",
            ),
        },
        (Some(_), None) => add(
            diff,
            ChangeLevel::Breaking,
            format!("{path}.stream"),
            "stream removed",
        ),
        (None, Some(_)) => add(
            diff,
            ChangeLevel::Breaking,
            format!("{path}.stream"),
            "stream added",
        ),
        (None, None) => {}
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
    used: &mut BTreeSet<String>,
    path: &str,
    old: &[ErrorResponse],
    new: &[ErrorResponse],
) {
    let paths = error_paths(used, path, old, new);
    let old: BTreeMap<_, _> = old
        .iter()
        .map(|error| ((error.status.clone(), error.name.clone()), error))
        .collect();
    let new: BTreeMap<_, _> = new
        .iter()
        .map(|error| ((error.status.clone(), error.name.clone()), error))
        .collect();
    for key in old.keys() {
        if !new.contains_key(key) {
            add(
                diff,
                ChangeLevel::Breaking,
                paths.get(key).unwrap().clone(),
                "error removed",
            );
        }
    }
    for (key, error) in new {
        let error_path = paths.get(&key).unwrap().clone();
        let Some(previous) = old.get(&key) else {
            add(
                diff,
                ChangeLevel::Breaking,
                error_path,
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

fn error_paths(
    used: &mut BTreeSet<String>,
    path: &str,
    old: &[ErrorResponse],
    new: &[ErrorResponse],
) -> BTreeMap<(crate::ErrorStatus, crate::Name), String> {
    let keys = old
        .iter()
        .chain(new)
        .map(|error| (error.status.clone(), error.name.clone()))
        .collect::<BTreeSet<_>>();
    injective_paths(
        used,
        keys,
        |(_, name)| format!("{path}.errors.{}", name.join("_")),
        |(status, name)| {
            let status = match status {
                crate::ErrorStatus::Exact(status) => status.to_string(),
                crate::ErrorStatus::Class(class) => format!("{class}xx"),
                crate::ErrorStatus::Default => "default".into(),
            };
            format!(
                "{path}.errors.{status}.{}",
                crate::operation_path(std::slice::from_ref(name))
            )
        },
    )
}
