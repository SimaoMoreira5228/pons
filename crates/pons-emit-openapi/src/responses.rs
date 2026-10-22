use pons_ir::{
    ContentKind, ErrorContent, ErrorStatus, MediaDescriptor, Operation, Presence, ResponseHeader,
    ResponseStatus, StreamKind, TypeRef, is_valid_media_type,
};
use serde_json::{Map, Value, json};

use crate::operations::operation_id;
use crate::schemas::{Place, WireForm};
use crate::{Builder, EmitError};

impl Builder<'_> {
    pub(super) fn responses(&self, operation: &Operation) -> Result<Value, EmitError> {
        if let Some(StreamKind::Websocket { client, server }) =
            operation.stream.as_ref().map(|stream| &stream.kind)
        {
            return self.websocket_responses(operation, client, server);
        }
        let mut responses = Map::new();
        for response in &operation.responses {
            let media = media_type(
                response.content,
                operation.stream.as_ref(),
                response.media.as_ref(),
            )?;
            let mut value = json!({"description":"Response"});
            match response.content {
                ContentKind::Empty => {}
                ContentKind::Stream => {
                    let media = media.ok_or_else(|| {
                        EmitError("stream response is missing its stream description".into())
                    })?;
                    value["content"] = media_content(media, self.stream_schema(operation)?);
                }
                ContentKind::Bytes => {
                    let media = media.ok_or_else(|| {
                        EmitError("byte response is missing its media type".into())
                    })?;
                    value["content"] = media_content(media, json!({}));
                }
                _ => {
                    let body = response.body.as_ref().ok_or_else(|| {
                        EmitError(format!(
                            "response {} content {:?} is missing its body",
                            response.status, response.content
                        ))
                    })?;
                    let media = media.ok_or_else(|| {
                        EmitError(format!(
                            "response {} content {:?} has no media type",
                            response.status, response.content
                        ))
                    })?;
                    value["content"] = media_content(media, self.type_schema(body)?);
                }
            }
            add_response_headers(self, &response.headers, &mut value, "response")?;
            value["x-pons"] = json!({"status":response.status,"body":response.body,"content":response.content,"media":response.media,"ext":response.ext});
            responses.insert(success_status(response.status), value);
        }
        self.add_errors(operation, &mut responses)?;
        if responses.is_empty() {
            return Err(EmitError(format!(
                "the operation {} declares no responses and no errors, so every status the client sees would be a protocol error",
                operation_id(operation)
            )));
        }
        Ok(Value::Object(responses))
    }

    fn websocket_responses(
        &self,
        operation: &Operation,
        client: &TypeRef,
        server: &TypeRef,
    ) -> Result<Value, EmitError> {
        if operation.responses.len() > 1 {
            return Err(EmitError(format!(
                "the websocket operation {} declares {} handshake responses, but an upgrade has one status",
                operation_id(operation),
                operation.responses.len()
            )));
        }
        let declared = operation.responses.first();
        let mut value = json!({"description":"Response"});
        if let Some(response) = declared {
            add_response_headers(self, &response.headers, &mut value, "response")?;
        }
        let ext = declared.map(|response| &response.ext);
        value["x-pons"] = json!({
            "status": 101,
            "body": null,
            "content": ContentKind::Empty,
            "media": null,
            "ext": ext,
            "websocket": {
                "client": self.type_schema(client)?,
                "server": self.type_schema(server)?,
            },
        });
        let mut responses = Map::new();
        responses.insert("101".into(), value);
        self.add_errors(operation, &mut responses)?;
        Ok(Value::Object(responses))
    }

    fn add_errors(
        &self,
        operation: &Operation,
        responses: &mut Map<String, Value>,
    ) -> Result<(), EmitError> {
        let errors = if operation.errors_inherit {
            operation
                .errors
                .iter()
                .chain(&self.document.common_errors)
                .collect::<Vec<_>>()
        } else {
            operation.errors.iter().collect::<Vec<_>>()
        };
        for error in errors {
            let status = error_status(&error.status);
            let description = error
                .doc
                .as_ref()
                .and_then(|doc| doc.description.as_deref().or(doc.summary.as_deref()))
                .unwrap_or("Error response");
            let mut value = json!({"description":description});
            if error.content != ErrorContent::Empty {
                let body = error.body.as_ref().ok_or_else(|| {
                    EmitError(format!("error {:?} is missing its body", error.name))
                })?;
                let media = error
                    .media
                    .as_ref()
                    .map(ToString::to_string)
                    .unwrap_or_else(|| error_media_type(error.content).into());
                value["content"] = media_content(media, self.type_schema(body)?);
            }
            add_response_headers(self, &error.headers, &mut value, "error response")?;
            if let Some(retryable) = error.retryable {
                value["x-pons-retryable"] = json!(retryable);
            }
            value["x-pons"] = json!({"name":error.name,"body":error.body,"headers":error.headers,"content":error.content,"media":error.media,"doc":error.doc,"ext":error.ext});
            responses.insert(status, value);
        }
        Ok(())
    }

    fn stream_schema(&self, operation: &Operation) -> Result<Value, EmitError> {
        let Some(stream) = &operation.stream else {
            return Err(EmitError(
                "stream response is missing its stream description".into(),
            ));
        };
        let mut schema = match &stream.kind {
            StreamKind::Sse { events } => {
                let events = events
                    .iter()
                    .map(|event| {
                        Ok(json!({"event":event.event,"schema":self.type_schema(&event.data)?}))
                    })
                    .collect::<Result<Vec<_>, EmitError>>()?;
                json!({"type":"string","x-pons-events":events})
            }
            StreamKind::Ndjson { item } => self.type_schema(item)?,
            StreamKind::Bytes { .. } | StreamKind::Websocket { .. } => json!({}),
        };
        schema["x-pons-stream"] = json!(stream.kind);
        Ok(schema)
    }
}

