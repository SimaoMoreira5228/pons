use std::collections::{BTreeMap, HashMap};

use pons_ir::{
    Diagnostic, Direction, ErrorStatus, Method, Name, Operation, OperationSemantics, PathSegment,
    TypeDef, TypeKind, TypeRef,
};

use crate::describe::OperationCx;
use crate::metadata::OperationMeta;
use crate::operation_cx::{lower_flat_carriers, push_diagnostic};

pub(crate) struct RouteDescription {
    pub method: Method,
    pub id: Vec<Name>,
    pub cx: OperationCx,
    pub meta: Option<OperationMeta>,
}

pub(crate) struct BuiltOperation {
    pub operation: Operation,
    pub types: BTreeMap<String, TypeDef>,
    pub carriers: Vec<TypeRef>,
}

pub(crate) fn build(path: &str, description: RouteDescription) -> BuiltOperation {
    let RouteDescription {
        method,
        id: inferred_id,
        mut cx,
        meta,
    } = description;
    let mut diagnostics = cx.diagnostics.clone();
    if cx.validated {
        declare_rejection(&mut cx, method, &mut diagnostics);
    }
    if cx.path_types.len() > 1 {
        push_diagnostic(
            &mut diagnostics,
            "PONS-W101",
            "multiple Path extractors are not represented separately",
        );
    }
    let (mut types, type_diagnostics) = cx.types.into_parts();
    lower_flat_carriers(&mut types, &cx.flat_carriers);
    let path_types = path_types(&types, &cx.path_types);
    let segments = parse_path(path, &path_types, &mut diagnostics);
    diagnostics.extend(type_diagnostics);
    let (id, doc, provides, pagination) = match meta {
        Some(meta) => (meta.id, Some(meta.doc), meta.provides, meta.pagination),
        None => (inferred_id, None, None, None),
    };
    let operation = Operation {
        id,
        method,
        path: segments,
        query: cx.query,
        headers: cx.headers,
        body: cx.body,
        responses: cx.responses,
        errors: cx.errors,
        errors_inherit: true,
        stream: cx.stream,
        auth: cx.auth,
        provides,
        semantics: OperationSemantics {
            safe: matches!(method, Method::Get | Method::Head | Method::Options),
            idempotent: matches!(
                method,
                Method::Get | Method::Put | Method::Delete | Method::Head | Method::Options
            ),
            pagination,
        },
        doc,
        origin: None,
        ext: BTreeMap::new(),
        diagnostics,
    };
    BuiltOperation {
        carriers: cx.carriers,
        operation,
        types,
    }
}

fn declare_rejection(cx: &mut OperationCx, method: Method, diagnostics: &mut Vec<Diagnostic>) {
    let status = ErrorStatus::Exact(crate::error::REJECTED);
    if let Some(declared) = cx.errors.iter().find(|error| error.status == status) {
        let name = format!("{:?}", declared.name);
        if method == Method::Head && declared.body.is_some() {
            return push_diagnostic(
                diagnostics,
                "PONS-E102",
                format!(
                    "a HEAD operation cannot serve the request rejection as {name}, which carries a body"
                ),
            );
        }
        return push_diagnostic(
            diagnostics,
            "PONS-W101",
            format!(
                "a request is rejected as {name}, the {status:?} the operation already declares"
            ),
        );
    }
    let body = match method {
        Method::Head => None,
        _ => Some(<crate::ViolationReport as pons_core::Schema>::schema(
            &mut cx.types,
            Direction::Output,
        )),
    };
    cx.errors.push(crate::ViolationReport::error(body));
}

pub(crate) fn ir_method(method: &http::Method) -> Method {
    match method.as_str() {
        "GET" => Method::Get,
        "POST" => Method::Post,
        "PUT" => Method::Put,
        "PATCH" => Method::Patch,
        "DELETE" => Method::Delete,
        "HEAD" => Method::Head,
        "OPTIONS" => Method::Options,
        other => panic!("Pons IR cannot describe the {other} method"),
    }
}

pub(crate) fn inferred_id(method: &http::Method, path: &str) -> Vec<Name> {
    let mut id = vec![name_words(method.as_str())];
    for segment in path.split('/').filter(|segment| !segment.is_empty()) {
        let name = match Segment::of(segment) {
            Segment::Param { name, .. } => name.trim_start_matches('*'),
            _ => segment,
        };
        let words = name_words(name);
        if !words.is_empty() {
            id.push(words);
        }
    }
    id
}

