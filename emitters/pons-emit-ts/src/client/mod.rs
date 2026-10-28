use std::collections::BTreeMap;

mod operations;
mod signatures;

use pons_ir::{Document, PathSegment, ResponseStatus, StreamKind, operation_subject};

use self::operations::operation_function;
use self::signatures::{Node, auth_types, operation_type_names, option_types};
use crate::naming::{camel, identifier, is_plain_identifier};
use crate::render::{jsdoc, string};
use crate::{
    EmitError, EmitMode, ErrorStrategy, GeneratedFile, Namespaces, Runtime, TsOptions, pagination,
    validation,
};

pub(crate) use self::operations::{normalize_integer_constraints, tag_carriers, type_ref_json};

pub(super) fn index(validation: Option<&str>) -> GeneratedFile {
    GeneratedFile {
        path: "index.ts".into(),
        content: format!(
            "export * from \"./errors.js\";\nexport * from \"./client.js\";\n{}",
            validation.map_or_else(String::new, |module| format!(
                "export * from \"./{module}.js\";\n"
            ))
        ),
    }
}

pub(super) fn emit(
    document: &Document,
    names: &BTreeMap<String, String>,
    options: &TsOptions,
    preserve_wide_integers: bool,
    validation: &mut validation::Backend,
    pagination_plans: &BTreeMap<usize, pagination::Plan>,
) -> Result<Vec<GeneratedFile>, EmitError> {
    let opaque_required = options.mode == EmitMode::Strict && document.has_reachable_opaque();
    let mut namespaces: BTreeMap<String, Namespace> = BTreeMap::new();
    let operation_names = operation_type_names(document);
    let mut pending_iterators = Vec::new();
    for (index, (operation, declaration_names)) in
        document.operations.iter().zip(operation_names).enumerate()
    {
        let plan = pagination_plans.get(&index);
        let (declarations, function, iterator) = operation_function(
            operation,
            &declaration_names,
            document,
            names,
            options,
            preserve_wide_integers,
            validation,
            plan,
        )?;
        let imports = validation.take_imports();
        let (parents, leaf) = operation
            .id
            .split_last()
            .map(|(leaf, parents)| (parents, camel(leaf)))
            .ok_or_else(|| EmitError("operation id cannot be empty".into()))?;
        let (member, rest) = match options.namespaces {
            Namespaces::Nested => match parents.split_first() {
                Some((parent, rest)) => (camel(parent), rest),
                None => (leaf.clone(), &[][..]),
            },
            Namespaces::Flat => (
                operation
                    .id
                    .iter()
                    .map(|segment| camel(segment))
                    .collect::<Vec<_>>()
                    .join("."),
                &[][..],
            ),
        };
        let namespace = namespaces.entry(member.clone()).or_default();
        if options.namespaces == Namespaces::Flat && !namespace.declarations.is_empty() {
            return Err(EmitError(format!("client member collision at {member}")));
        }
        if !imports.is_empty() {
            let capabilities = validation
                .capabilities()
                .expect("a backend that imports a validator has capabilities");
            namespace
                .validation_module
                .get_or_insert(capabilities.module);
            namespace.imports.merge(imports);
        }
        namespace.declarations.push(declarations);
        namespace.aliases.extend(declaration_names.iter().cloned());
        namespace.is_operation |= parents.is_empty() || options.namespaces == Namespaces::Flat;
        namespace.declares_api_error |= !operation.errors.is_empty()
            || (operation.errors_inherit && !document.common_errors.is_empty());
        namespace.encodes_path |= !operation.path.is_empty();
        namespace.carries_path_values |= operation
            .path
            .iter()
            .any(|segment| matches!(segment, PathSegment::Param { .. }));
        let websocket = operation
            .stream
            .as_ref()
            .is_some_and(|stream| matches!(stream.kind, StreamKind::Websocket { .. }));
        let sse = matches!(
            operation.stream.as_ref().map(|stream| &stream.kind),
            Some(StreamKind::Sse { .. })
        );
        let ndjson = matches!(
            operation.stream.as_ref().map(|stream| &stream.kind),
            Some(StreamKind::Ndjson { .. })
        );
        namespace.sse |= sse;
        namespace.ndjson |= ndjson;
        namespace.streams |= sse || ndjson;
        namespace.websockets |= websocket;
        namespace.paginates |= iterator.is_some();
        namespace.uses_request |= !websocket;
        if !websocket {
            namespace.wraps_status |= operation.responses.iter().any(|response| {
                matches!(
                    response.status,
                    ResponseStatus::Default | ResponseStatus::Unknown
                )
            });
            namespace.declares_headers |= operation
                .responses
                .iter()
                .any(|response| !response.headers.is_empty());
        }
        namespace.declares_headers |= operation.errors_inherit
            && document
                .common_errors
                .iter()
                .any(|error| !error.headers.is_empty());
        let mut node = &mut namespace.node;
        let path = rest
            .iter()
            .map(|segment| camel(segment))
            .collect::<Vec<_>>();
        for parent in rest {
            let segment = camel(parent);
            if node.operations.iter().any(|(_, name, _)| name == &segment) {
                return Err(EmitError(format!("client member collision at {segment}")));
            }
            node = node.children.entry(segment).or_default();
        }
        if node.children.contains_key(&leaf)
            || node.operations.iter().any(|(_, name, _)| name == &leaf)
        {
            return Err(EmitError(format!("client member collision at {leaf}")));
        }
        node.operations
            .push((operation.doc.clone(), leaf.clone(), function));
        if let Some(iterator) = iterator {
            pending_iterators.push(PendingIterator {
                namespace: member.clone(),
                path,
                name: format!("{leaf}All"),
                owner: format!("operations.{}", operation_subject(operation)),
                member: iterator,
            });
        }
    }
    for pending in pending_iterators {
        let namespace = namespaces
            .get_mut(&pending.namespace)
            .expect("a pending iterator names an emitted namespace");
        let mut node = &mut namespace.node;
        for segment in &pending.path {
            node = node
                .children
                .get_mut(segment)
                .expect("a pending iterator names an emitted namespace path");
        }
        if node.children.contains_key(&pending.name)
            || node
                .operations
                .iter()
                .any(|(_, name, _)| name == &pending.name)
        {
            return Err(EmitError(format!(
                "the TypeScript emitter cannot emit the pagination iterator {} for {}, because \
                 the client already has a member named {}",
                pending.name, pending.owner, pending.name
            )));
        }
        node.operations.push((None, pending.name, pending.member));
    }
    let mut files = Vec::new();
    for (member, namespace) in &namespaces {
        files.push(GeneratedFile {
            path: format!("operations/{member}.ts"),
            content: namespace_file(namespace, document, options.errors, options.runtime),
        });
    }
    files.push(GeneratedFile {
        path: "client.ts".into(),
        content: client_file(
            document,
            &namespaces,
            opaque_required,
            !preserve_wide_integers,
            options,
        ),
    });
    Ok(files)
}

