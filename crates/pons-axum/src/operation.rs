use std::collections::{BTreeMap, HashMap};

use pons_ir::{
    Diagnostic, Direction, ErrorStatus, Method, Operation, OperationSemantics, PathSegment,
    TypeDef, TypeKind, TypeRef,
};

use crate::describe::OperationCx;
use crate::metadata::OperationMeta;
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
    let (id, doc, provides) = match meta {
        Some(meta) => (meta.id, Some(meta.doc), meta.provides.or(cx.provides)),
        None => (inferred_id, None, cx.provides),
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
    let parameter_count = path
        .split('/')
        .filter(|segment| segment.starts_with('{') && segment.ends_with('}'))
        .count();
    let mut used_fallback = false;
    let mut parameter_index = 0;
    let segments = path
        .split('/')
        .skip(1)
        .filter(|segment| !segment.is_empty() || path.ends_with('/'))
        .map(|segment| {
            if segment.starts_with('{') && segment.ends_with('}') {
                let inner = &segment[1..segment.len() - 1];
                if inner.starts_with('*') {
                    push_diagnostic(
                        diagnostics,
                        "PONS-E102",
                        "a wildcard path parameter matches the remaining segments, which a path parameter of one segment cannot describe; the route cannot be published",
                    );
                }
                let param = inner.trim_start_matches('*');
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
            } else {
                PathSegment::Literal {
                    lit: segment.to_owned(),
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

fn string_type() -> TypeRef {
    TypeRef::Inline(pons_ir::InlineType::String {
        format: None,
        min_length: None,
        max_length: None,
        pattern: None,
    })
}
