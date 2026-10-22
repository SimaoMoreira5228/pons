use std::collections::{BTreeMap, HashMap};

use pons_ir::{
    Diagnostic, Direction, ErrorStatus, Method, Operation, OperationSemantics, PathSegment,
    TypeDef, TypeKind, TypeRef,
};

use crate::describe::OperationCx;
use crate::metadata::{OperationMeta, Semantics};
use crate::operation_cx::push_diagnostic;

pub(crate) struct RouteDescription {
    pub method: Method,
    pub id: Vec<Vec<String>>,
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
    crate::operation_cx::lower_flat_carriers(&mut types, &cx.flat_carriers);
    let path_types = path_types(&types, &cx.path_types);
    let segments = parse_path(path, &path_types, &mut diagnostics);
    diagnostics.extend(type_diagnostics);
    let (id, doc, provides, semantics) = match meta {
        Some(meta) => (
            meta.id,
            Some(meta.doc),
            meta.provides.or(cx.provides),
            meta.semantics,
        ),
        None => (inferred_id, None, cx.provides, Semantics::default()),
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
            safe: semantics.safe.unwrap_or(matches!(
                method,
                Method::Get | Method::Head | Method::Options
            )),
            idempotent: semantics.idempotent.unwrap_or(matches!(
                method,
                Method::Get | Method::Put | Method::Delete | Method::Head | Method::Options
            )),
            pagination: None,
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
                            "the route {path} has an empty path segment at position {index}, which axum serves as a segment of its own; a path with no segment there is a different route, so the route cannot be published"
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
    if !path.starts_with('/') {
        return Err(format!("the nest prefix {path:?} must start with '/'"));
    }
    if path == "/" {
        return Err("nesting at the root is a merge, so use Router::merge instead".to_owned());
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
    let trailing_slash =
        matches!(prefix.last(), Some(PathSegment::Literal { lit }) if lit.is_empty());
    let base = if trailing_slash {
        &prefix[..prefix.len() - 1]
    } else {
        prefix
    };
    let mut path = base.to_vec();
    if inner.is_empty() {
        if trailing_slash {
            path.push(PathSegment::Literal { lit: String::new() });
        }
    } else {
        path.extend_from_slice(inner);
    }
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
