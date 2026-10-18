use std::collections::BTreeMap;

use pons_ir::{
    ApiKeyLocation, AuthRequirement, AuthScheme, ContentKind, CredentialLocator,
    CredentialProvision, Doc, Document, ErrorContent, ErrorResponse, ErrorStatus, Operation,
    PathSegment, Presence, Response, ResponseHeader, ResponseStatus, StreamKind, TypeRef,
    operation_subject,
};

use crate::{
    DateMode, EmitError, EmitMode, ErrorStrategy, GeneratedFile, Int64, Namespaces, Runtime,
    TsOptions, camel, identifier, is_plain_identifier, jsdoc, pascal, string, types, union_type,
    unique_names, validation,
};

#[derive(Default)]
struct Node {
    children: BTreeMap<String, Node>,
    operations: Vec<(Option<Doc>, String, String)>,
}

impl Node {
    fn render(&self, depth: usize) -> String {
        let indent = " ".repeat(depth);
        let mut members = self
            .operations
            .iter()
            .map(|(doc, name, value)| {
                let key = if name == "__proto__" {
                    format!("[{}]", string(name))
                } else {
                    name.clone()
                };
                let comment = jsdoc(doc.as_ref());
                if comment.is_empty() {
                    format!("{indent}{key}: {value}")
                } else {
                    format!("{indent}{comment}\n{indent}{key}: {value}")
                }
            })
            .collect::<Vec<_>>();
        members.extend(self.children.iter().map(|(name, child)| {
            let key = if name == "__proto__" {
                format!("[{}]", string(name))
            } else {
                name.clone()
            };
            format!("{indent}{key}: {}", child.render(depth + 2))
        }));
        format!(
            "{{\n{}\n{}}}",
            members.join(",\n"),
            " ".repeat(depth.saturating_sub(2))
        )
    }
}

fn operation_type_names(document: &Document) -> Vec<Vec<String>> {
    let mut counts = Vec::with_capacity(document.operations.len());
    let mut candidates = Vec::new();
    for operation in &document.operations {
        let id = operation.id.iter().flatten().cloned().collect::<Vec<_>>();
        let name = pascal(&id);
        candidates.extend([
            format!("{name}Args"),
            format!("{name}Response"),
            format!("{name}Error"),
        ]);
        let mut extra = Vec::new();
        match operation.stream.as_ref().map(|stream| &stream.kind) {
            Some(StreamKind::Sse { .. }) => extra.push(format!("{name}Event")),
            Some(StreamKind::Ndjson { .. }) => extra.push(format!("{name}Item")),
            Some(StreamKind::Websocket { .. }) => {
                extra.push(format!("{name}ClientMessage"));
                extra.push(format!("{name}ServerMessage"));
            }
            _ => {}
        }
        if !extra.is_empty() {
            extra.push(format!("{name}StreamError"));
        }
        counts.push(3 + extra.len());
        candidates.extend(extra);
    }
    let mut names = unique_names(candidates, &[]).into_iter();
    counts
        .into_iter()
        .map(|count| {
            let allocated = names.by_ref().take(count).collect::<Vec<_>>();
            debug_assert_eq!(allocated.len(), count);
            allocated
        })
        .collect()
}

pub(super) fn error_types(error_strategy: ErrorStrategy) -> String {
    let mut output = if error_strategy == ErrorStrategy::Throw {
        String::from(
            "type Result<T, E> = { isOk(): boolean; isErr(): boolean; value?: T; error?: E };\nfunction ok<T>(value: T): Result<T, never> { return { isOk: () => true, isErr: () => false, value }; }\nfunction err<E>(error: E): Result<never, E> { return { isOk: () => false, isErr: () => true, error }; }\nexport { ok, err };\nexport type { Result };\n",
        )
    } else {
        String::from(
            "import { err, ok, type Result } from \"neverthrow\";\nexport { err, ok };\nexport type { Result };\n",
        )
    };
    output.push_str("export type ResponseWithHeaders<T, H> = { value: T; headers: H };\n");
    output.push_str("export type ResponseWithStatus<T> = { value: T; status: number };\n");
    output.push_str("export type ClientFailure = { kind: \"network\"; cause: unknown } | { kind: \"timeout\"; afterMs: number } | { kind: \"aborted\"; cause: unknown } | { kind: \"auth_adapter\"; scheme: string; cause: unknown } | { kind: \"auth_source\"; scheme: string; cause: unknown } | { kind: \"opaque_codec\"; codec: string; phase: \"encode\" | \"decode\"; cause: unknown } | { kind: \"transport\"; phase: \"request\" | \"response\"; cause: unknown } | { kind: \"protocol\"; message: string; cause?: unknown } | { kind: \"unexpected_http\"; status: number; body: unknown } | { kind: \"auth\"; schemes: string[] };\n");
    output
}

