use std::collections::BTreeMap;

use pons_ir::{ContentKind, Operation, PathSegment, Presence, TypeRef};
use serde_json::{Map, Value, json};

use crate::responses::{media_content, media_type};
use crate::schemas::{Place, WireForm};
use crate::{Builder, EmitError};

impl Builder<'_> {
    pub(super) fn paths(&self) -> Result<Value, EmitError> {
        let mut paths = BTreeMap::<String, Map<String, Value>>::new();
        let mut operation_ids = std::collections::BTreeSet::new();
        let mut routes = BTreeMap::<String, String>::new();
        for operation in &self.document.operations {
            let path = openapi_path(operation)?;

            let method = operation.method.as_str().to_ascii_lowercase();
            let id = operation_id(operation);
            if !operation_ids.insert(id.clone()) {
                return Err(EmitError(format!("duplicate OpenAPI operationId {id}")));
            }
            let route = route_pattern(operation);
            if let Some(existing) = routes.get(&route) {
                if existing != &path {
                    return Err(EmitError(format!(
                        "OpenAPI route pattern {route} uses conflicting parameter names"
                    )));
                }
            } else {
                routes.insert(route, path.clone());
            }
            let item = paths.entry(path.clone()).or_default();
            if item.contains_key(method.as_str()) {
                return Err(EmitError(format!(
                    "multiple operations use {method} {path}"
                )));
            }
            item.insert(method, self.operation(operation)?);
        }
        Ok(Value::Object(
            paths
                .into_iter()
                .map(|(path, item)| (path, Value::Object(item)))
                .collect(),
        ))
    }

    fn operation(&self, operation: &Operation) -> Result<Value, EmitError> {
        let mut value = Map::new();
        value.insert("operationId".into(), json!(operation_id(operation)));
        let mut parameters = self.parameters(operation)?;
        if !parameters.is_empty() {
            value.insert(
                "parameters".into(),
                Value::Array(std::mem::take(&mut parameters)),
            );
        }
        if let Some(body) = &operation.body {
            value.insert("requestBody".into(), self.request_body(body)?);
        }
        value.insert("responses".into(), self.responses(operation)?);
        if let Some(doc) = &operation.doc {
            if let Some(summary) = &doc.summary {
                value.insert("summary".into(), json!(summary));
            }
            if let Some(description) = &doc.description {
                value.insert("description".into(), json!(description));
            }
            if let Some(deprecated) = doc.deprecated {
                value.insert("deprecated".into(), json!(deprecated));
            }
            if !doc.tags.is_empty() {
                value.insert("tags".into(), json!(doc.tags));
            }
        }
        value.insert(
            "security".into(),
            Value::Array(self.operation_security(operation)?),
        );
        let metadata = json!({
            "id": operation.id,
            "auth": operation.auth,
            "semantics": operation.semantics,
            "provides": operation.provides,
            "errors_inherit": operation.errors_inherit,
            "stream": operation.stream,
            "origin": operation.origin,
            "ext": operation.ext,
            "diagnostics": operation.diagnostics,
        });
        value.insert("x-pons".into(), metadata);
        Ok(Value::Object(value))
    }

    fn parameters(&self, operation: &Operation) -> Result<Vec<Value>, EmitError> {
        let mut parameters = Vec::new();
        for segment in &operation.path {
            if let PathSegment::Param { param, ty } = segment {
                parameters.push(json!({"name":param,"in":"path","required":true,"schema":self.type_schema(ty)?}));
            }
        }
        if let Some(query) = &operation.query {
            self.query_parameters(query, &mut parameters)?;
        }
        for header in &operation.headers {
            parameters.push(json!({
                "name": header.wire,
                "in": "header",
                "required": header.presence == Presence::Required,
                "schema": self.type_schema(&header.ty)?,
                "x-pons":{"ident":header.ident,"presence":header.presence,"ext":header.ext},
            }));
        }
        Ok(parameters)
    }

    fn query_parameters(
        &self,
        query: &TypeRef,
        parameters: &mut Vec<Value>,
    ) -> Result<(), EmitError> {
        for field in pons_ir::query_fields(self.document, query)
            .map_err(|rejection| EmitError(format!("OpenAPI {}", rejection.message())))?
        {
            let mut parameter = json!({
                "name": field.wire,
                "in": "query",
                "required": field.presence == Presence::Required,
                "schema": self.field_schema(field)?,
            });
            add_parameter_doc(&mut parameter, field.doc.as_ref());
            parameters.push(parameter);
        }
        Ok(())
    }

    fn request_body(&self, body: &pons_ir::Body) -> Result<Value, EmitError> {
        let media = media_type(body.content, None, body.media_type.as_deref())?
            .ok_or_else(|| EmitError("request body cannot use empty content".into()))?;
        let schema = if body.content == ContentKind::Bytes {
            json!({})
        } else {
            self.type_schema(&body.ty)?
        };
        let content = match body.content {
            ContentKind::Form | ContentKind::Multipart => {
                let mut entry = Map::new();
                entry.insert("schema".into(), schema);
                entry.insert(
                    "encoding".into(),
                    self.form_encoding(&body.ty, body.content)?,
                );
                let mut content = Map::new();
                content.insert(media, Value::Object(entry));
                Value::Object(content)
            }
            _ => media_content(media, schema),
        };
        Ok(json!({
            "required":body.required,
            "content":content,
            "x-pons":{"ty":body.ty,"required":body.required,"content":body.content,"media_type":body.media_type,"ext":body.ext}
        }))
    }

    fn form_encoding(&self, ty: &TypeRef, content: ContentKind) -> Result<Value, EmitError> {
        let Some((fields, _)) = self.object_fields(ty, &mut Vec::new())? else {
            return Err(EmitError(
                "a form or multipart body must be a struct, because the client writes one value per key and OpenAPI states an encoding per property".into(),
            ));
        };
        let mut encoding = Map::new();
        for field in fields {
            if encoding.contains_key(&field.wire) {
                return Err(EmitError(format!(
                    "the form or multipart field {:?} is declared more than once, because the client writes one value per key",
                    field.wire
                )));
            }
            let form = self
                .wire_form(&field.ty, Place::Part {
                    required: field.presence == Presence::Required,
                })?
                .ok_or_else(|| {
                    EmitError(format!(
                        "the form or multipart field {:?} has no encoding this emitter can state, because the client writes no single value the document can name",
                        field.wire
                    ))
                })?;
            if content == ContentKind::Multipart && form == WireForm::Json {
                return Err(EmitError(format!(
                    "the multipart field {:?} has no part media type this emitter can state, because the client writes the JSON of it as a text/plain string part",
                    field.wire
                )));
            }
            encoding.insert(field.wire.clone(), json!({"contentType":"text/plain"}));
        }
        Ok(Value::Object(encoding))
    }
}