struct PendingIterator {
    namespace: String,
    path: Vec<String>,
    name: String,
    owner: String,
    member: String,
}

#[derive(Default)]
struct Namespace {
    node: Node,
    declarations: Vec<String>,
    aliases: Vec<String>,
    encodes_path: bool,
    carries_path_values: bool,
    streams: bool,
    sse: bool,
    ndjson: bool,
    websockets: bool,
    paginates: bool,
    declares_api_error: bool,
    uses_request: bool,
    wraps_status: bool,
    declares_headers: bool,
    is_operation: bool,
    validation_module: Option<&'static str>,
    imports: validation::Imports,
}

fn namespace_file(
    namespace: &Namespace,
    document: &Document,
    error_strategy: ErrorStrategy,
    runtime: Runtime,
) -> String {
    let mut members = Vec::new();
    if namespace.uses_request {
        members.push("request".to_owned());
    }
    if error_strategy == ErrorStrategy::Throw {
        members.push("requestOrThrow".into());
    }
    if namespace.encodes_path {
        members.push("encodePathSegment".into());
    }
    if namespace.carries_path_values {
        members.push("stringifyCarrier".into());
    }
    if namespace.websockets {
        members.push("websocket".into());
    }
    if namespace.paginates {
        members.push("paginate".into());
    }
    members.push("type ConfiguredClient".into());
    let mut output = String::new();
    if !document.types.is_empty() {
        output.push_str("import type * as Types from \"../types.js\";\n");
    }
    output.push_str("import type { CallOptions } from \"../client.js\";\n");
    let mut declared = vec!["ClientFailure"];
    if namespace.declares_api_error {
        declared.push("ApiError");
    }
    if namespace.declares_headers {
        declared.push("ResponseWithHeaders");
    }
    if namespace.wraps_status {
        declared.push("ResponseWithStatus");
    }
    if namespace.streams {
        declared.push("Result");
    }
    if namespace.sse {
        declared.push("SseStreamError");
    }
    if namespace.ndjson {
        declared.push("NdjsonStreamError");
    }
    if namespace.websockets {
        declared.push("WebSocketFailure");
    }
    output.push_str(&format!(
        "import type {{ {} }} from \"../errors.js\";\n",
        declared.join(", ")
    ));
    let runtime_specifier = crate::runtime::specifier(runtime, "\"../runtime.js\"");
    output.push_str(&format!(
        "import {{ {} }} from {runtime_specifier};\n",
        members.join(", "),
    ));
    if namespace.websockets {
        output.push_str(&format!(
            "import type {{ WebSocketConnection }} from {runtime_specifier};\n"
        ));
    }
    if let Some(module) = namespace.validation_module {
        let names = namespace
            .imports
            .names
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        output.push_str(&format!("import {{ {names} }} from \"../{module}.js\";\n"));
    }
    output.push('\n');
    for declaration in &namespace.declarations {
        output.push_str(declaration);
    }
    let body = match namespace.node.operations.as_slice() {
        [(doc, _, function)] if namespace.is_operation => {
            let comment = jsdoc(doc.as_ref());
            let comment = if comment.is_empty() {
                String::new()
            } else {
                format!("  {comment}\n")
            };
            format!("{comment}  return {function};\n")
        }
        _ => format!("  return {};\n", namespace.node.render(4)),
    };
    output.push_str(&format!(
        "export function operations(configured: ConfiguredClient) {{\n{body}}}\n"
    ));
    output
}