pub(super) fn index(validation: Option<&str>) -> GeneratedFile {
    GeneratedFile {
        path: "index.ts".into(),
        content: format!(
            "export * from \"./client.js\";\n{}",
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
) -> Result<Vec<GeneratedFile>, EmitError> {
    let opaque_required = options.mode == EmitMode::Strict && document.has_reachable_opaque();
    let mut namespaces: BTreeMap<String, Namespace> = BTreeMap::new();
    let operation_names = operation_type_names(document);
    for (operation, declaration_names) in document.operations.iter().zip(operation_names) {
        let (declarations, function) = operation_function(
            operation,
            &declaration_names,
            document,
            names,
            options,
            preserve_wide_integers,
            validation,
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
        namespace.encodes_path |= !operation.path.is_empty();
        namespace.carries_path_values |= operation
            .path
            .iter()
            .any(|segment| matches!(segment, PathSegment::Param { .. }));
        let websocket = operation
            .stream
            .as_ref()
            .is_some_and(|stream| matches!(stream.kind, StreamKind::Websocket { .. }));
        namespace.streams |= matches!(
            operation.stream.as_ref().map(|stream| &stream.kind),
            Some(StreamKind::Sse { .. } | StreamKind::Ndjson { .. })
        );
        namespace.websockets |= websocket;
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
            .push((operation.doc.clone(), leaf, function));
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

#[derive(Default)]
struct Namespace {
    node: Node,
    declarations: Vec<String>,
    aliases: Vec<String>,
    encodes_path: bool,
    carries_path_values: bool,
    streams: bool,
    websockets: bool,
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
    members.push("type ConfiguredClient".into());
    let mut output = String::new();
    if !document.types.is_empty() {
        output.push_str("import type * as Types from \"../types.js\";\n");
    }
    output.push_str("import type { CallOptions } from \"../client.js\";\n");
    let mut declared = vec!["ClientFailure"];
    if namespace.declares_headers {
        declared.push("ResponseWithHeaders");
    }
    if namespace.wraps_status {
        declared.push("ResponseWithStatus");
    }
    if namespace.streams {
        declared.push("Result");
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
            "import type {{ WebSocketConnection, WebSocketFailure }} from {runtime_specifier};\n"
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

fn option_types(opaque_required: bool) -> String {
    let mut output = String::new();
    output.push_str("export type AuthAttachment = { headers?: Record<string, string>; query?: Record<string, string>; cookies?: Record<string, string> };\nexport type AuthAdapter = (context: { scheme: string; scopes: readonly string[]; method: string; url: URL; headers: Headers }) => AuthAttachment | Promise<AuthAttachment>;\n/** Scope and lifetime captured with a credential. `expiresAt` is a Unix millisecond timestamp; `host`, `path`, `secure`, and `origin` bind the value to the response that issued it. */\nexport type CredentialMetadata = { expiresAt?: number | undefined; path?: string | undefined; secure?: boolean | undefined; host?: string | undefined; origin?: string | undefined };\nexport type StoredCredential = { value: string } & CredentialMetadata;\n/** A store returns a `{ value, ...metadata }` record, never a bare string, and must persist the metadata it is given in `set` so that expiry and origin scope survive a new client. A record with a non-string `value` or a metadata field of the wrong type is rejected. */\nexport type TokenStore = { get(): StoredCredential | undefined | Promise<StoredCredential | undefined>; set(value: string, metadata?: CredentialMetadata): void | Promise<void>; clear(): void | Promise<void> };\nexport type CredentialSource = string | (() => string | undefined | Promise<string | undefined>) | TokenStore;\nexport type TransportRequest = { url: URL; init: RequestInit };\nexport type TransportResponse = { response: Response; url: URL; init: RequestInit };\nexport type TransportRequestHook = (context: TransportRequest) => void | Promise<void>;\nexport type TransportResponseHook = (context: TransportResponse) => void | Promise<void>;\nexport type Transport = { onRequest?: TransportRequestHook; onResponse?: TransportResponseHook };\nexport type AuthSchemeDefinition = { kind: string; location?: string | undefined; name?: string | undefined; adapter?: string | undefined };\nexport type RetryOptions = { attempts?: number; backoffMs?: number; maxBackoffMs?: number; jitterMs?: number };\nexport type CallOptions = { headers?: HeadersInit; signal?: AbortSignal; timeoutMs?: number; retry?: RetryOptions | false };\n");
    output.push_str(&format!(
        "export type ClientOptions = {{ baseUrl: string; headers?: HeadersInit | (() => HeadersInit | Promise<HeadersInit>); auth?: Record<string, CredentialSource>; authAdapters?: Record<string, AuthAdapter>; codecs{}: OpaqueCodecRegistry; fetch?: typeof fetch; webSocket?: WebSocketConstructor; signal?: AbortSignal; timeoutMs?: number; retry?: RetryOptions; transport?: Transport }};\n\n",
        if opaque_required { "" } else { "?" },
    ));
    output
}

fn auth_types(document: &Document) -> String {
    let mut entries = Vec::new();
    for (id, scheme) in &document.auth_schemes {
        let key = string(id);
        let value = match scheme {
            AuthScheme::Bearer { .. } | AuthScheme::OAuth2 { .. } => "{ kind: \"bearer\" }".into(),
            AuthScheme::Basic => "{ kind: \"basic\" }".into(),
            AuthScheme::ApiKey { location, name } => format!(
                "{{ kind: \"api_key\", location: {}, name: {} }}",
                string(match location {
                    ApiKeyLocation::Header => "header",
                    ApiKeyLocation::Query => "query",
                    ApiKeyLocation::Cookie => "cookie",
                }),
                string(name)
            ),
            AuthScheme::CookieSession { cookie } => {
                format!("{{ kind: \"cookie\", name: {} }}", string(cookie))
            }
            AuthScheme::Custom { adapter, .. } => match adapter {
                Some(adapter) => format!("{{ kind: \"custom\", adapter: {} }}", string(adapter)),
                None => "{ kind: \"custom\" }".into(),
            },
        };
        entries.push(format!("authSchemes[{key}] = {value};"));
    }
    format!(
        "const authSchemes: Record<string, AuthSchemeDefinition> = Object.create(null);\n{}\n\n",
        entries.join("\n")
    )
}

fn operation_function(
    operation: &Operation,
    declaration_names: &[String],
    document: &Document,
    names: &BTreeMap<String, String>,
    options: &TsOptions,
    preserve_wide_integers: bool,
    validation: &mut validation::Backend,
) -> Result<(String, String), EmitError> {
    let args_name = &declaration_names[0];
    let success_name = &declaration_names[1];
    let error_name = &declaration_names[2];
    let args = operation_args(operation, document, names, options)?;
    let (success, stream_types) =
        response_type(operation, declaration_names, document, names, options)?;
    let errors = if operation.errors_inherit {
        operation
            .errors
            .iter()
            .chain(document.common_errors.iter())
            .collect::<Vec<_>>()
    } else {
        operation.errors.iter().collect()
    };
    let failure = error_type(&errors, document, names, options)?;
    let subject = format!("operations.{}", operation_subject(operation));
    let method = operation.method.as_str();
    let (args_type, call_options) = if args.is_empty() {
        ("(_options?: CallOptions) =>".to_owned(), "_options")
    } else {
        (
            format!("(args: {args_name}, options?: CallOptions) =>"),
            "options",
        )
    };
    if let Some(StreamKind::Websocket {
        client: client_message,
        server: server_message,
    }) = operation.stream.as_ref().map(|stream| &stream.kind)
    {
        let client_name = &declaration_names[3];
        let server_name = &declaration_names[4];
        let spec = websocket_spec(
            operation,
            client_message,
            server_message,
            options,
            validation,
            &subject,
            preserve_wide_integers,
            call_options,
        )?;
        let declarations = format!(
            "export type {args_name} = {{ {args} }};\n{stream_types}export type {success_name} = {success};\nexport type {error_name} = {failure};\n",
        );
        let call = format!("websocket<{client_name}, {server_name}>(configured, {{ {spec} }})");
        let function = if options.errors == ErrorStrategy::Throw {
            format!("{args_type} requestOrThrow<{success_name}, {error_name}>({call})")
        } else {
            format!("{args_type} {call}")
        };
        return Ok((declarations, function));
    }
    let mut spec = vec![
        format!("method: {method:?}"),
        format!("path: {}", path_expression(operation)),
    ];
    if operation
        .path
        .iter()
        .any(|segment| matches!(segment, PathSegment::Param { .. }))
    {
        spec.push(String::from("pathValues: args.path"));
    }
    if operation.query.is_some() {
        spec.push(String::from("query: args.query"));
    }
    if !operation.headers.is_empty() {
        spec.push(String::from("headers: args.headers"));
    }
    if let Some(body) = &operation.body {
        spec.push(String::from("body: args.body"));
        spec.push(format!(
            "bodyContent: {}",
            string(content_name(body.content))
        ));
        if let Some(media_type) = &body.media_type {
            spec.push(format!("bodyMediaType: {}", string(media_type)));
        }
        spec.push(format!("bodySchema: {}", type_ref_json(&body.ty, options)?));
    }
    if !operation.auth.alternatives.is_empty() {
        spec.push(format!("auth: {}", auth_requirement(&operation.auth)));
    }
    spec.push(format!(
        "inputs: {}",
        request_input_descriptor(operation, options)?
    ));
    spec.push(format!(
        "responses: [{}]",
        response_descriptors(operation, options, validation)?
    ));
    spec.push(format!(
        "errors: [{}]",
        errors
            .iter()
            .map(|error| error_descriptor(error, options, validation, &subject))
            .collect::<Result<Vec<_>, _>>()?
            .join(", ")
    ));
    if preserve_wide_integers {
        spec.push(String::from("preserveWideIntegers: true"));
    }
    if let Some(provides) = &operation.provides {
        spec.push(format!("provides: {}", credential_provision(provides)));
    }
    spec.push(format!("options: {call_options}"));
    if operation.semantics.idempotent {
        spec.push(String::from("idempotent: true"));
    }
    let declarations = format!(
        "export type {args_name} = {{ {args} }};\n{stream_types}export type {success_name} = {success};\nexport type {error_name} = {failure};\n",
    );
    let request = format!(
        "request<{success_name}, {error_name}>(configured, {{ {} }})",
        spec.join(", ")
    );
    let function = if options.errors == ErrorStrategy::Throw {
        format!("{args_type} requestOrThrow({request})")
    } else {
        format!("{args_type} {request}")
    };
    Ok((declarations, function))
}

fn auth_requirement(auth: &AuthRequirement) -> String {
    let alternatives = auth
        .alternatives
        .iter()
        .map(|group| {
            format!(
                "[{}]",
                group
                    .iter()
                    .map(|scheme| string(scheme))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let mut keys = vec![format!("alternatives: [{alternatives}]")];
    if !auth.scopes.is_empty() {
        let scopes = auth
            .scopes
            .iter()
            .map(|(scheme, scopes)| {
                let key = string(scheme);
                let key = if scheme == "__proto__" {
                    format!("[{key}]")
                } else {
                    key
                };
                format!(
                    "{key}: [{}]",
                    scopes
                        .iter()
                        .map(|scope| string(scope))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        keys.push(format!("scopes: {{{scopes}}}"));
    }
    if auth.optional {
        keys.push(String::from("optional: true"));
    }
    format!("{{ {} }}", keys.join(", "))
}

fn credential_provision(provides: &CredentialProvision) -> String {
    let locator = |locator: &CredentialLocator| match locator {
        CredentialLocator::Body { pointer } => {
            format!("{{ from: \"body\", pointer: {} }}", string(pointer))
        }
        CredentialLocator::Header { name } => {
            format!("{{ from: \"header\", name: {} }}", string(name))
        }
        CredentialLocator::SetCookie { name } => {
            format!("{{ from: \"set_cookie\", name: {} }}", string(name))
        }
    };
    let value = locator(&provides.value);
    let expires_at = provides
        .expires_at
        .as_ref()
        .map(locator)
        .unwrap_or_else(|| "undefined".into());
    format!(
        "{{ scheme: {}, value: {value}, expiresAt: {expires_at} }}",
        string(&provides.scheme)
    )
}

fn operation_args(
    operation: &Operation,
    document: &Document,
    names: &BTreeMap<String, String>,
    options: &TsOptions,
) -> Result<String, EmitError> {
    let mut fields = Vec::new();
    let mut path_params = Vec::new();
    for segment in &operation.path {
        if let PathSegment::Param { param, ty } = segment {
            path_params.push(format!(
                "{}: {}",
                string(param),
                types::type_ref(ty, document, names, options)?
            ));
        }
    }
    if !path_params.is_empty() {
        fields.push(format!("path: {{ {} }}", path_params.join("; ")));
    }
    if let Some(query) = &operation.query {
        fields.push(format!(
            "query: {}",
            types::type_ref(query, document, names, options)?
        ));
    }
    if !operation.headers.is_empty() {
        let headers = operation
            .headers
            .iter()
            .map(|header| {
                let optional = if header.presence == pons_ir::Presence::Optional {
                    "?"
                } else {
                    ""
                };
                Ok(format!(
                    "{}{}: {}",
                    string(&header.wire),
                    optional,
                    types::type_ref(&header.ty, document, names, options)?
                ))
            })
            .collect::<Result<Vec<_>, EmitError>>()?;
        let bag = if operation
            .headers
            .iter()
            .all(|header| header.presence == pons_ir::Presence::Optional)
        {
            "?"
        } else {
            ""
        };
        fields.push(format!("headers{bag}: {{ {} }}", headers.join("; ")));
    }
    if let Some(body) = &operation.body {
        let optional = if body.required { "" } else { "?" };
        fields.push(format!(
            "body{optional}: {}",
            types::type_ref(&body.ty, document, names, options)?
        ));
    }
    Ok(fields.join("; "))
}

fn response_type(
    operation: &Operation,
    declaration_names: &[String],
    document: &Document,
    names: &BTreeMap<String, String>,
    options: &TsOptions,
) -> Result<(String, String), EmitError> {
    if let Some(StreamKind::Websocket {
        client: client_message,
        server: server_message,
    }) = operation.stream.as_ref().map(|stream| &stream.kind)
    {
        let client_name = &declaration_names[3];
        let server_name = &declaration_names[4];
        let failure_name = &declaration_names[5];
        let response = format!("WebSocketConnection<{client_name}, {server_name}>");
        let declarations = format!(
            "export type {client_name} = {};\nexport type {server_name} = {};\nexport type {failure_name} = WebSocketFailure;\n",
            types::type_ref(client_message, document, names, options)?,
            types::type_ref(server_message, document, names, options)?,
        );
        return Ok((response, declarations));
    }
    if let Some(StreamKind::Sse { events }) = operation.stream.as_ref().map(|stream| &stream.kind) {
        let event_name = &declaration_names[3];
        let failure_name = &declaration_names[4];
        let variants = events
            .iter()
            .filter(|event| event.event != "error")
            .map(|event| {
                Ok(format!(
                    "{{ event: {}; data: {} }}",
                    string(&event.event),
                    types::type_ref(&event.data, document, names, options)?
                ))
            })
            .collect::<Result<Vec<_>, EmitError>>()?;
        let mut stream_failures = vec![
            "{ kind: \"stream_error\"; cause: unknown }".to_owned(),
            "{ kind: \"unknown_event\"; event: string }".to_owned(),
            "{ kind: \"invalid_event\"; event: string; cause: unknown }".to_owned(),
        ];
        for event in events.iter().filter(|event| event.event == "error") {
            stream_failures.push(format!(
                "{{ kind: \"server_error\"; body: {} }}",
                types::type_ref(&event.data, document, names, options)?
            ));
        }
        let stream_type = format!("AsyncIterable<Result<{event_name}, {failure_name}>>");
        let response_type =
            stream_response_type(operation, &stream_type, document, names, options)?;
        return Ok((
            response_type,
            format!(
                "export type {event_name} = {};\nexport type {failure_name} = {};\n",
                if variants.is_empty() {
                    "never".into()
                } else {
                    variants.join(" | ")
                },
                stream_failures.join(" | ")
            ),
        ));
    }
    if let Some(StreamKind::Ndjson { item }) = operation.stream.as_ref().map(|stream| &stream.kind)
    {
        let item_name = &declaration_names[3];
        let failure_name = &declaration_names[4];
        let stream_type = format!("AsyncIterable<Result<{item_name}, {failure_name}>>");
        return Ok((
            stream_response_type(operation, &stream_type, document, names, options)?,
            format!(
                "export type {item_name} = {};\nexport type {failure_name} = {{ kind: \"stream_error\"; cause: unknown }} | {{ kind: \"invalid_item\"; cause: unknown }};\n",
                types::type_ref(item, document, names, options)?
            ),
        ));
    }
    let bodies = operation
        .responses
        .iter()
        .map(|response| {
            let body = match &response.body {
                Some(body) => types::type_ref(body, document, names, options),
                None => Ok("void".into()),
            }?;
            response_value_type(body, response, document, names, options)
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok((
        if bodies.is_empty() {
            "void".into()
        } else {
            bodies.join(" | ")
        },
        String::new(),
    ))
}

fn stream_response_type(
    operation: &Operation,
    stream_type: &str,
    document: &Document,
    names: &BTreeMap<String, String>,
    options: &TsOptions,
) -> Result<String, EmitError> {
    let mut responses = Vec::new();
    for response in &operation.responses {
        let body = if response.content == ContentKind::Stream {
            stream_type.to_owned()
        } else {
            match &response.body {
                Some(body) => types::type_ref(body, document, names, options)?,
                None => "void".into(),
            }
        };
        responses.push(response_value_type(
            body, response, document, names, options,
        )?);
    }
    if responses.is_empty() {
        Ok(stream_type.into())
    } else {
        Ok(responses.join(" | "))
    }
}

fn response_value_type(
    body: String,
    response: &Response,
    document: &Document,
    names: &BTreeMap<String, String>,
    options: &TsOptions,
) -> Result<String, EmitError> {
    let body = if response.headers.is_empty() {
        body
    } else {
        let headers = response_headers_type(&response.headers, document, names, options)?;
        format!("ResponseWithHeaders<{body}, {headers}>")
    };
    if matches!(
        response.status,
        ResponseStatus::Default | ResponseStatus::Unknown
    ) {
        Ok(format!("ResponseWithStatus<{body}>"))
    } else {
        Ok(body)
    }
}

fn response_headers_type(
    headers: &[ResponseHeader],
    document: &Document,
    names: &BTreeMap<String, String>,
    options: &TsOptions,
) -> Result<String, EmitError> {
    let fields = headers
        .iter()
        .map(|header| {
            let optional = if header.presence == Presence::Optional {
                "?"
            } else {
                ""
            };
            Ok(format!(
                "{}{}: {}",
                string(&header.wire),
                optional,
                types::type_ref(&header.ty, document, names, options)?
            ))
        })
        .collect::<Result<Vec<_>, EmitError>>()?;
    Ok(format!("{{ {} }}", fields.join("; ")))
}

fn error_type(
    errors: &[&ErrorResponse],
    document: &Document,
    names: &BTreeMap<String, String>,
    options: &TsOptions,
) -> Result<String, EmitError> {
    let mut failures = vec![(None, "ClientFailure".to_owned())];
    for error in errors {
        let status = match error.status {
            ErrorStatus::Exact(status) => status.to_string(),
            ErrorStatus::Class(_) => "number".into(),
            ErrorStatus::Default => "number".into(),
        };
        let body = error
            .body
            .as_ref()
            .map(|body| types::type_ref(body, document, names, options))
            .transpose()?;
        let headers = if error.headers.is_empty() {
            "Headers".to_owned()
        } else {
            response_headers_type(&error.headers, document, names, options)?
        };
        let body = body.unwrap_or_else(|| "unknown".into());
        failures.push((
            error.doc.as_ref(),
            format!("{{ kind: \"http\"; status: {status}; body: {body}; headers: {headers} }}"),
        ));
    }
    Ok(union_type(&failures))
}

fn request_input_descriptor(
    operation: &Operation,
    options: &TsOptions,
) -> Result<String, EmitError> {
    let path = operation
        .path
        .iter()
        .filter_map(|segment| match segment {
            PathSegment::Param { param, ty } => Some((param, ty)),
            PathSegment::Literal { .. } => None,
        })
        .map(|(param, ty)| {
            Ok(format!(
                "{{ name: {}, required: true, schema: {} }}",
                string(param),
                type_ref_json(ty, options)?
            ))
        })
        .collect::<Result<Vec<_>, EmitError>>()?
        .join(", ");
    let query = operation
        .query
        .as_ref()
        .map(|ty| type_ref_json(ty, options))
        .transpose()?
        .unwrap_or_else(|| "undefined".into());
    let headers = operation
        .headers
        .iter()
        .map(|header| {
            Ok(format!(
                "{{ name: {}, required: {}, schema: {} }}",
                string(&header.wire),
                header.presence == Presence::Required,
                type_ref_json(&header.ty, options)?
            ))
        })
        .collect::<Result<Vec<_>, EmitError>>()?
        .join(", ");
    let body_required = operation.body.as_ref().is_some_and(|body| body.required);
    Ok(format!(
        "{{ path: [{path}], query: {query}, headers: [{headers}], bodyRequired: {body_required} }}"
    ))
}

fn path_expression(operation: &Operation) -> String {
    if operation.path.is_empty() {
        return "\"/\"".into();
    }
    let has_params = operation
        .path
        .iter()
        .any(|segment| matches!(segment, PathSegment::Param { .. }));
    let segments = operation
        .path
        .iter()
        .map(|segment| match segment {
            PathSegment::Literal { lit } => format!("encodePathSegment({})", string(lit)),
            PathSegment::Param { param, .. } => {
                format!(
                    "encodePathSegment(stringifyCarrier(pathValues[{}]))",
                    string(param)
                )
            }
        })
        .collect::<Vec<_>>();
    let path = format!("[\"\", {}].join(\"/\")", segments.join(", "));
    if has_params {
        format!("(pathValues) => {path}")
    } else {
        path
    }
}

fn content_name(content: ContentKind) -> &'static str {
    match content {
        ContentKind::Json => "json",
        ContentKind::ProblemJson => "problem_json",
        ContentKind::Form => "form",
        ContentKind::Multipart => "multipart",
        ContentKind::Text => "text",
        ContentKind::Bytes => "bytes",
        ContentKind::Stream => "stream",
        ContentKind::Empty => "empty",
    }
}

#[allow(clippy::too_many_arguments)]
fn websocket_spec(
    operation: &Operation,
    client_message: &TypeRef,
    server_message: &TypeRef,
    options: &TsOptions,
    validation: &mut validation::Backend,
    subject: &str,
    preserve_wide_integers: bool,
    call_options: &str,
) -> Result<String, EmitError> {
    let mut spec = vec![format!("path: {}", path_expression(operation))];
    if operation
        .path
        .iter()
        .any(|segment| matches!(segment, PathSegment::Param { .. }))
    {
        spec.push(String::from("pathValues: args.path"));
    }
    if operation.query.is_some() {
        spec.push(String::from("query: args.query"));
    }
    if !operation.headers.is_empty() {
        spec.push(String::from("headers: args.headers"));
    }
    if !operation.auth.alternatives.is_empty() {
        spec.push(format!("auth: {}", auth_requirement(&operation.auth)));
    }
    spec.push(format!(
        "inputs: {}",
        request_input_descriptor(operation, options)?
    ));
    spec.push(format!(
        "client: {{ schema: {} }}",
        type_ref_json(client_message, options)?
    ));
    let validate = validator(Some(server_message), validation, subject)?;
    spec.push(format!(
        "server: {{ schema: {}{validate} }}",
        type_ref_json(server_message, options)?
    ));
    if preserve_wide_integers {
        spec.push(String::from("preserveWideIntegers: true"));
    }
    spec.push(format!("options: {call_options}"));
    Ok(spec.join(", "))
}

fn response_descriptors(
    operation: &Operation,
    options: &TsOptions,
    validation: &mut validation::Backend,
) -> Result<String, EmitError> {
    let subject = format!("operations.{}", operation_subject(operation));
    let stream_descriptor = match operation.stream.as_ref().map(|stream| &stream.kind) {
        Some(StreamKind::Sse { events }) => {
            let events = events
                .iter()
                .map(|event| {
                    let validate = validator(Some(&event.data), validation, &subject)?;
                    Ok(format!(
                        "{{ event: {}, schema: {}{validate} }}",
                        string(&event.event),
                        type_ref_json(&event.data, options)?
                    ))
                })
                .collect::<Result<Vec<_>, EmitError>>()?
                .join(", ");
            Some(format!("stream: \"sse\", events: [{events}]"))
        }
        Some(StreamKind::Ndjson { .. }) => Some("stream: \"ndjson\"".into()),
        _ => None,
    };
    operation
        .responses
        .iter()
        .map(|response| {
            let item = match (response.content, operation.stream.as_ref()) {
                (ContentKind::Stream, Some(stream)) => match &stream.kind {
                    StreamKind::Ndjson { item } => Some(item),
                    _ => None,
                },
                _ => None,
            };
            let validate = match item {
                Some(item) => validator(Some(item), validation, &subject)?,
                None if response.content == ContentKind::Stream => String::new(),
                None => validator(response.body.as_ref(), validation, &subject)?,
            };
            let schema = match item {
                Some(item) => type_ref_json(item, options)?,
                None => match &response.body {
                    Some(body) => type_ref_json(body, options),
                    None => Ok("undefined".into()),
                }?,
            };
            let stream = if response.content == ContentKind::Stream {
                stream_descriptor
                    .as_ref()
                    .map_or(String::new(), |stream| format!(", {stream}"))
            } else {
                String::new()
            };
            let media_type = response
                .media_type
                .as_deref()
                .map(|value| format!(", mediaType: {}", string(value)))
                .unwrap_or_default();
            let headers = response
                .headers
                .iter()
                .map(|header| header_descriptor(header, options, validation, &subject))
                .collect::<Result<Vec<_>, EmitError>>()?
                .join(", ");
            let headers = if headers.is_empty() {
                String::new()
            } else {
                format!(", headers: [{headers}]")
            };
            let status = serde_json::to_string(&response.status)
                .map_err(|error| EmitError(format!("cannot encode response status: {error}")))?;
            Ok(format!(
                "{{ status: {status}, content: {}, schema: {schema}{validate}{media_type}{stream}{headers} }}",
                string(content_name(response.content))
            ))
        })
        .collect::<Result<Vec<_>, EmitError>>()
        .map(|descriptors| descriptors.join(", "))
}

fn header_descriptor(
    header: &ResponseHeader,
    options: &TsOptions,
    validation: &mut validation::Backend,
    subject: &str,
) -> Result<String, EmitError> {
    let validate = validator(Some(&header.ty), validation, subject)?;
    Ok(format!(
        "{{ name: {}, required: {}, schema: {}{validate} }}",
        string(&header.wire),
        header.presence == Presence::Required,
        type_ref_json(&header.ty, options)?
    ))
}

fn validator(
    body: Option<&TypeRef>,
    validation: &mut validation::Backend,
    subject: &str,
) -> Result<String, EmitError> {
    let Some(body) = body else {
        return Ok(String::new());
    };
    let Some(expression) = validation.validate(body, subject)? else {
        return Ok(String::new());
    };
    Ok(format!(", validate: {expression}"))
}

fn error_descriptor(
    error: &ErrorResponse,
    options: &TsOptions,
    validation: &mut validation::Backend,
    subject: &str,
) -> Result<String, EmitError> {
    let validate = validator(error.body.as_ref(), validation, subject)?;
    let status = serde_json::to_string(&error.status)
        .map_err(|error| EmitError(format!("cannot encode error status: {error}")))?;
    let content = match error.content {
        ErrorContent::Json => "json",
        ErrorContent::ProblemJson => "problem_json",
        ErrorContent::Text => "text",
        ErrorContent::Empty => "empty",
    };
    let schema = error
        .body
        .as_ref()
        .map(|ty| type_ref_json(ty, options))
        .transpose()?
        .unwrap_or_else(|| "undefined".into());
    let headers = error
        .headers
        .iter()
        .map(|header| header_descriptor(header, options, validation, subject))
        .collect::<Result<Vec<_>, EmitError>>()?
        .join(", ");
    let headers = if headers.is_empty() {
        String::new()
    } else {
        format!(", headers: [{headers}]")
    };
    let retryable = error
        .retryable
        .map(|value| format!(", retryable: {value}"))
        .unwrap_or_default();
    let media_type = error
        .media_type
        .as_deref()
        .map(|value| format!(", mediaType: {}", string(value)))
        .unwrap_or_default();
    Ok(format!(
        "{{ status: {status}, content: {}, schema: {schema}{validate}{media_type}{retryable}{headers} }}",
        string(content)
    ))
}

pub(crate) fn type_ref_json(reference: &TypeRef, options: &TsOptions) -> Result<String, EmitError> {
    let mut value = serde_json::to_value(reference)
        .map_err(|error| EmitError(format!("cannot encode type reference: {error}")))?;
    normalize_integer_constraints(&mut value);
    tag_carriers(&mut value, options);
    serde_json::to_string(&value)
        .map_err(|error| EmitError(format!("cannot encode type reference: {error}")))
}

pub(crate) fn tag_carriers(value: &mut serde_json::Value, options: &TsOptions) {
    match value {
        serde_json::Value::Object(fields) => {
            if fields.get("kind").and_then(serde_json::Value::as_str) == Some("int") {
                let carrier =
                    if fields.get("wire").and_then(serde_json::Value::as_str) == Some("string") {
                        "string"
                    } else if fields
                        .get("bits")
                        .and_then(serde_json::Value::as_u64)
                        .is_some_and(|bits| bits > 53)
                    {
                        match options.int64 {
                            Int64::String => "string",
                            Int64::Bigint if options.mode == EmitMode::Strict => "bigint",
                            Int64::Bigint | Int64::Number => "number",
                        }
                    } else {
                        "number"
                    };
                fields.insert("carrier".into(), serde_json::Value::from(carrier));
            }
            if options.date_mode == DateMode::Date
                && fields.get("kind").and_then(serde_json::Value::as_str) == Some("string")
                && fields.get("format").and_then(serde_json::Value::as_str) == Some("date-time")
            {
                fields.insert("carrier".into(), serde_json::Value::from("date"));
            }
            for child in fields.values_mut() {
                tag_carriers(child, options);
            }
        }
        serde_json::Value::Array(values) => {
            for child in values {
                tag_carriers(child, options);
            }
        }
        _ => {}
    }
}

pub(crate) fn normalize_integer_constraints(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(fields) => {
            if fields.get("kind").and_then(serde_json::Value::as_str) == Some("int") {
                for key in ["minimum", "maximum", "multiple_of"] {
                    if let Some(serde_json::Value::Number(number)) = fields.get(key) {
                        let encoded = number.to_string();
                        fields.insert(key.into(), serde_json::Value::String(encoded));
                    }
                }
            }
            for child in fields.values_mut() {
                normalize_integer_constraints(child);
            }
        }
        serde_json::Value::Array(values) => {
            for child in values {
                normalize_integer_constraints(child);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::{Value, json};

    fn operations_of(operations: Value) -> String {
        let raw = json!({
            "ir_version": pons_ir::IR_VERSION,
            "info": {"title": "Verbs", "version": "1"},
            "operations": operations,
        });
        let document: pons_ir::Document = serde_json::from_value(raw).unwrap();
        crate::emit(&document)
            .unwrap()
            .into_iter()
            .filter(|file| file.path.starts_with("operations/"))
            .map(|file| file.content)
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn every_operation_sends_the_verb_the_document_names_and_not_the_variant() {
        let methods = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];
        let operations = methods
            .iter()
            .map(|method| {
                let name = method.to_lowercase();
                json!({
                    "id": [["verbs"], [name]],
                    "method": method,
                    "path": [{"lit": name}],
                    "auth": {"alternatives": [], "optional": false},
                    "semantics": {"safe": false, "idempotent": false},
                    "responses": [{"status": 200, "content": "empty"}],
                })
            })
            .collect::<Vec<_>>();
        let client = operations_of(Value::Array(operations));
        let sent = client
            .lines()
            .filter_map(|line| {
                let after = line.split_once("configured, { method: ")?.1;
                let verb = after.split(',').next().unwrap().trim();
                let path = line
                    .split("encodePathSegment(\"")
                    .nth(1)?
                    .split('"')
                    .next()
                    .unwrap();
                Some((path.to_owned(), verb.to_owned()))
            })
            .collect::<BTreeMap<_, _>>();
        assert_eq!(sent.len(), methods.len(), "{client}");
        for method in methods {
            let name = method.to_lowercase();
            assert_eq!(sent[&name], format!("\"{method}\""), "{client}");
        }
    }
}