fn name_words(source: &str) -> Name {
    let mut words = Vec::new();
    let mut current = String::new();
    let chars: Vec<char> = source.trim_start_matches("r#").chars().collect();
    for (index, character) in chars.iter().copied().enumerate() {
        if character == '_' {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
        } else {
            if character.is_uppercase() && !current.is_empty() {
                let previous = chars
                    .get(index.wrapping_sub(1))
                    .copied()
                    .unwrap_or_default();
                let next = chars.get(index + 1).copied().unwrap_or_default();
                if previous.is_lowercase() || next.is_lowercase() && previous.is_uppercase() {
                    words.push(std::mem::take(&mut current));
                }
            }
            current.extend(character.to_lowercase());
        }
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

fn path_types(types: &BTreeMap<String, TypeDef>, carriers: &[TypeRef]) -> HashMap<String, TypeRef> {
    let mut result = HashMap::new();
    for ty in carriers {
        add_path_types(ty, types, &mut result);
    }
    result
}

fn add_path_types(
    ty: &TypeRef,
    definitions: &BTreeMap<String, TypeDef>,
    result: &mut HashMap<String, TypeRef>,
) {
    match ty {
        TypeRef::Named(reference) => {
            match definitions.get(&reference.id).map(|def| &def.definition) {
                Some(TypeKind::Struct { fields, .. }) => {
                    for field in fields {
                        result.insert(field.wire.clone(), field.ty.clone());
                    }
                }
                Some(TypeKind::Newtype { inner }) => add_path_types(inner, definitions, result),
                _ => {
                    result.insert(String::new(), ty.clone());
                }
            }
        }
        TypeRef::Inline(pons_ir::InlineType::Tuple { items }) => {
            for (index, item) in items.iter().enumerate() {
                result.insert(index.to_string(), item.clone());
            }
        }
        _ => {
            result.insert(String::new(), ty.clone());
        }
    }
}

fn parse_path(
    path: &str,
    path_types: &HashMap<String, TypeRef>,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<PathSegment> {
    if path == "/" {
        return Vec::new();
    }
    let raw: Vec<Segment<'_>> = path.split('/').skip(1).map(Segment::of).collect();
    let parameter_count = raw
        .iter()
        .filter(|segment| matches!(segment, Segment::Param { .. }))
        .count();
    let mut used_fallback = false;
    let mut parameter_index = 0;
    let segments = raw
        .iter()
        .enumerate()
        .map(|(index, segment)| match segment {
            Segment::Param { name, wildcard } => {
                if *wildcard {
                    push_diagnostic(
                        diagnostics,
                        "PONS-E102",
                        "a wildcard path parameter matches the remaining segments, which a path parameter of one segment cannot describe; the route cannot be published",
                    );
                }
                let param = name.trim_start_matches('*');
                let tuple_key = parameter_index.to_string();
                parameter_index += 1;
                let ty = path_types.get(param).or_else(|| {
                    (parameter_count == 1)
                        .then(|| path_types.get(""))
                        .flatten()
                        .or_else(|| path_types.get(&tuple_key))
                });
                let ty = ty.cloned().unwrap_or_else(|| {
                    used_fallback = true;
                    string_type()
                });
                PathSegment::Param {
                    param: param.to_owned(),
                    ty,
                }
            }
            Segment::Literal(lit) => PathSegment::Literal {
                lit: unescape(lit),
            },
            Segment::Empty => {
                if index + 1 < raw.len() {
                    push_diagnostic(
                        diagnostics,
                        "PONS-E102",
                        format!(
                            "the route {path} has an empty path segment at position {index}, which Tako serves as a segment of its own; a path with no segment there is a different route, so the route cannot be published"
                        ),
                    );
                }
                PathSegment::Literal {
                    lit: String::new(),
                }
            }
        })
        .collect::<Vec<_>>();
    if used_fallback {
        push_diagnostic(
            diagnostics,
            "PONS-W102",
            "path parameters without a matching Path extractor are typed as strings",
        );
    }
    segments
}

enum Segment<'a> {
    Literal(&'a str),
    Param { name: &'a str, wildcard: bool },
    Empty,
}

impl<'a> Segment<'a> {
    fn of(segment: &'a str) -> Self {
        if segment.is_empty() {
            return Segment::Empty;
        }
        let braced = segment.starts_with('{') && segment.ends_with('}');
        if braced && !segment.starts_with("{{") && !segment.ends_with("}}") {
            let name = &segment[1..segment.len() - 1];
            return Segment::Param {
                wildcard: name.starts_with('*'),
                name,
            };
        }
        Segment::Literal(segment)
    }
}

fn unescape(segment: &str) -> String {
    if !segment.contains("{{") && !segment.contains("}}") {
        return segment.to_owned();
    }
    let mut unescaped = String::with_capacity(segment.len());
    let mut characters = segment.chars();
    while let Some(character) = characters.next() {
        unescaped.push(character);
        if matches!(character, '{' | '}') && characters.as_str().starts_with(character) {
            characters.next();
        }
    }
    unescaped
}

pub(crate) fn literal_prefix(path: &str) -> Result<Vec<PathSegment>, String> {
    if path.is_empty() || path == "/" {
        return Ok(Vec::new());
    }
    if !path.starts_with('/') {
        return Err(format!("the nest prefix {path:?} must start with '/'"));
    }
    let raw: Vec<&str> = path.split('/').skip(1).collect();
    let mut segments = Vec::with_capacity(raw.len());
    for (index, segment) in raw.iter().enumerate() {
        match Segment::of(segment) {
            Segment::Param { wildcard: true, .. } => {
                return Err(format!(
                    "the nest prefix {path:?} contains a wildcard; a group is described only under a literal prefix, so match the remaining segments on the routes inside the group"
                ));
            }
            Segment::Param { .. } => {
                return Err(format!(
                    "the nest prefix {path:?} contains a parameter; a group is described only under a literal prefix, so put the parameter on the routes inside the group"
                ));
            }
            Segment::Empty if index + 1 < raw.len() => {
                return Err(format!(
                    "the nest prefix {path:?} has an empty segment at position {index}, so it cannot be described as a path"
                ));
            }
            Segment::Empty => segments.push(PathSegment::Literal { lit: String::new() }),
            Segment::Literal(lit) => segments.push(PathSegment::Literal { lit: unescape(lit) }),
        }
    }
    Ok(segments)
}

pub(crate) fn nested_path(prefix: &[PathSegment], inner: &[PathSegment]) -> Vec<PathSegment> {
    let mut prefix = prefix;
    while matches!(prefix.last(), Some(PathSegment::Literal { lit }) if lit.is_empty()) {
        prefix = &prefix[..prefix.len() - 1];
    }
    if prefix.is_empty() {
        return inner.to_vec();
    }
    let mut path = prefix.to_vec();
    path.extend_from_slice(inner);
    path
}

fn string_type() -> TypeRef {
    TypeRef::Inline(pons_ir::InlineType::String {
        format: None,
        min_length: None,
        max_length: None,
        pattern: None,
    })
}