pub(super) fn media_type(
    content: ContentKind,
    stream: Option<&pons_ir::StreamDescription>,
    override_media: Option<&MediaDescriptor>,
) -> Result<Option<String>, EmitError> {
    if let Some(StreamKind::Bytes { content_type }) = stream.map(|stream| &stream.kind)
        && !is_valid_media_type(content_type)
    {
        return Err(EmitError(format!(
            "invalid byte stream content type {content_type:?}"
        )));
    }
    if content == ContentKind::Stream
        && let Some(media) = override_media
        && let Some(expected) = stream.and_then(|stream| stream.kind.media_type())
        && !pons_ir::media_type_matches(&media.to_string(), expected)
    {
        return Err(EmitError(format!(
            "response media type {media} does not match stream kind media type {expected:?}"
        )));
    }
    if let Some(media) = override_media {
        media
            .validate()
            .map_err(|message| EmitError(message.to_string()))?;
        return Ok(Some(media.to_string()));
    }
    Ok(Some(match content {
        ContentKind::Json => "application/json".into(),
        ContentKind::ProblemJson => "application/problem+json".into(),
        ContentKind::Form => "application/x-www-form-urlencoded".into(),
        ContentKind::Multipart => "multipart/form-data".into(),
        ContentKind::Text => "text/plain".into(),
        ContentKind::Bytes => "application/octet-stream".into(),
        ContentKind::Stream => {
            let Some(kind) = stream.map(|stream| &stream.kind) else {
                return Err(EmitError(
                    "stream response is missing its stream description".into(),
                ));
            };
            let Some(content_type) = kind.media_type() else {
                return Ok(None);
            };
            content_type.into()
        }
        ContentKind::Empty => return Ok(None),
    }))
}

fn error_media_type(content: ErrorContent) -> &'static str {
    match content {
        ErrorContent::Json => "application/json",
        ErrorContent::ProblemJson => "application/problem+json",
        ErrorContent::Text => "text/plain",
        ErrorContent::Empty => unreachable!(),
    }
}

pub(super) fn media_content(media: String, schema: Value) -> Value {
    let mut content = Map::new();
    content.insert(media, json!({"schema":schema}));
    Value::Object(content)
}