fn client_file(
    document: &Document,
    namespaces: &BTreeMap<String, Namespace>,
    opaque_required: bool,
    allow_unsafe_wide_integers: bool,
    options: &TsOptions,
) -> String {
    let mut output = String::new();
    if options.runtime == Runtime::Package {
        output.push_str(&format!(
            "// runtime = \"package\": the runtime is the {package} package beside this file, not \
             the vendored runtime/*.ts the default writes here\n",
            package = crate::runtime::PACKAGE
        ));
    }
    for member in namespaces.keys() {
        output.push_str(&format!(
            "import {{ operations as {} }} from \"./operations/{member}.js\";\n",
            binding(member)
        ));
    }
    let runtime = crate::runtime::specifier(options.runtime, "\"./runtime.js\"");
    output.push_str(&format!(
        "import type {{ ConfiguredClient, CredentialValue, OpaqueCodecRegistry, WebSocketConstructor }} from {runtime};\n"
    ));
    output.push_str(&format!(
        "export type {{ OpaqueCodec, OpaqueCodecRegistry, OpaqueWire }} from {runtime};\n"
    ));
    for (member, namespace) in namespaces {
        output.push_str(&format!(
            "export type {{ {} }} from \"./operations/{member}.js\";\n",
            namespace.aliases.join(", ")
        ));
    }
    output.push('\n');
    output.push_str(&option_types(opaque_required));
    output.push_str(&auth_types(document));
    output.push_str(&format!("export function createClient(client: ClientOptions) {{\n  const auth: Record<string, CredentialSource> = Object.assign(Object.create(null), client.auth ?? {{}});\n  const authAdapters: Record<string, AuthAdapter> = Object.assign(Object.create(null), client.authAdapters ?? {{}});\n  const authRecords: Record<string, CredentialValue[]> = Object.create(null);\n  const authInvalidated: Record<string, boolean> = Object.create(null);\n  const configured: ConfiguredClient = {{ ...client, auth, authAdapters, authRecords, authInvalidated, authSchemes, opaque: {{ codecs: client.codecs, required: {opaque_required}, allowUnsafeWideIntegers: {allow_unsafe_wide_integers} }} }};\n  return {{\n"));
    for (index, member) in namespaces.keys().enumerate() {
        let key = member_key(member, options.namespaces);
        let tail = if index + 1 == namespaces.len() {
            ""
        } else {
            ","
        };
        output.push_str(&format!(
            "    {key}: {}(configured){tail}\n",
            binding(member)
        ));
    }
    output.push_str("  };\n}\n");
    output
}

fn binding(member: &str) -> String {
    identifier(&format!("{member}Operations"), "operations")
}

fn member_key(member: &str, namespaces: Namespaces) -> String {
    if member == "__proto__" || (namespaces == Namespaces::Flat && !is_plain_identifier(member)) {
        format!("[{}]", string(member))
    } else {
        member.to_owned()
    }
}