fn openapi_path(operation: &Operation) -> Result<String, EmitError> {
    let mut segments = Vec::with_capacity(operation.path.len());
    for segment in &operation.path {
        match segment {
            PathSegment::Literal { lit } if lit.contains('{') || lit.contains('}') => {
                return Err(EmitError(format!(
                    "path literal {lit:?} contains OpenAPI template syntax"
                )));
            }
            PathSegment::Literal { lit } if is_dot_segment(lit) => {
                return Err(EmitError(format!(
                    "path literal {lit:?} is a dot segment the TypeScript client refuses to encode"
                )));
            }
            PathSegment::Literal { lit } => segments.push(escape_path_literal(lit)),
            PathSegment::Param { param, .. } if !valid_path_parameter_name(param) => {
                return Err(EmitError(format!(
                    "path parameter {param:?} is not a valid OpenAPI path parameter name"
                )));
            }
            PathSegment::Param { param, .. } => segments.push(format!("{{{param}}}")),
        }
    }
    if segments.is_empty() {
        Ok("/".into())
    } else {
        Ok(format!("/{}", segments.join("/")))
    }
}

fn route_pattern(operation: &Operation) -> String {
    if operation.path.is_empty() {
        return "/".into();
    }
    format!(
        "/{}",
        operation
            .path
            .iter()
            .map(|segment| match segment {
                PathSegment::Literal { lit } => escape_path_literal(lit),
                PathSegment::Param { .. } => "{}".into(),
            })
            .collect::<Vec<_>>()
            .join("/")
    )
}