fn add_response_headers(
    builder: &Builder<'_>,
    headers: &[ResponseHeader],
    value: &mut Value,
    kind: &str,
) -> Result<(), EmitError> {
    if headers.is_empty() {
        return Ok(());
    }
    let mut response_headers = Map::new();
    for header in headers {
        if header.wire.eq_ignore_ascii_case("content-type") {
            return Err(EmitError(format!(
                "{kind} header {} is defined by the response content",
                header.wire
            )));
        }
        let schema = builder.type_schema(&header.ty)?;
        let mut entry = json!({
            "required": header.presence == Presence::Required,
            "x-pons": {"presence": header.presence, "ext": header.ext}
        });

        match builder.wire_form(&header.ty, Place::Header)? {
            Some(WireForm::Json) => {
                entry["content"] = media_content("application/json".into(), schema);
            }
            Some(WireForm::Text) => {
                entry["schema"] = schema;
            }
            None => {
                return Err(EmitError(format!(
                    "{kind} header {} has no header encoding this emitter can state, because the client reads it in a shape no keyword names",
                    header.wire
                )));
            }
        }
        response_headers.insert(header.wire.clone(), entry);
    }
    value["headers"] = Value::Object(response_headers);
    Ok(())
}

fn success_status(status: ResponseStatus) -> String {
    match status {
        ResponseStatus::Exact(status) => status.to_string(),
        ResponseStatus::Class(class) => format!("{class}XX"),
        ResponseStatus::Default | ResponseStatus::Unknown => "default".into(),
    }
}

fn error_status(status: &ErrorStatus) -> String {
    match status {
        ErrorStatus::Exact(status) => status.to_string(),
        ErrorStatus::Class(class) => format!("{class}XX"),
        ErrorStatus::Default => "default".into(),
    }
}

#[cfg(test)]
mod tests {
    use pons_ir::{ContentKind, InlineType, Response, ResponseStatus, StreamKind, TypeRef};
    use serde_json::{Value, json};

    use crate::EmitMode;

    fn fixture() -> pons_ir::Document {
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap()
    }

    fn string() -> TypeRef {
        TypeRef::Inline(InlineType::String {
            format: None,
            min_length: None,
            max_length: None,
            pattern: None,
        })
    }

    fn named(id: &str) -> TypeRef {
        TypeRef::Named(pons_ir::NamedRef {
            id: id.into(),
            variant: None,
        })
    }

    fn websocket_document(response: Response) -> pons_ir::Document {
        let mut document = fixture();
        let operation = document
            .operations
            .iter_mut()
            .find(|operation| operation.stream.is_some())
            .unwrap();
        operation.stream = Some(pons_ir::StreamDescription {
            kind: StreamKind::Websocket {
                client: named("LoginRequest"),
                server: named("User"),
            },
        });
        operation.responses = vec![response];
        document
    }

    fn handshake() -> Response {
        Response {
            status: ResponseStatus::Exact(101),
            content: ContentKind::Empty,
            media: None,
            body: None,
            headers: Vec::new(),
            ext: Default::default(),
        }
    }

    #[test]
    fn a_websocket_operation_emits_a_bodyless_upgrade_and_both_message_schemas() {
        let document = websocket_document(handshake());
        let emitted = crate::emit_with_mode(&document, EmitMode::Strict).unwrap();
        let output: Value = serde_json::from_str(&emitted.content).unwrap();
        let operation = &output["paths"]["/events/{cursor}"]["get"];

        assert_eq!(operation["operationId"], "events.watch");
        assert_eq!(operation["x-pons"]["stream"]["kind"], "websocket");
        assert_eq!(
            operation["x-pons"]["stream"]["client"]["ref"],
            "LoginRequest"
        );
        assert_eq!(operation["x-pons"]["stream"]["server"]["ref"], "User");

        let responses = operation["responses"].as_object().unwrap();
        assert_eq!(responses.keys().collect::<Vec<_>>(), ["101"]);
        let upgrade = &responses["101"];
        assert!(upgrade.get("content").is_none(), "{upgrade:#}");
        assert_eq!(upgrade["x-pons"]["status"], 101);
        assert_eq!(upgrade["x-pons"]["content"], "empty");
        assert!(upgrade["x-pons"]["body"].is_null());
        assert!(upgrade["x-pons"]["media_type"].is_null());
        assert_eq!(
            upgrade["x-pons"]["websocket"]["client"]["$ref"],
            "#/components/schemas/LoginRequest"
        );
        assert_eq!(
            upgrade["x-pons"]["websocket"]["server"]["$ref"],
            "#/components/schemas/User"
        );

        assert!(emitted.coverage.opaque.contains("operations.events.watch"));
        assert!(!emitted.coverage.exact.contains("operations.events.watch"));
        assert!(!emitted.coverage.lossy.contains("operations.events.watch"));
    }

