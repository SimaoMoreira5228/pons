use std::collections::{BTreeMap, HashMap};

use pons_ir::{
    Diagnostic, Method, Operation, OperationSemantics, PathSegment, TypeDef, TypeKind, TypeRef,
};

use crate::describe::OperationCx;

pub(crate) struct RouteDescription {
    pub method: Method,
    pub id: Vec<Vec<String>>,
    pub cx: OperationCx,
}

pub(crate) struct BuiltOperation {
    pub operation: Operation,
    pub types: BTreeMap<String, TypeDef>,
}

pub(crate) fn build(path: &str, description: RouteDescription) -> BuiltOperation {
    let mut diagnostics = description.cx.diagnostics.clone();
    if description.cx.path_types.len() > 1 {
        diagnostics.push(diagnostic(
            "PONS-W101",
            "multiple Path extractors are not represented separately",
        ));
    }
    let path_types = path_types(&description.cx);
    let segments = parse_path(path, &path_types, &mut diagnostics);
    let (types, type_diagnostics) = description.cx.types.into_parts();
    diagnostics.extend(type_diagnostics);
    let id = description.id;
    let method = description.method;
    let operation = Operation {
        id,
        method,
        path: segments,
        query: description.cx.query,
        headers: Vec::new(),
        body: description.cx.body,
        responses: description.cx.responses,
        errors: description.cx.errors,
        errors_inherit: true,
        stream: description.cx.stream,
        auth: description.cx.auth,
        provides: None,
        semantics: OperationSemantics {
            safe: matches!(method, Method::Get | Method::Head | Method::Options),
            idempotent: matches!(
                method,
                Method::Get | Method::Put | Method::Delete | Method::Head | Method::Options
            ),
            pagination: None,
        },
        doc: None,
        origin: None,
        ext: BTreeMap::new(),
        diagnostics,
    };
    BuiltOperation { operation, types }
}

fn path_types(cx: &OperationCx) -> HashMap<String, TypeRef> {
    let mut result = HashMap::new();
    for ty in &cx.path_types {
        add_path_types(ty, cx.type_definitions(), &mut result);
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
                let param = segment[1..segment.len() - 1].trim_start_matches('*');
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
        diagnostics.push(diagnostic(
            "PONS-W102",
            "path parameters without a matching Path extractor are typed as strings",
        ));
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

fn diagnostic(code: &str, message: &str) -> Diagnostic {
    Diagnostic {
        severity: pons_ir::DiagnosticSeverity::Warning,
        code: code.to_owned(),
        message: message.to_owned(),
        origin: None,
        ext: BTreeMap::new(),
    }
}