fn escape_path_literal(literal: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut escaped = String::with_capacity(literal.len());
    for byte in literal.bytes() {
        if byte.is_ascii_alphanumeric()
            || matches!(
                byte,
                b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')'
            )
        {
            escaped.push(char::from(byte));
        } else {
            escaped.push('%');
            escaped.push(char::from(HEX[usize::from(byte >> 4)]));
            escaped.push(char::from(HEX[usize::from(byte & 0x0F)]));
        }
    }
    escaped
}

fn is_dot_segment(literal: &str) -> bool {
    matches!(literal, "." | "..")
}

fn valid_path_parameter_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~'))
}

fn add_parameter_doc(parameter: &mut Value, doc: Option<&pons_ir::Doc>) {
    let Some(doc) = doc else { return };
    if let Some(description) = doc.description.as_deref().or(doc.summary.as_deref()) {
        parameter["description"] = json!(description);
    }
    if let Some(deprecated) = doc.deprecated {
        parameter["deprecated"] = json!(deprecated);
    }
}

pub(super) fn operation_id(operation: &Operation) -> String {
    operation
        .id
        .iter()
        .map(|part| part.join("_"))
        .collect::<Vec<_>>()
        .join(".")
}

#[cfg(test)]
mod tests {
    use pons_ir::{ContentKind, PathSegment};
    use serde_json::{Value, json};

    fn fixture() -> pons_ir::Document {
        serde_json::from_str(include_str!("../../../fixtures/login.json")).unwrap()
    }

    fn with_body(types: Value, ty: Value, content: ContentKind) -> pons_ir::Document {
        let mut raw = serde_json::to_value(fixture()).unwrap();
        for (name, definition) in types.as_object().unwrap() {
            raw["types"][name.as_str()] = definition.clone();
        }
        raw["operations"][0]["body"] = json!({"ty":ty,"content":content,"required":true});
        serde_json::from_value(raw).unwrap()
    }

    fn body_encoding(document: &pons_ir::Document, media: &str) -> Value {
        let output: Value = serde_json::from_str(&crate::emit(document).unwrap()).unwrap();
        output["paths"]["/auth/login"]["post"]["requestBody"]["content"][media]["encoding"].clone()
    }

    #[test]
    fn states_the_one_value_per_key_encoding_the_client_writes() {
        let types = json!({
            "Seal": {
                "kind": "struct", "name": ["seal"], "direction": "both",
                "openness": "reject",
                "fields": [
                    {"wire": "kind", "ident": ["kind"], "ty": {"kind": "string"}, "presence": "required"},
                    {"wire": "blob", "ident": ["blob"], "ty": {"kind": "bytes", "encoding": "base64url"}, "presence": "optional"}
                ]
            },
            "Upload": {
                "kind": "struct", "name": ["upload"], "direction": "both",
                "openness": "reject",
                "flattened": [{"ref": "Seal"}],
                "fields": [
                    {"wire": "title", "ident": ["title"], "ty": {"kind": "string"}, "presence": "required"},
                    {"wire": "code", "ident": ["code"], "ty": {"kind": "int", "bits": 32, "signed": false, "wire": "string"}, "presence": "optional"}
                ]
            }
        });
        let multipart = with_body(
            types.clone(),
            json!({"ref": "Upload"}),
            ContentKind::Multipart,
        );
        let encoding = body_encoding(&multipart, "multipart/form-data");
        assert_eq!(
            encoding,
            json!({
                "title": {"contentType": "text/plain"},
                "code": {"contentType": "text/plain"},
                "kind": {"contentType": "text/plain"},
                "blob": {"contentType": "text/plain"}
            }),
            "every part is a text/plain string the client writes through FormData.set"
        );

        let flat = json!({
            "Signup": {
                "kind": "struct", "name": ["signup"], "direction": "both",
                "openness": "reject",
                "fields": [
                    {"wire": "email", "ident": ["email"], "ty": {"kind": "string"}, "presence": "required"},
                    {"wire": "tags", "ident": ["tags"], "ty": {"kind": "array", "item": {"kind": "string"}}, "presence": "optional"}
                ]
            }
        });
        let form = with_body(flat, json!({"ref": "Signup"}), ContentKind::Form);
        let encoding = body_encoding(&form, "application/x-www-form-urlencoded");
        assert_eq!(
            encoding,
            json!({
                "email": {"contentType": "text/plain"},
                "tags": {"contentType": "text/plain"}
            }),
            "a urlencoded body carries every value as text, and its encoding object is advisory"
        );
    }