    #[test]
    fn a_websocket_upgrade_keeps_declared_headers_and_errors() {
        let mut response = handshake();
        response.headers.push(pons_ir::ResponseHeader {
            wire: "Sec-WebSocket-Protocol".into(),
            ty: string(),
            presence: pons_ir::Presence::Optional,
            ext: Default::default(),
        });
        let mut document = websocket_document(response);
        document.operations[1].errors.push(pons_ir::ErrorResponse {
            status: pons_ir::ErrorStatus::Exact(403),
            name: vec!["forbidden".into()],
            content: pons_ir::ErrorContent::Json,
            media: None,
            body: Some(string()),
            headers: Vec::new(),
            retryable: None,
            doc: None,
            ext: Default::default(),
        });
        let output: Value = serde_json::from_str(&crate::emit(&document).unwrap()).unwrap();
        let responses = &output["paths"]["/events/{cursor}"]["get"]["responses"];
        assert_eq!(
            responses["101"]["headers"]["Sec-WebSocket-Protocol"]["schema"]["type"],
            "string"
        );
        assert_eq!(
            responses["403"]["content"]["application/json"]["schema"]["type"],
            "string"
        );
    }

    #[test]
    fn a_websocket_stream_response_still_emits_the_upgrade() {
        let mut response = handshake();
        response.status = ResponseStatus::Exact(200);
        response.content = ContentKind::Stream;
        let document = websocket_document(response);
        let output: Value = serde_json::from_str(&crate::emit(&document).unwrap()).unwrap();
        let responses = &output["paths"]["/events/{cursor}"]["get"]["responses"];
        assert!(responses["101"].is_object(), "{responses:#}");
        assert!(responses.get("200").is_none(), "{responses:#}");
        assert!(responses["101"].get("content").is_none());
    }

    #[test]
    fn rejecting_two_websocket_handshake_responses_names_the_operation() {
        let mut document = websocket_document(handshake());
        let mut extra = handshake();
        extra.status = ResponseStatus::Exact(200);
        document.operations[1].responses.push(extra);
        let error = crate::emit(&document).unwrap_err().0;
        assert!(error.contains("events.watch"), "{error}");
        assert!(error.contains("handshake responses"), "{error}");
    }

    #[test]
    fn a_websocket_operation_produces_stable_canonical_json() {
        let document = websocket_document(handshake());
        let encoded = crate::emit(&document).unwrap();
        assert!(encoded.ends_with('\n'));
        assert!(encoded.contains("\n  \"paths\": {"), "{encoded}");
        assert_eq!(encoded, crate::emit(&document.clone()).unwrap());

        let mut reversed = document;
        reversed.operations.reverse();
        assert_eq!(encoded, crate::emit(&reversed).unwrap());
    }

    #[test]
    fn other_stream_kinds_and_ordinary_operations_still_emit() {
        let document = fixture();
        let output: Value = serde_json::from_str(&crate::emit(&document).unwrap()).unwrap();
        assert_eq!(
            output["paths"]["/auth/login"]["post"]["responses"]["200"]["content"]["application/json"]
                ["schema"]["$ref"],
            "#/components/schemas/User"
        );
        assert!(
            output["paths"]["/events/{cursor}"]["get"]["responses"]["200"]["content"]
                ["text/event-stream"]
                .is_object()
        );

        let mut ndjson = fixture();
        ndjson.operations[1].stream.as_mut().unwrap().kind = StreamKind::Ndjson { item: string() };
        let output: Value = serde_json::from_str(&crate::emit(&ndjson).unwrap()).unwrap();
        let content = &output["paths"]["/events/{cursor}"]["get"]["responses"]["200"]["content"];
        assert_eq!(content["application/x-ndjson"]["schema"]["type"], "string");

        let mut bytes = fixture();
        bytes.operations[1].stream.as_mut().unwrap().kind = StreamKind::Bytes {
            content_type: "application/pdf".into(),
        };
        let output: Value = serde_json::from_str(&crate::emit(&bytes).unwrap()).unwrap();
        let content = &output["paths"]["/events/{cursor}"]["get"]["responses"]["200"]["content"];
        assert_eq!(
            content["application/pdf"]["schema"],
            json!({"x-pons-stream":{"kind":"bytes","content_type":"application/pdf"}})
        );
    }
}
