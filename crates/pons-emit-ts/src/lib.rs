mod client;
mod runtime;
mod types;

use std::collections::BTreeMap;

use pons_ir::{Document, StreamKind};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedFile {
    pub path: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmitError(pub String);

impl std::fmt::Display for EmitError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for EmitError {}

pub fn emit(document: &Document) -> Result<Vec<GeneratedFile>, EmitError> {
    let mut document = document.clone();
    document.canonicalize();
    document.validate().map_err(|errors| {
        EmitError(
            errors
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("\n"),
        )
    })?;
    if document.operations.iter().any(|operation| {
        operation
            .stream
            .as_ref()
            .is_some_and(|stream| !matches!(&stream.kind, StreamKind::Sse { .. }))
            || operation.stream.is_none()
                && operation
                    .responses
                    .iter()
                    .any(|response| response.content == pons_ir::ContentKind::Stream)
    }) {
        return Err(EmitError(
            "the TypeScript emitter currently supports SSE streams only".into(),
        ));
    }
    let names = type_names(&document);
    Ok(vec![
        GeneratedFile {
            path: "types.ts".into(),
            content: types::emit(&document, &names)?,
        },
        GeneratedFile {
            path: "client.ts".into(),
            content: client::emit(&document, &names)?,
        },
    ])
}

fn type_names(document: &Document) -> BTreeMap<String, String> {
    let mut result = BTreeMap::new();
    let mut used = BTreeMap::<String, usize>::new();
    for (id, definition) in &document.types {
        let base = pascal(&definition.name);
        let count = used.entry(base.clone()).or_default();
        *count += 1;
        let name = if *count == 1 {
            base
        } else {
            format!("{base}{count}")
        };
        result.insert(id.clone(), name);
    }
    result
}

fn pascal(words: &[String]) -> String {
    let value = words
        .iter()
        .map(|word| {
            let mut chars = word.chars();
            chars
                .next()
                .map(|first| first.to_uppercase().chain(chars).collect::<String>())
                .unwrap_or_default()
        })
        .collect::<String>();
    identifier(&value, "Type")
}

fn camel(words: &[String]) -> String {
    let Some((first, rest)) = words.split_first() else {
        return "operation".into();
    };
    let mut result = first.to_lowercase();
    for word in rest {
        let mut chars = word.chars();
        if let Some(first) = chars.next() {
            result.extend(first.to_uppercase());
            result.extend(chars);
        }
    }
    identifier(&result, "operation")
}

fn identifier(value: &str, fallback: &str) -> String {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return fallback.into();
    };
    let mut result = String::new();
    if first.is_ascii_alphabetic() || first == '_' || first == '$' {
        result.push(first);
    } else {
        result.push('_');
        result.push(first);
    }
    result.extend(chars.map(|character| {
        if character.is_ascii_alphanumeric() || character == '_' || character == '$' {
            character
        } else {
            '_'
        }
    }));
    if is_keyword(&result) {
        format!("{result}_")
    } else {
        result
    }
}

fn is_keyword(value: &str) -> bool {
    matches!(
        value,
        "break"
            | "case"
            | "catch"
            | "class"
            | "const"
            | "continue"
            | "debugger"
            | "default"
            | "delete"
            | "do"
            | "else"
            | "enum"
            | "export"
            | "extends"
            | "false"
            | "finally"
            | "for"
            | "function"
            | "if"
            | "import"
            | "in"
            | "instanceof"
            | "new"
            | "null"
            | "return"
            | "super"
            | "switch"
            | "this"
            | "throw"
            | "true"
            | "try"
            | "typeof"
            | "var"
            | "void"
            | "while"
            | "with"
            | "as"
            | "implements"
            | "interface"
            | "let"
            | "package"
            | "private"
            | "protected"
            | "public"
            | "static"
            | "yield"
            | "any"
            | "boolean"
            | "number"
            | "string"
            | "symbol"
            | "type"
            | "from"
            | "of"
    )
}

pub(crate) fn string(value: &str) -> String {
    serde_json::to_string(value).expect("serializing a string cannot fail")
}

#[cfg(test)]
mod tests {
    use pons_ir::{Document, IR_VERSION, StreamKind};
    use std::collections::BTreeMap;