    #[test]
    fn refuses_a_multipart_field_the_client_writes_as_a_text_part() {
        let types = json!({
            "Upload": {
                "kind": "struct", "name": ["upload"], "direction": "both",
                "openness": "reject",
                "fields": [
                    {"wire": "title", "ident": ["title"], "ty": {"kind": "string"}, "presence": "required"},
                    {"wire": "tags", "ident": ["tags"], "ty": {"kind": "array", "item": {"kind": "string"}}, "presence": "optional"}
                ]
            }
        });
        let document = with_body(types, json!({"ref": "Upload"}), ContentKind::Multipart);
        let error = crate::emit(&document).unwrap_err().0;
        assert!(error.contains("\"tags\""), "{error}");
        assert!(
            error.contains("no part media type this emitter can state"),
            "{error}"
        );
    }

    #[test]
    fn refuses_a_form_body_the_encoding_cannot_describe() {
        let opaque = json!({
            "Signed": {
                "kind": "struct", "name": ["signed"], "direction": "both",
                "openness": "reject",
                "fields": [{
                    "wire": "seal", "ident": ["seal"],
                    "ty": {"kind": "opaque", "codec": "example.seal", "wire": "json"},
                    "presence": "required"
                }]
            }
        });
        let document = with_body(opaque, json!({"ref": "Signed"}), ContentKind::Multipart);
        let error = crate::emit(&document).unwrap_err().0;
        assert!(error.contains("\"seal\""), "{error}");
        assert!(
            error.contains("no encoding this emitter can state"),
            "{error}"
        );

        let required_nullable = json!({
            "Signed": {
                "kind": "struct", "name": ["signed"], "direction": "both",
                "openness": "reject",
                "fields": [{
                    "wire": "note", "ident": ["note"],
                    "ty": {"kind": "nullable", "inner": {"kind": "string"}},
                    "presence": "required"
                }]
            }
        });
        let document = with_body(
            required_nullable,
            json!({"ref": "Signed"}),
            ContentKind::Multipart,
        );
        let error = crate::emit(&document).unwrap_err().0;
        assert!(error.contains("\"note\""), "{error}");
        assert!(
            error.contains("no encoding this emitter can state"),
            "{error}"
        );

        for ty in [
            json!({"kind": "map", "key": {"kind": "string"}, "value": {"kind": "string"}}),
            json!({"kind": "array", "item": {"kind": "string"}}),
            json!({"kind": "string"}),
        ] {
            let document = with_body(json!({}), ty, ContentKind::Form);
            let error = crate::emit(&document).unwrap_err().0;
            assert!(error.contains("must be a struct"), "{error}");
        }
    }

    #[test]
    fn refuses_a_form_or_multipart_body_with_two_fields_on_one_wire_name() {
        let types = json!({
            "Seal": {
                "kind": "struct", "name": ["seal"], "direction": "both",
                "openness": "reject",
                "fields": [{
                    "wire": "kind", "ident": ["kind"],
                    "ty": {"kind": "string"}, "presence": "required"
                }]
            },
            "Upload": {
                "kind": "struct", "name": ["upload"], "direction": "both",
                "openness": "reject",
                "flattened": [{"ref": "Seal"}],
                "fields": [{
                    "wire": "kind", "ident": ["sealKind"],
                    "ty": {"kind": "int", "bits": 32, "signed": false, "wire": "string"},
                    "presence": "required"
                }]
            }
        });
        for content in [ContentKind::Form, ContentKind::Multipart] {
            let document = with_body(types.clone(), json!({"ref": "Upload"}), content);
            let error = crate::emit(&document).unwrap_err().0;
            assert!(error.contains("\"kind\""), "{error}");
            assert!(error.contains("is declared more than once"), "{error}");
        }
    }