    #[test]
    fn emits_a_client_for_the_contract_fixture() {
        let document: Document =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        let files = super::emit(&document).unwrap();
        let files = files
            .into_iter()
            .map(|file| (file.path, file.content))
            .collect::<BTreeMap<_, _>>();
        let types = &files["types.ts"];
        let client = &files["client.ts"];
        assert!(types.contains("export type User"));
        assert!(types.contains("export type LoginError"));
        assert!(client.contains("export function createClient"));
        assert!(client.contains("auth"));
        assert!(client.contains("neverthrow"));
        assert!(client.contains("kind: \"api_key\", location: \"header\""));
        assert!(client.contains("body: Extract<Types.LoginError"));
        assert!(client.contains("client.auth![provides.scheme]"));
        assert!(client.contains("AsyncIterable<Result<EventsWatchEvent, EventsWatchStreamError>>"));
        assert!(client.contains("event: \"user\""));
        assert!(client.contains("name === \"error\""));
        assert!(client.contains("headers: [{ name: \"X-Trace-ID\", required: true }]"));
        assert!(client.contains("missing required response header"));
        assert!(client.contains("expectedContentTypes"));
        assert!(client.contains("application/problem+json"));
        assert!(client.contains("stream: \"text/event-stream\""));
        assert!(client.contains("actualContentType !== expectedContentType"));
        assert!(!client.contains("responseContent.includes(\"text/event-stream\")"));
        assert!(client.contains("response.headers.has(header.name)"));
        assert!(client.contains("header.required && !response.headers.has(header.name)"));
        assert!(client.contains("declaredSuccess?.headers"));
        let missing_header_branch = client
            .split("const missingHeader")
            .nth(1)
            .unwrap()
            .split("const responseSchema")
            .next()
            .unwrap();
        assert!(missing_header_branch.contains("if (missingHeader)"));
        assert!(
            missing_header_branch.contains("header.required && !response.headers.has(header.name)")
        );
        assert!(missing_header_branch.contains("kind: \"protocol\""));
        assert!(client.contains("undeclared success response"));
        assert!(client.contains("Math.floor(response.status / 100)"));
        assert!(!client.contains("response.ok"));
        let declared_success = client.find("const declaredSuccess").unwrap();
        let declared_error = client.find("const declaredError").unwrap();
        let undeclared_success = client.find("undeclared success response").unwrap();
        assert!(declared_success < declared_error && declared_error < undeclared_success);
        let stream_capture = client
            .find("captureCredential(client, response, provides, undefined, false)")
            .unwrap();
        let stream_return = client.find("return ok(readSse").unwrap();
        assert!(stream_capture < stream_return);
    }

    #[test]
    fn emits_optional_response_headers() {
        let mut document: Document =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        document.operations[1].responses[0].headers[0].presence = pons_ir::Presence::Optional;
        let files = super::emit(&document).unwrap();
        let client = files.iter().find(|file| file.path == "client.ts").unwrap();
        assert!(
            client
                .content
                .contains("headers: [{ name: \"X-Trace-ID\", required: false }]")
        );
    }

    #[test]
    fn types_sse_error_events_as_stream_failures() {
        let mut document: Document =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        let operation = document
            .operations
            .iter_mut()
            .find(|operation| operation.stream.is_some())
            .unwrap();
        let StreamKind::Sse { events } = &mut operation.stream.as_mut().unwrap().kind else {
            unreachable!()
        };
        events.push(pons_ir::StreamMessage {
            event: "error".into(),
            data: pons_ir::TypeRef::Inline(pons_ir::InlineType::String {
                format: None,
                min_length: None,
                max_length: None,
                pattern: None,
            }),
        });

        let files = super::emit(&document).unwrap();
        let client = files.iter().find(|file| file.path == "client.ts").unwrap();
        assert!(
            client
                .content
                .contains("kind: \"server_error\"; body: string")
        );
        assert!(!client.content.contains("event: \"error\"; data:"));
    }

    #[test]
    fn emission_is_independent_of_operation_order() {
        let mut document: Document =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        let expected = super::emit(&document).unwrap();
        document.operations.reverse();
        assert_eq!(super::emit(&document).unwrap(), expected);
    }

    #[test]
    fn rejects_documents_with_an_unsupported_ir_version() {
        let mut document: Document =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        document.ir_version = "9.0".into();
        assert!(super::emit(&document).is_err());
        assert_eq!(document.ir_version, "9.0");
        assert_eq!(IR_VERSION, "0.1");
    }

    #[test]
    fn rejects_unsupported_streams_instead_of_emitting_a_false_contract() {
        let document: Document =
            serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap();
        let mut document = document;
        let operation = document
            .operations
            .iter_mut()
            .find(|operation| operation.stream.is_some())
            .unwrap();
        operation.stream.as_mut().unwrap().kind = StreamKind::Ndjson {
            item: pons_ir::TypeRef::Inline(pons_ir::InlineType::String {
                format: None,
                min_length: None,
                max_length: None,
                pattern: None,
            }),
        };
        let error = super::emit(&document).unwrap_err();
        assert!(error.0.contains("SSE streams only"));
    }
}