    #[test]
    fn states_a_structured_response_header_as_json_content() {
        let string = || json!({"kind": "string"});
        let mut raw = serde_json::to_value(fixture()).unwrap();
        raw["operations"][1]["responses"][0]["headers"] = json!([
            {"wire": "X-Items", "ty": {"kind": "array", "item": string()}, "presence": "required"},
            {"wire": "X-Pair", "ty": {"kind": "tuple", "items": [string()]}, "presence": "required"},
            {"wire": "X-Trace-ID", "ty": string(), "presence": "required"}
        ]);
        let document: pons_ir::Document = serde_json::from_value(raw).unwrap();
        let output: Value = serde_json::from_str(&crate::emit(&document).unwrap()).unwrap();
        let headers = &output["paths"]["/events/{cursor}"]["get"]["responses"]["200"]["headers"];

        assert_eq!(
            headers["X-Items"]["content"]["application/json"]["schema"]["type"],
            "array"
        );
        assert_eq!(
            headers["X-Pair"]["content"]["application/json"]["schema"]["maxItems"],
            1
        );
        for wire in ["X-Items", "X-Pair"] {
            assert!(headers[wire].get("schema").is_none(), "{headers:#}");
            assert_eq!(headers[wire]["required"], true);
        }
        assert_eq!(headers["X-Trace-ID"]["schema"]["type"], "string");
        assert!(
            headers["X-Trace-ID"].get("content").is_none(),
            "{headers:#}"
        );
    }

    #[test]
    fn refuses_a_response_header_no_encoding_states() {
        for (ty, expected) in [
            (
                json!({"kind": "opaque", "codec": "example.seal", "wire": "bytes"}),
                "no header encoding this emitter can state",
            ),
            (
                json!({"ref": "Choice"}),
                "no header encoding this emitter can state",
            ),
        ] {
            let mut raw = serde_json::to_value(fixture()).unwrap();
            raw["types"]["Choice"] = json!({
                "kind": "enum", "name": ["choice"], "direction": "both",
                "repr": {"kind": "external"},
                "variants": [
                    {"wire": "quiet", "ident": ["quiet"], "payload": {"kind": "unit"}},
                    {
                        "wire": "loud", "ident": ["loud"],
                        "payload": {"kind": "newtype", "ty": {"kind": "map", "key": {"kind": "string"}, "value": {"kind": "string"}}}
                    }
                ]
            });
            raw["operations"][1]["responses"][0]["headers"] = json!([{
                "wire": "X-Choice", "ty": ty, "presence": "required"
            }]);
            let document: pons_ir::Document = serde_json::from_value(raw).unwrap();
            let error = crate::emit(&document).unwrap_err().0;
            assert!(error.contains("X-Choice"), "{error}");
            assert!(error.contains(expected), "{error}");
        }
    }

    #[test]
    fn refuses_an_operation_that_declares_no_response_at_all() {
        let mut document = fixture();
        let operation = document
            .operations
            .iter_mut()
            .find(|operation| operation.id[0][0] == "auth")
            .unwrap();
        operation.responses.clear();
        operation.errors.clear();
        let error = crate::emit(&document).unwrap_err().0;
        assert!(error.contains("auth.login"), "{error}");
        assert!(
            error.contains("declares no responses and no errors"),
            "{error}"
        );

        let mut document = fixture();
        document.common_errors = document.operations[0].errors.clone();
        document.operations[0].errors.clear();
        let output: Value = serde_json::from_str(&crate::emit(&document).unwrap()).unwrap();
        assert!(output["paths"]["/auth/login"]["post"]["responses"]["4XX"].is_object());
    }

    #[test]
    fn rejects_dot_segment_path_literals() {
        for lit in [".", ".."] {
            let mut document = fixture();
            document.operations[0].path = vec![PathSegment::Literal { lit: lit.into() }];
            let error = crate::emit(&document).unwrap_err().0;
            assert!(error.contains("dot segment"), "{error}");
        }
    }

    #[test]
    fn keeps_literals_that_only_look_like_dot_segments() {
        let mut document = fixture();
        document.operations[0].path = vec![PathSegment::Literal { lit: "..a".into() }];
        let output: Value = serde_json::from_str(&crate::emit(&document).unwrap()).unwrap();
        assert!(
            output["paths"].as_object().unwrap().contains_key("/..a"),
            "{:#}",
            output["paths"]
        );
    }
}
